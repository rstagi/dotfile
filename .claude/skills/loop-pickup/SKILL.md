---
name: loop-pickup
description: >-
  Resume a multi-phase effort in a fresh chat/worktree: recover the plan from `.loop/plan.md`,
  then the Loop daemon — pick (or accept) a parallel lane, conflict-check it against sibling
  worktrees, claim the phase (status + branch), and load just enough context to start. Use at
  the start of a new Conductor worktree or clean chat, or when asked to "pick up the plan",
  "resume the effort", "start a lane", or "continue where the handoff left off".
argument-hint: "<project name> [lane/phase] (optional) | --auto <project> phase:<N>"
---

# Loop Pickup

The receiving end of `multiphase-plan` and `loop-handoff`. A fresh chat has no memory of
prior sessions — this skill rebuilds working context from the durable plan store (the Loop
daemon + `.loop/plan.md`), so a **Claude Code or Codex** agent in **any** worktree can grab
an unblocked lane and start. Designed for Conductor's parallel worktrees: each worktree runs
this and claims a different lane.

## Prerequisites

The Loop daemon (`http://localhost:7717`) is the only backend — `loop-plan.sh` talks to it,
no MCP required. This skill works purely locally against the daemon + `.loop/plan.md`.

## Workflow

### 1. Find the effort and download the plan

Resolve the plan in this order — stop at the first that yields it:

1. **Local `.loop/plan.md`:** if it exists in the worktree, read its canonical header
   (`<!-- loop-plan`) for the `planId` and `daemon` (ignore legacy `kestral*` keys).
2. **The Loop daemon:** `loop-plan.sh get --plan-id <id>` fetches the plan markdown (use
   `loop-plan.sh list` — a compact `runId · status · effort` table — to find the `planId`
   by effort/status when the argument or header didn't give it).

**Freshness.** The daemon copy is authoritative and current (a sibling worktree's
`loop-plan.sh push` already updated it), so `loop-plan.sh get` gives you the latest.
**Ask** which effort to resume if ambiguous.

Save the resolved plan to `.loop/plan.md` with the canonical header (see
`../multiphase-plan/references/plan-format.md`).

Parse `## Repositories` and every work phase's `Repository:`. Resolve the chosen phase's
`owner/repo` through `loop-repo.sh get`; if missing in interactive mode, ask for its local
checkout and persist it with `loop-repo.sh map`. Validate origin/default branch/collisions
with `loop-repo.sh check`. Plans never carry checkout paths. Legacy plans use `primary`
and the launching checkout.

### 2. Pick a lane

Parse the plan's phases, lanes, `[status: …]`, and optional `[kind: pr-review]` marker.
Review phases are orchestrator-owned barriers: never offer, claim, or execute one from a
worktree. The `[status: …]` markers in `.loop/plan.md` are authoritative.
A phase that an `[archived]` stub lists is an existing phase with status `done`. A `Depends on`
edge to it is satisfied. Never offer or claim it. Its block is in `.loop/milestones/m<N>.md`.
Read that file only when the chosen phase needs its history. Then:

- **If the user named a lane/phase** → use it.
- **Otherwise** show the lanes with their state and recommend the next **unblocked** one
  (all its `Depends on` phases are `done`, and it isn't already `in-progress` in another
  worktree):

  > **[project] — [effort]** · N phases in M lanes.
  > - **Lane A:** Phase 1 ✅ done · Phase 3 ⏳ ready ← recommended
  > - **Lane B:** Phase 2 🔒 claimed (in progress, another worktree)
  > - **Integration:** Phase 4 — blocked on Lane A + B
  >
  > Which lane should this worktree take? (recommend Lane A / Phase 3)

Do not start integration phases until their dependency phases are `done`.

**Milestones.** In a plan with milestones, read `state.milestone` from `.loop/state.json`; when
that file is absent (a fresh worktree), ask the daemon — `GET /api/loops/<id>/plan` → `.milestone`
— and, if the daemon is down, read the plan: when plan `**Status:**` is `awaiting merge`, the current milestone is the one whose gate is the last `done` gate
(the loop is parked there; nothing later is claimable); otherwise take the lowest milestone whose gate phase is not `done`.
If neither state nor daemon is available and the status is unclear, refuse every later-milestone claim.
Then never offer or claim a phase of a later milestone than `state.milestone` — those stay locked
until a human merges the gate and resumes the loop, even if their `Depends on` looks satisfied —
and never claim a phase of an earlier (shipped) milestone either (inspecting it read-only is fine).

### 3. Conflict-check before claiming

So two worktrees don't collide:

- The check is the plan's `[status: …]` marker (already `in-progress`?) plus whether the
  phase's Suggested branch already exists in a sibling worktree.
- Read the plan's **Conflict watch**: if a sibling lane in flight touches the same files,
  surface it now so the user sequences merges deliberately.

### 4. Claim the phase

Confirm with the user, then claim the phase: flip its marker to `[status: in-progress]` in
`.loop/plan.md`, run `loop-plan.sh push`, and create/switch to the phase's Suggested branch.
The plan's `[status: …]` markers + the **Conflict watch** (step 3) *are* the conflict check
— last-writer-wins is acceptable for single-user local mode.

Use the phase's **Suggested branch** in its declared repository. Create/switch that branch
from the repository integration tip, never from another repository's lane worktree. If a
lane moved repositories, remove the previous worktree and create a target-repository
worktree. The Conductor worktree, commits, and repository PR inherit this name.

**If the suggested branch is missing or non-descriptive** (a bare `phase-N`, an
`<effort>-phase-N`, or just a number), do not use it — derive a descriptive
`<type>/<imperative-outcome-slug>` from the phase title instead (e.g. phase "Add OAuth token
refresh endpoint" → `feat/oauth-token-refresh`) and offer to fix the plan's `Suggested
branch` line on the next `loop-handoff`.

> Claiming **Phase 3 - <phase title>** on branch `feat/oauth-token-refresh`, set to in-progress. Proceed?

### 5. Load focused context and start

Pull only what this phase needs — its phase body (*Depends on* / *Touches* /
*Done when*) and the relevant plan sections. Summarize in a few lines, then explore the code
paths the phase *Touches* and outline the first concrete steps. Hand control back to the
user to begin implementation (in auto mode, return the digest to the caller instead — see
**Auto mode**).

> Ready on **Phase 3 — <title>**. Done when: <criteria>. Touches: <areas>. First steps: …
> When this lane advances or you switch worktrees, run **`/loop-handoff`**
> (Codex: **`$loop-handoff`**) to repush the plan.

## Auto mode

`loop-pickup --auto <project> phase:<N>` — executed by a `loop-execute` **runner** at
the start of its headless session, inside the lane worktree the orchestrator created (see
`../loop-execute/references/loop-protocol.md`); never in a human-facing session unasked.
Every "ask the user" gate becomes deterministic:

- **Step 1 still re-resolves** the plan from the daemon — freshness matters even more with
  parallel lanes.
- **No lane menu (step 2):** the given phase *is* the choice.
- **Milestone refusal:** resolve `state.milestone` as in step 2. If the given phase belongs to a
  later milestone, emit `REFUSED: phase belongs to a later milestone`; if to an earlier one, emit
  `REFUSED: phase belongs to a shipped milestone`. Either way write blocked status and stop.
- **Review refusal:** if the given phase is `[kind: pr-review]`, emit
  `REFUSED: review phases are orchestrator-owned`, write blocked status, and stop.
- **No claim confirmation (step 4), no git prompt** — you are already in the lane worktree
  on the pre-created branch. Claim nothing (the orchestrator is the single writer): verify
  checkout origin + `git branch --show-current` match the phase repository/branch (mismatch
  → `REFUSED`), load context, and continue into the work. Do NOT flip `[status: …]` or run
  `loop-plan.sh push` — that's the orchestrator's job via `loop-handoff`.
- **Refuse instead of asking:** emit a one-line structured refusal `REFUSED: <reason>`,
  then (per the loop protocol) write the runner's `status.json` with outcome `blocked` and
  stop. Conditions: branch mismatch, plan missing, unmet *Depends on*. Never ask, never
  work around.
- **Step 5 flows into implementation:** build the focused context digest (*Done when* /
  *Touches* / first steps) and continue straight into the work — there is no user to hand
  control back to.

## Cross-agent notes

Everything needed to resume comes from the **Loop daemon + `.loop/plan.md`**, re-fetched
fresh — so it doesn't matter which agent or worktree wrote the last handoff. Use the Loop
daemon (`loop-plan.sh`) + git + local files; reference both `/name` and `$name` invocation.
