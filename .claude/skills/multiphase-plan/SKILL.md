---
name: multiphase-plan
description: >-
  Turn a goal into a multi-phase implementation plan, marking any genuinely independent
  lanes that could run in separate Conductor worktrees (and staying fully sequential when
  the work doesn't parallelize), then publish it to Kestral as a plan document + phase
  tasks on a new or existing project. Use when asked to "plan this out", "make a
  multi-phase plan", "break this into parallel phases", "put a plan on Kestral", or before
  spinning up parallel worktrees for a larger effort.
argument-hint: "<goal, or existing Kestral project name> (optional)"
---

# Multi-Phase Plan → Kestral

Produce a correct phased plan, and store it on Kestral as the shared source of truth so any
coding agent — **Claude Code or Codex** — can pick it up and continue. *Where the work
genuinely allows it*, mark phases into **lanes** that can be worked concurrently in separate
worktrees (Conductor). Pairs with `kestral-pickup` (start a lane in a fresh worktree) and
`kestral-handoff` (reconcile progress and repush the plan).

**Correctness before parallelism.** The job is a plan that reflects the real shape of the
work. Parallelism is an *observation* about that plan (some phases happen to be
independent), never a *goal* you bend the plan toward. A fully sequential, single-lane plan
is a normal, expected, good outcome — most plans are mostly sequential. More lanes is not
better.

## Prerequisites

The **Kestral** MCP server must be in this session. Auth is handled by the MCP connection
(OAuth) — proceed directly. If a call returns auth failure (401, `unauthorized`, or
`Not authenticated`), tell the user to reconnect through their host UI (Claude Code:
`/mcp` → reconnect; Codex: Settings → MCP Servers → Authenticate) and stop. Keep Kestral
IDs internal in user-facing output; show `slug - title` for tasks and readable names +
URLs for projects/documents.

## Workflow

### 1. Understand the goal

The argument (or the conversation) is the goal. Build enough understanding to phase the
work well — **do not plan blind**:

- Explore the codebase for the areas the work touches (dispatch an `Explore`/general
  agent or read the relevant files). Note the modules, seams, and shared files that
  phases will contend over — this drives the parallelization analysis.
- Pull any existing Kestral context: `search_operations`→`search_projects`/`find_documents`
  for a related project, brief, or prior plan (or invoke `kestral-context`).
- If the goal is thin or ambiguous, ask **2–4 sharp questions** before planning (scope,
  constraints, definition of done, hard sequencing). Offer `grilling` if the user wants
  the plan stress-tested first.

### 2. Draft the phased plan

Author the plan in the **canonical plan format** (see
`references/plan-format.md` — read it and follow it exactly; `kestral-handoff` and
`kestral-pickup` parse the same markers). The plan must decide:

- **Phases** — each a coherent, independently-reviewable unit of work with a clear
  *Done when* (acceptance criteria). Decompose the work into the phases it *naturally* has
  — the phases you'd write with no parallelism in mind at all. Order them; record
  `Depends on` edges. **Title each phase as an imperative outcome** ("Add OAuth token
  refresh endpoint"), never "Phase 1" or a vague label — the title seeds the Kestral task
  title and usually the PR title.
- **Lanes** — **default: one lane, fully sequential.** A lane is a chain of phases one
  worktree owns start-to-finish. Only after the phases exist, look for independence and
  split into separate lanes — and only when they pass the **Independence test** below.
- **Parallelization guide** — *only if there are 2+ lanes.* For each lane, which worktree
  runs it and what it can start on immediately; the **integration points** (where lanes
  merge and who owns the merge); and a **conflict watch** listing files/areas that multiple
  lanes touch. If there's one lane, omit this section.
- **Suggested branch** per phase — a **descriptive** `<type>/<imperative-outcome-slug>`
  (e.g. `feat/oauth-token-refresh`, `refactor/invoice-retry-state`), so `kestral-pickup` and
  `kestral-sync` claim it deterministically and the worktree/commits/PR all read clearly.
  **Name the work, not the index:** never `phase-4`, `<effort>-phase-N`, or a bare number —
  the phase↔branch link is carried by the `phase:<N>` task tag and this plan doc, so it must
  not clutter the branch name. See "Naming" in `references/plan-format.md`.

#### Independence test (the safety gate of this skill)

Two phases may go in **different lanes** only when **all three** hold. If any fails, keep
them in the **same** lane:

1. **No dependency** — neither needs the other's output; both can start from current `main`.
2. **Low file contention** — they don't edit the same files/modules. Incidental overlap
   (a shared type, a config line) is tolerable only if a tiny, one-time interface is
   settled up front and listed in the conflict watch — not if they'd fight over the same
   core files.
3. **No coordination tax** — parallelizing them does **not** require stubs, mocks, frozen
   interfaces, feature flags, duplicated scaffolding, or any seam you wouldn't otherwise
   write. If parallelism only works by adding artificial structure or making "something
   weird" happen, it isn't worth it — sequence them instead.

**Never reshape, split, or reorder the natural phases to manufacture parallelism.** When in
doubt, keep phases sequential — that is always the safe answer. If the work is inherently
sequential, produce a single-lane plan and say so plainly; that is a success, not a
shortfall. Silent over-parallelization causes merge pain and is the exact failure this gate
exists to prevent.

### 3. Resolve the target project

`search_operations`→`search_projects({ query })` with the goal keywords.

- **Existing project fits** → use its `projectId` + `url` (confirm with the user which one).
- **No fit / new effort** → create one: `execute_operation("create_project", { title, description })`.
  For a from-scratch effort with lots of source context to import, prefer invoking
  `kestral-setup` (it builds the project + Project Brain from connected tools); come back
  here to attach the plan.

Confirm the target with the user before any write:

> Plan target: **[project]** (existing / new). I'll add a plan document + N phase tasks.
> Proceed? (yes / adjust)

### 4. Publish to Kestral

After approval:

1. **Plan document:** `execute_operation("create_document", { title: "<Effort> — Multi-Phase Plan", content: "<canonical markdown>", projectId })`.
   Capture the returned document id (`workContextId`) and `url`.
2. **Phase tasks:** `execute_operation("create_tasks_batch", { projectId, tasks })` — one
   task per phase. For each task set `title` = phase title, `description` = the phase's
   *Depends on* / *Parallelizable with* / *Touches* / *Done when* / *Suggested branch*,
   `priority`, and `tags: ["phase:<N>", "lane:<X>"]` (Kestral has no native phase/dependency
   fields — tags + the plan doc carry that structure). Use `list_statuses` if you need a
   non-default starting status. Capture each returned `slug` + `url`.
   - Use `create_task` with a `subtasks` array instead when a phase has meaningful
     sub-steps worth tracking individually.
3. **Back-link tasks into the plan:** `execute_operation("update_document", { workContextId, content })`
   — fill each phase's **Task:** line with `[<slug> - <title>](task-url)` now that the
   tasks exist. The plan doc and the tasks must cross-reference.
4. **Trigger brain:** `execute_operation("trigger_brain_build", { projectId })` so the
   Project Brain absorbs the new plan + tasks.

### 5. Save a local working copy

Write the same markdown to `.kestral/plan.md` in the current worktree, with a small header
block so the other skills can find the Kestral source (see `references/plan-format.md` →
"Local copy header"). This lets the current worktree start immediately and lets
`kestral-handoff` repush from here. Mention the user can add `.kestral/` to `.gitignore` if
they don't want the plan committed.

### 6. Report

State the shape honestly — do not imply parallelism that isn't there.

**Single-lane (sequential) plan — the common case:**

> **Plan published:** [<Effort> — Multi-Phase Plan](doc-url) on [project](project-url) ·
> N phases, sequential.
>
> This work is sequential — each phase builds on the last, so it runs in one worktree in
> order. Start with **Phase 1**: run **`/kestral-pickup <project>`** (Codex:
> **`$kestral-pickup`**). When a phase advances, run **`/kestral-handoff`** to repush the plan.

**Multi-lane plan — only when phases passed the Independence test:**

> **Plan published:** [<Effort> — Multi-Phase Plan](doc-url) on [project](project-url) ·
> N phases in M lanes.
>
> **Parallel lanes:**
> - **Lane A** (worktree 1): Phase 1 → Phase 3 — start now.
> - **Lane B** (worktree 2): Phase 2 — independent, start now.
> - **Integration:** Phase 4 merges A + B (single worktree, after both).
> - **Conflict watch:** Phases 2 & 3 both touch `src/api/*` — rebase before merge.
>
> In each new Conductor worktree, run **`/kestral-pickup <project>`** (Codex:
> **`$kestral-pickup`**) and claim its lane. When a lane advances, run **`/kestral-handoff`**
> to reconcile and repush the plan.

If you split into lanes, be ready to justify each split against the Independence test if the
user asks. If it was a close call, prefer reporting it as sequential and note the *optional*
parallelism rather than presenting it as the required structure.

## Cross-agent notes

The durable contract is the **Kestral plan document + phase tasks**, not this chat — so a
Codex agent in another worktree resumes from the same state a Claude Code agent wrote. Keep
the plan mechanics to Kestral MCP operations + git + local files only; do not rely on any
host-specific feature. Reference both invocation styles (`/name` for Claude Code,
`$name` for Codex).
