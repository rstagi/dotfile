---
name: loop-pickup
description: >-
  Resume a multi-phase effort in a fresh chat/worktree: recover the plan from `.loop/plan.md`,
  then the Loop daemon, then Kestral only when the plan is linked — pick (or accept) a parallel
  lane, conflict-check it against sibling worktrees, claim the phase (task + branch), and load
  just enough context to start. Use at the start of a new Conductor worktree or clean chat, or
  when asked to "pick up the plan", "resume the effort", "start a lane", or "continue where the
  handoff left off".
argument-hint: "<project name> [lane/phase] (optional) | --auto <project> phase:<N>"
---

# Loop Pickup

The receiving end of `multiphase-plan` and `loop-handoff`. A fresh chat has no memory of
prior sessions — this skill rebuilds working context from the durable plan store (the Loop
daemon + `.loop/plan.md`, and Kestral when the plan is linked), so a **Claude Code or Codex**
agent in **any** worktree can grab an unblocked lane and start. Designed for Conductor's
parallel worktrees: each worktree runs this and claims a different lane.

## Prerequisites

The **Kestral** MCP server is needed **only when the plan is linked** (or the user names a
Kestral project) — otherwise this skill works purely locally against the Loop daemon +
`.loop/plan.md`, no MCP required. In linked mode, auth is at the MCP layer (OAuth) — proceed
directly; on auth failure (401, `unauthorized`, `Not authenticated`) tell the user to
reconnect via their host UI (Claude Code: `/mcp`; Codex: Settings → MCP Servers →
Authenticate) and stop. Keep Kestral IDs internal; show `slug - title` and readable
names + URLs.

## Workflow

### 1. Find the effort and download the plan

Resolve the plan in this order — stop at the first that yields it:

1. **Local `.loop/plan.md`:** if it exists in the worktree, read its canonical header
   (`<!-- loop-plan`) for the `planId`, `daemon`, and — LINKED only — the
   `kestralProject/kestralProjectId/kestralWorkContextId/kestralDocUrl` coordinates
   (LINKED ⇔ `kestralWorkContextId` present).
2. **The Loop daemon:** `loop-plan.sh get --plan-id <id>` fetches the plan markdown (use
   `loop-plan.sh list` — a compact `runId · status · effort` table — to find the `planId`
   by effort/status when the argument or header didn't give it).
3. **Kestral — LINKED only** (or when the user named a Kestral project): resolve via
   `entity_lookup` (URL/slug) or `search_operations`→`search_projects` / `find_documents`,
   then `execute_operation("get_document_content", { workContextId })` (follow
   `isTruncated`/`nextOffset` if paged).

**Freshness.** *Unlinked* → the daemon copy is authoritative and current (a sibling
worktree's `loop-plan.sh push` already updated it), so `loop-plan.sh get` gives you the
latest. *Linked* → ALSO re-fetch the Kestral doc; newest **Last updated** wins, then
re-sync both sides (daemon + Kestral). **Ask** which effort to resume if ambiguous.

Save the resolved plan to `.loop/plan.md` with the canonical header (see
`../multiphase-plan/references/plan-format.md`). Optionally pull the Project Brain for
background (`entity_lookup({ type: "project_brain", id: projectId })`, or invoke
`kestral-context`) — keep it to a short digest, don't dump it.

Parse `## Repositories` and every phase's `Repository:`. Resolve the chosen phase's
`owner/repo` through `loop-repo.sh get`; if missing in interactive mode, ask for its local
checkout and persist it with `loop-repo.sh map`. Validate origin/default branch/collisions
with `loop-repo.sh check`. Plans never carry checkout paths. Legacy plans use `primary`
and the launching checkout.

### 2. Pick a lane

Parse the plan's phases, lanes, and `[status: …]` markers. **When linked**, reconcile them
with live Kestral task status (`entity_lookup` on the phase tasks, or `list_tasks_by_status`);
**unlinked**, the `[status: …]` markers in `.loop/plan.md` are authoritative. Then:

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

### 3. Conflict-check before claiming

So two worktrees don't collide:

- **Linked** → apply `kestral-sync`'s **Conflict Check**: is the chosen phase's task already
  assigned / `in-progress` (claimed by another worktree)? If so, warn with assignee + status
  and offer a different lane. **Unlinked** → the check is the plan's `[status: …]` marker
  (already `in-progress`?) plus whether the phase's Suggested branch already exists in a
  sibling worktree.
- **Either mode** → read the plan's **Conflict watch**: if a sibling lane in flight touches
  the same files, surface it now so the user sequences merges deliberately.

### 4. Claim the phase

Confirm with the user, then claim the phase — the mechanism depends on the mode:

- **Linked** → prefer invoking **`kestral-sync`** (it owns branch derivation,
  `claim_task_and_branch`, status discovery, and the 409 already-linked conflict signal).
- **Unlinked** → there is no Kestral claim: flip the phase's marker to
  `[status: in-progress]` in `.loop/plan.md`, run `loop-plan.sh push`, and create/switch to
  the phase's Suggested branch. The plan's `[status: …]` markers + the **Conflict watch**
  (step 3) *are* the conflict check — last-writer-wins is acceptable for single-user local
  mode.

Use the phase's **Suggested branch** in its declared repository. Create/switch that branch
from the repository integration tip, never from another repository's lane worktree. If a
lane moved repositories, remove the previous worktree and create a target-repository
worktree. The Conductor worktree, commits, and repository PR inherit this name.

**If the suggested branch is missing or non-descriptive** (a bare `phase-N`, an
`<effort>-phase-N`, or just a number), do not use it — derive a descriptive
`<type>/<imperative-outcome-slug>` from the phase title instead (e.g. phase "Add OAuth token
refresh endpoint" → `feat/oauth-token-refresh`) and offer to fix the plan's `Suggested
branch` line on the next `loop-handoff`.

> Claiming **[slug] - <phase title>** on branch `feat/oauth-token-refresh`, set to In Progress. Proceed?

### 5. Load focused context and start

Pull only what this phase needs — its task description (*Depends on* / *Touches* /
*Done when*), the relevant plan sections, and any linked docs/brain bullets. Summarize in a
few lines, then explore the code paths the phase *Touches* and outline the first concrete
steps. Hand control back to the user to begin implementation (in auto mode, return the
digest to the caller instead — see **Auto mode**).

> Ready on **Phase 3 — <title>**. Done when: <criteria>. Touches: <areas>. First steps: …
> When this lane advances or you switch worktrees, run **`/loop-handoff`**
> (Codex: **`$loop-handoff`**) to repush the plan.

## Auto mode

`loop-pickup --auto <project> phase:<N>` — executed by a `loop-execute` **runner** at
the start of its headless session, inside the lane worktree the orchestrator created (see
`../loop-execute/references/loop-protocol.md`); never in a human-facing session unasked.
Every "ask the user" gate becomes deterministic:

- **Step 1 still re-resolves** the plan (daemon; also Kestral when linked) — freshness
  matters even more with parallel lanes.
- **No lane menu (step 2):** the given phase *is* the choice.
- **No claim confirmation (step 4), no git prompt** — you are already in the lane worktree
  on the pre-created branch. What happens next depends on the mode:
  - **Linked** → verify both the checkout origin matches phase `Repository:` and
    `git branch --show-current` matches **Suggested branch** (mismatch → `REFUSED`), then claim via `claim_task_and_branch` with that
    branch. Re-claiming a branch already linked to your own task is a no-op; never create
    or switch branches, never steal a claim.
  - **Unlinked** → claim nothing (the orchestrator is the single writer): verify checkout
    origin + `git branch --show-current` match the phase repository/branch (mismatch →
    `REFUSED`), load context, and continue into the work. Do NOT flip `[status: …]` or run
    `loop-plan.sh push` — that's the orchestrator's job via `loop-handoff`.
- **Refuse instead of asking:** emit a one-line structured refusal `REFUSED: <reason>`,
  then (per the loop protocol) write the runner's `status.json` with outcome `blocked` and
  stop. Conditions differ by mode — *linked:* task already claimed by someone else / 409,
  plan missing, phase blocked (unmet *Depends on*), ambiguous project; *unlinked:* branch
  mismatch, plan missing, unmet *Depends on*. Never ask, never work around.
- **Step 5 flows into implementation:** build the focused context digest (*Done when* /
  *Touches* / first steps) and continue straight into the work — there is no user to hand
  control back to.

## Cross-agent notes

Everything needed to resume comes from the **Loop daemon + `.loop/plan.md`** (and the
**Kestral plan doc + phase tasks when linked**), re-fetched fresh — so it doesn't matter which
agent or worktree wrote the last handoff. Use the Loop daemon (`loop-plan.sh`) + git + local
files, and Kestral MCP only when the plan is linked; reference both `/name` and `$name`
invocation. In linked mode, delegate claim/status/conflict mechanics to `kestral-sync` rather
than reimplementing them.
