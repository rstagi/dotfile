---
name: kestral-handoff
description: >-
  Kestral-backed handoff: reconcile the current worktree's progress against the shared
  Kestral plan, repush the updated plan document + task statuses to Kestral, and leave a
  clean pointer so a fresh chat (Claude Code or Codex, any worktree) can continue. Use when
  wrapping up a work session on a Kestral-tracked effort, before switching worktrees, or
  when asked to "hand this off", "sync the plan back to Kestral", or "repush the plan".
argument-hint: "<what the next session should focus on> (optional)"
---

# Kestral Handoff

The generic `/handoff` writes a throwaway doc for the next agent. This one makes **Kestral
the durable handoff**: it reconciles what moved against the shared plan document, repushes
the plan + task state to Kestral, and points the next clean chat at it. Because the state
lives in Kestral (not this conversation), the next session can be a different agent in a
different worktree. Counterpart to `multiphase-plan` (creates the plan) and `kestral-pickup`
(resumes it).

## Prerequisites

The **Kestral** MCP server must be in this session. Auth is at the MCP layer (OAuth) —
proceed directly; on auth failure (401, `unauthorized`, `Not authenticated`) tell the user
to reconnect via their host UI (Claude Code: `/mcp`; Codex: Settings → MCP Servers →
Authenticate) and stop. Keep Kestral IDs internal; show `slug - title` and readable
names + URLs.

## Workflow

### 1. Locate the plan (the thing being handed off)

Resolve the Kestral plan document and its project, in this order — stop at the first hit:

1. **Local copy:** read `.kestral/plan.md` in the worktree. Its header comment carries
   `projectId`, `workContextId`, and `docUrl` (see the plan-format reference). This is the
   fast path.
2. **Argument / user pointer:** a project name, plan-doc URL, or task slug the user gave →
   `entity_lookup` (URL/slug) or `search_operations`→`find_documents` / `search_projects`.
3. **Branch → task → project:** `execute_operation("find_task_by_branch", { branchName })`
   → the task's `projectId` → `find_documents({ query: "<effort> plan", projectId })` for
   the plan doc.
4. **Ask** the user which project/plan this session was working on.

If there is genuinely no plan document (ad-hoc work, not a `multiphase-plan` effort), fall
back to a lighter handoff: sync the task via `kestral-sync` and, if the user wants a
narrative handoff, invoke the generic `handoff` skill. Don't fabricate a plan doc. (In auto
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

### 4. Repush to Kestral (the "handoff")

Show the user the before/after of the changed plan sections and confirm (skipped entirely
in auto mode), then:

1. **Plan document:** `execute_operation("update_document", { workContextId, content })`
   with the full updated markdown. (If the plan only ever existed locally — never
   published — `create_document` it into the project instead, then write the coordinates
   back into `.kestral/plan.md`.)
2. **Task statuses:** for each phase whose status changed, update its linked task. Prefer
   invoking **`kestral-sync`** (it owns status discovery via `list_statuses`, the PR-merge
   gate, progress-comment style, PR linking, and conflict dedup). Manual path if not
   delegating: `execute_operation("update_task_status", { taskId, statusKey })` +
   `execute_operation("post_progress_comment", { taskId, content })` (2–4 lines,
   conversational outcomes, no file paths). Link the PR with
   `execute_operation("complete_task_with_review", { taskId, prUrl, comment })` when one
   was opened.
3. **Keep local + remote in lockstep:** write the same updated markdown back to
   `.kestral/plan.md`.
4. Optionally `execute_operation("trigger_brain_build", { projectId })` if the plan
   changed materially, so the Project Brain absorbs the update.

### 5. Point the next session forward

End with a crisp resume instruction — the next chat is fresh (no memory of this one):

> **Handed off to Kestral.** Plan updated: [<title>](doc-url) on [project](project-url).
> - Done this session: <phases/outcomes>.
> - **Next up:** <the next unblocked phase(s) / lane(s), or the focus from the argument>.
> - Blocked: <blockers, or "none">.
>
> To continue in a fresh chat: **`/kestral-pickup <project>`** (Codex: **`$kestral-pickup`**),
> then claim <lane/phase>. Everything it needs is in the plan doc and phase tasks.

If the user passed an argument, treat it as what the next session should focus on and steer
the "Next up" line accordingly.

## Auto mode

`kestral-handoff --auto phase:<N> status:<done|blocked|in-progress> [lane:<X>
engine:<codex|claude>]` — used only inside the `kestral-loop` flow (contract:
`../kestral-loop/references/loop-protocol.md`), in two contexts distinguished by the
status value:

- **`status:in-progress` — the runner**, at the end of its headless session:
  implementation complete, awaiting merge. **Task-scoped ops only**: update the local
  `.kestral/plan.md` copy, `post_progress_comment`, keep/confirm the task's in-progress
  status. **Never `update_document`** — the orchestrator is the sole plan-doc writer.
- **`status:done|blocked` — the orchestrator**, post-merge (or on HIL pause): the full
  handoff including the plan-doc repush. It has already run the verify gate and the
  merge — apply the verdict, don't re-decide.

Differences from the interactive flow (both contexts):

- **No confirmation gate:** skip step 4's before/after confirmation — repush without
  asking.
- **No fallback, no questions:** never fall back to the generic `handoff` skill. If the
  plan doc can't be located (step 1 exhausted), return `REFUSED: <reason>` to the caller
  instead of asking.
- **Loop-mode status semantics:** `[status: done]` = merged into the integration branch
  (the plan's Loop config names it). The human PR-merge gate applies to the single effort
  PR, so do NOT call `complete_task_with_review` per phase — use `update_task_status` +
  `post_progress_comment` only. PR linking happens once at effort completion (the caller
  does it via `link_pr_to_task`).
- **Progress log:** entries carry the lane + engine that did the work (the caller passes
  them), e.g. "via loop lane A (codex)".
- **Redaction is on you:** in auto mode nobody reviews the content before repush — step 3's
  redact-secrets rule is non-negotiable.

## Cross-agent notes

The handoff artifact is the **Kestral plan doc + task state + `.kestral/plan.md`**, all
host-agnostic — a Codex worktree resumes exactly what a Claude Code worktree left. Use only
Kestral MCP + git + local files; reference both `/name` and `$name` invocation. Do not
duplicate `kestral-sync`'s logic — delegate task/PR/status work to it and keep this skill
focused on the plan reconciliation + repush.
