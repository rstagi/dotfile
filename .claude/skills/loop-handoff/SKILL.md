---
name: loop-handoff
description: >-
  Reconcile the current worktree's progress against the shared multi-phase plan and repush it
  — always to the Loop daemon (and `.loop/plan.md`), plus Kestral when the plan is linked —
  then leave a clean pointer so a fresh chat (Claude Code or Codex, any worktree) can
  continue. Use when wrapping up a work session on a looped effort, before switching
  worktrees, or when asked to "hand this off", "sync the plan back to Kestral", or "repush
  the plan".
argument-hint: "<what the next session should focus on> (optional)"
---

# Loop Handoff

The generic `/handoff` writes a throwaway doc for the next agent. This one makes the **shared
plan the durable handoff**: it reconciles what moved against the plan, repushes it to the Loop
daemon + `.loop/plan.md` (and Kestral when the plan is linked), and points the next clean chat
at it. Because the state lives in the plan store (not this conversation), the next session can
be a different agent in a different worktree. Counterpart to `multiphase-plan` (creates the
plan) and `loop-pickup` (resumes it).

## Prerequisites

The Loop daemon (`http://localhost:7717`) is the base backend — `loop-plan.sh` talks to it,
no MCP required. The **Kestral** MCP server is needed **only when the plan is linked** (its
`.loop/plan.md` header carries the `kestral*` coordinates). In linked mode auth is at the MCP
layer (OAuth) — proceed directly; on auth failure (401, `unauthorized`, `Not authenticated`)
tell the user to reconnect via their host UI (Claude Code: `/mcp`; Codex: Settings → MCP
Servers → Authenticate) and stop. Keep Kestral IDs internal; show `slug - title` and readable
names + URLs.

## Workflow

### 1. Locate the plan (the thing being handed off)

Resolve the plan and its coordinates, in this order — stop at the first hit:

1. **Local copy (fast path):** read `.loop/plan.md` in the worktree. Its header comment
   (`<!-- loop-plan`) carries the `planId` + `daemon` URL, and — when linked — the four
   `kestral*` keys (see the plan-format reference). LINKED ⇔ `kestralWorkContextId` present.
2. **Daemon by planId:** `loop-plan.sh get --plan-id <id>` fetches the authoritative copy
   from the daemon (use the `planId` from step 1, or `loop-plan.sh list` to find it). If the
   worktree has no local copy at all, this is how you recover the plan when unlinked.
3. **Linked only — argument / user pointer:** a project name, plan-doc URL, or task slug the
   user gave → `entity_lookup` (URL/slug) or `search_operations`→`find_documents` /
   `search_projects`.
4. **Linked only — branch → task → project:**
   `execute_operation("find_task_by_branch", { branchName })` → the task's `projectId` →
   `find_documents({ query: "<effort> plan", projectId })` for the plan doc.
5. **Ask** the user which effort/plan this session was working on.

If there is genuinely no plan (ad-hoc work, not a `multiphase-plan` effort), fall back to a
lighter handoff: when linked, sync the task via `kestral-sync`; either way, if the user wants
a narrative handoff, invoke the generic `handoff` skill. Don't fabricate a plan. (In auto
mode there is no fallback — see **Auto mode**.)

### 2. Reconcile current state against the plan

Gather what actually moved this session and map it onto the plan's phases:

- `git branch --show-current`, `git log --oneline -20`, `git diff --stat <base>...HEAD`.
- Read the current plan markdown (local copy, or `get_document_content({ workContextId })`).
- For each phase, decide the new `[status: …]`: `done` (acceptance criteria met — verify
  against the diff, don't assume), `in-progress`, `blocked` (note why), or unchanged.
- Note any **plan drift**: scope that changed, phases added/split/dropped, new conflicts
  discovered. The plan should reflect reality, not the original guess.

### 3. Update the plan markdown

Edit the canonical plan (follow
`../multiphase-plan/references/plan-format.md`):

- Flip the `[status: …]` markers for phases that moved.
- Update the top **Status** line (planning → in progress → integrating → done) and the
  **Last updated** line with today's date + host + worktree/branch.
- Append a **Progress log** entry: one line, plain-language outcomes, which lane/worktree.
- Apply any drift (edit phase bodies, add integration/conflict notes) so a fresh reader
  trusts it.

Redact secrets (API keys, tokens, PII) — never write them into the plan or comments.

### 4. Repush the plan (the "handoff")

Show the user the before/after of the changed plan sections and confirm (skipped entirely
in auto mode), then:

**Always:**

1. **Local + daemon:** apply the step-3 edits to `.loop/plan.md` (flip the `[status: …]`
   markers, the **Status** / **Last updated** lines, append the **Progress log** entry),
   then run `loop-plan.sh push` — it overwrites the daemon's copy of the plan and reprints
   the `planId`. The daemon copy is what a cross-worktree pickup and the Loop Observatory
   read, so it must reflect reality.
2. **Progress stream (unlinked):** also append a timestamped entry to `.loop/progress.md`
   (2–4 lines, plain-language outcomes — the phase/lane/engine that moved, exactly what a
   Kestral progress comment would have said), then run
   `loop-plan.sh note --plan-id <id> [--phase N] --body "<same summary>"` so it lands on the
   loop timeline cross-worktree. `.loop/progress.md` is the local substitute for Kestral
   progress comments — skip it when linked (the Kestral comments below cover it).

**When linked, additionally** — Kestral is authoritative for the plan doc + task status;
write it too, *second*, so local + daemon + Kestral stay in lockstep:

3. **Plan document:** `execute_operation("update_document", { workContextId, content })`
   with the full updated markdown. (If the plan only ever existed locally — never
   published — `create_document` it into the project instead, then write the coordinates
   back into `.loop/plan.md`.)
4. **Task statuses:** for each phase whose status changed, update its linked task. Prefer
   invoking **`kestral-sync`** (it owns status discovery via `list_statuses`, the PR-merge
   gate, progress-comment style, PR linking, and conflict dedup). Manual path if not
   delegating: `execute_operation("update_task_status", { taskId, statusKey })` +
   `execute_operation("post_progress_comment", { taskId, content })` (2–4 lines,
   conversational outcomes, no file paths). Link the PR with
   `execute_operation("complete_task_with_review", { taskId, prUrl, comment })` when one
   was opened.
5. Optionally `execute_operation("trigger_brain_build", { projectId })` if the plan
   changed materially, so the Project Brain absorbs the update.

### 5. Point the next session forward

End with a crisp resume instruction — the next chat is fresh (no memory of this one):

> **Handed off.** Plan repushed to the Loop daemon (planId `<id>`)<when linked: · [<title>](doc-url) on [project](project-url)>.
> - Done this session: <phases/outcomes>.
> - **Next up:** <the next unblocked phase(s) / lane(s), or the focus from the argument>.
> - Blocked: <blockers, or "none">.
>
> To continue in a fresh chat: **`/loop-pickup <planId | project>`** (Codex: **`$loop-pickup`**),
> then claim <lane/phase>. Everything it needs is in `.loop/plan.md` + the daemon (and the
> Kestral plan doc + phase tasks when linked).

If the user passed an argument, treat it as what the next session should focus on and steer
the "Next up" line accordingly.

## Auto mode

`loop-handoff --auto phase:<N> status:<done|blocked|in-progress> [lane:<X>
engine:<codex|claude>]` — used only inside the `loop-execute` flow (contract:
`../loop-execute/references/loop-protocol.md`), in two contexts distinguished by the
status value:

- **`status:in-progress` — the runner**, at the end of its headless session:
  implementation complete, awaiting merge. **Events/notes only, never the plan write:**
  update the local `.loop/plan.md` copy and run `loop-plan.sh note` (unlinked) or
  `post_progress_comment` (linked). **Never `loop-plan.sh push` and never `update_document`**
  — the orchestrator is the sole plan-writer to the daemon + Kestral.
- **`status:done|blocked` — the orchestrator**, post-merge (or on HIL pause): the full
  handoff — `loop-plan.sh push` + append `.loop/progress.md` (unlinked), plus the Kestral
  `update_document` / `update_task_status` / `post_progress_comment` ops when linked. It has
  already run the verify gate and the merge — apply the verdict, don't re-decide.

Differences from the interactive flow (both contexts):

- **No confirmation gate:** skip step 4's before/after confirmation — repush without
  asking.
- **No fallback, no questions:** never fall back to the generic `handoff` skill. If the
  plan can't be located (step 1 exhausted), return `REFUSED: <reason>` to the caller
  instead of asking.
- **Loop-mode status semantics:** `[status: done]` = merged into the integration branch
  (the plan's Loop config names it). When linked, the human PR-merge gate applies to the
  single effort PR, so do NOT call `complete_task_with_review` per phase — use
  `update_task_status` + `post_progress_comment` only. PR linking happens once at effort
  completion (the caller does it via `link_pr_to_task`).
- **Progress log:** entries (in `.loop/plan.md`, `.loop/progress.md`, the daemon note, and
  the Kestral comment when linked) carry the lane + engine that did the work (the caller
  passes them), e.g. "via loop lane A (codex)".
- **Redaction is on you:** in auto mode nobody reviews the content before repush — step 3's
  redact-secrets rule is non-negotiable.

## Cross-agent notes

The handoff artifact is the **daemon plan + `.loop/plan.md` + `.loop/progress.md`** (plus the
Kestral plan doc + task state when linked), all host-agnostic — a Codex worktree resumes
exactly what a Claude Code worktree left. Use the Loop daemon (`loop-plan.sh`) + git + local
files, and Kestral MCP only when linked; reference both `/name` and `$name` invocation. Do
not duplicate `kestral-sync`'s logic — delegate task/PR/status work to it and keep this skill
focused on the plan reconciliation + repush.
