---
name: loop-handoff
description: >-
  Reconcile the current worktree's progress against the shared multi-phase plan and repush it
  to the Loop daemon (and `.loop/plan.md`), then leave a clean pointer so a fresh chat (Claude
  Code or Codex, any worktree) can continue. Use when wrapping up a work session on a looped
  effort, before switching worktrees, or when asked to "hand this off", "sync the plan back
  to the daemon", or "repush the plan".
argument-hint: "<what the next session should focus on> (optional)"
---

# Loop Handoff

The generic `/handoff` writes a throwaway doc for the next agent. This one makes the **shared
plan the durable handoff**: it reconciles what moved against the plan, repushes it to the Loop
daemon + `.loop/plan.md`, and points the next clean chat at it. Because the state lives in the
plan store (not this conversation), the next session can be a different agent in a different
worktree. Counterpart to `multiphase-plan` (creates the plan) and `loop-pickup` (resumes it).

## Prerequisites

The Loop daemon (`http://localhost:7717`) is the only backend — `loop-plan.sh` talks to it,
no MCP required.

## Workflow

### 1. Locate the plan (the thing being handed off)

Resolve the plan and its coordinates, in this order — stop at the first hit:

1. **Local copy (fast path):** read `.loop/plan.md` in the worktree. Its header comment
   (`<!-- loop-plan`) carries the `planId` + `daemon` URL (see the plan-format reference).
2. **Daemon by planId:** `loop-plan.sh get --plan-id <id>` fetches the authoritative copy
   from the daemon (use the `planId` from step 1, or `loop-plan.sh list` to find it). If the
   worktree has no local copy at all, this is how you recover the plan.
3. **Ask** the user which effort/plan this session was working on.

If there is genuinely no plan (ad-hoc work, not a `multiphase-plan` effort), fall back to a
lighter handoff: if the user wants a narrative handoff, invoke the generic `handoff` skill.
Don't fabricate a plan. (In auto mode there is no fallback — see **Auto mode**.)

### 2. Reconcile current state against the plan

Gather what actually moved this session and map it onto the plan's phases:

- `git branch --show-current`, `git log --oneline -20`, `git diff --stat <base>...HEAD`.
- Read the current plan markdown (local copy, or `loop-plan.sh get`).
- For each phase, decide the new `[status: …]`: `done` (acceptance criteria met — verify
  against the diff, don't assume), `in-progress`, `blocked` (note why), or unchanged.
- Note any **plan drift**: scope that changed, phases added/split/dropped, new conflicts
  discovered. The plan should reflect reality, not the original guess.
- Treat phase numbers as stable identities and document order as the sequence. Allocate the
  next unused number for new phases; never renumber or reuse an existing number after
  execution starts.
- In a plan with milestones, never mark a phase of a later milestone than `state.milestone` `done` — a later milestone only starts after the gate merge and `/loop-execute resume`. Read `state.milestone` from `.loop/state.json`; when absent, `GET /api/loops/<id>/plan` → `.milestone`, else (daemon down) the lowest milestone whose gate phase is not `done`.
- Likewise never change the status of a phase of an earlier (shipped) milestone — its gate PRs are merged; reading it is fine.
- Reconcile a phase only against its declared `Repository:` checkout. Work in another
  repository cannot complete it. Preserve repository blocks and never write checkout paths
  into the plan.

### 3. Update the plan markdown

Edit the canonical plan (follow
`../multiphase-plan/references/plan-format.md`):

- Flip the `[status: …]` markers for phases that moved.
- Update the top **Status** line (planning → in progress → integrating → done) and the
  **Last updated** line with today's date + host + worktree/branch.
- Append a **Progress log** entry: one line, plain-language outcomes, which lane/worktree.
- Apply any drift (edit phase bodies, add integration/conflict notes) so a fresh reader
  trusts it.
- Reconcile the terminal review invariant. If the last `[kind: pr-review]` phase has not run,
  insert new work immediately before it and extend its `Depends on` closure. If it is `done`,
  leave it in place, append new work after it, and append a new terminal review phase. Never
  reopen a completed review. Push one coherent edit so readers never observe a work-terminal
  executable plan.

Redact secrets (API keys, tokens, PII) — never write them into the plan or notes.

### 4. Repush the plan (the "handoff")

Show the user the before/after of the changed plan sections and confirm (skipped entirely
in auto mode), then:

1. **Local + daemon:** apply the step-3 edits to `.loop/plan.md` (flip the `[status: …]`
   markers, the **Status** / **Last updated** lines, append the **Progress log** entry),
   then run `loop-plan.sh push` — it overwrites the daemon's copy of the plan and reprints
   the `planId`. The daemon copy is what a cross-worktree pickup and the Loop Observatory
   read, so it must reflect reality.
2. **Progress stream:** also append a timestamped entry to `.loop/progress.md` (2–4 lines,
   plain-language outcomes — the phase/lane/engine that moved), then run
   `loop-plan.sh note --plan-id <id> [--phase N] --body "<same summary>"` so it lands on the
   loop timeline cross-worktree.

### 5. Point the next session forward

End with a crisp resume instruction — the next chat is fresh (no memory of this one):

> **Handed off.** Plan repushed to the Loop daemon (planId `<id>`).
> - Done this session: <phases/outcomes>.
> - **Next up:** <the next unblocked phase(s) / lane(s), or the focus from the argument>.
> - Blocked: <blockers, or "none">.
>
> To continue in a fresh chat: **`/loop-pickup <planId | project>`** (Codex: **`$loop-pickup`**),
> then claim <lane/phase>. Everything it needs is in `.loop/plan.md` + the daemon.

If the user passed an argument, treat it as what the next session should focus on and steer
the "Next up" line accordingly.

## Auto mode

`loop-handoff --auto phase:<N> status:<done|blocked|in-progress> [lane:<X>
engine:<codex|claude|grok>]` — used only inside the `loop-execute` flow (contract:
`../loop-execute/references/loop-protocol.md`), in two contexts distinguished by the
status value:

- **`status:in-progress` — the runner**, at the end of its headless session:
  implementation complete, awaiting merge. **Events/notes only, never the plan write:**
  update the local `.loop/plan.md` copy and run `loop-plan.sh note`. **Never
  `loop-plan.sh push`** — the orchestrator is the sole plan-writer to the daemon.
- **`status:done|blocked` — the orchestrator**, post-merge (or on HIL pause): the full
  handoff — `loop-plan.sh push` + append `.loop/progress.md`. It has already run the verify
  gate and the merge — apply the verdict, don't re-decide.

Differences from the interactive flow (both contexts):

- **No confirmation gate:** skip step 4's before/after confirmation — repush without
  asking.
- **No fallback, no questions:** never fall back to the generic `handoff` skill. If the
  plan can't be located (step 1 exhausted), return `REFUSED: <reason>` to the caller
  instead of asking.
- **Loop-mode status semantics:** `[status: done]` = merged into the phase repository's
  integration branch for a work phase; for `[kind: pr-review]`, `done` means every scoped
  repository review returned a non-blocking verdict. The human PR-merge gate applies per
  repository.
- **Progress log:** entries (in `.loop/plan.md`, `.loop/progress.md`, and the daemon note)
  carry the lane + engine that did the work (the caller passes them), e.g. "via loop lane A
  (codex)".
- **Redaction is on you:** in auto mode nobody reviews the content before repush — step 3's
  redact-secrets rule is non-negotiable.

## Cross-agent notes

The handoff artifact is the **daemon plan + `.loop/plan.md` + `.loop/progress.md`**, all
host-agnostic — a Codex worktree resumes exactly what a Claude Code worktree left. Use the
Loop daemon (`loop-plan.sh`) + git + local files; reference both `/name` and `$name`
invocation. Keep this skill focused on the plan reconciliation + repush.
