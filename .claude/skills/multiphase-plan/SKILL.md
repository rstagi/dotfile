---
name: multiphase-plan
description: >-
  Turn a goal into a multi-phase implementation plan with explicit parallelization
  hints (independent lanes you can run in separate Conductor worktrees), then publish
  it to Kestral as a plan document + phase tasks on a new or existing project. Use when
  asked to "plan this out", "make a multi-phase plan", "break this into parallel phases",
  "put a plan on Kestral", or before spinning up parallel worktrees for a larger effort.
argument-hint: "<goal, or existing Kestral project name> (optional)"
---

# Multi-Phase Plan → Kestral

Produce a phased plan whose phases are grouped into **lanes** that can be worked
concurrently in separate worktrees (Conductor), and store it on Kestral as the shared
source of truth so any coding agent — **Claude Code or Codex** — can pick up a lane and
continue. Pairs with `kestral-pickup` (start a lane in a fresh worktree) and
`kestral-handoff` (reconcile progress and repush the plan).

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
  *Done when* (acceptance criteria). Order them; record `Depends on` edges.
- **Lanes** — group phases into lanes that can run **in parallel in separate worktrees**.
  A lane is a chain of phases one agent/worktree owns start-to-finish. Two phases belong
  in different lanes only when they are genuinely independent — verify against the shared
  files you found in step 1.
- **Parallelization guide** — for each lane, which worktree runs it and what it can start
  on immediately; the **integration points** (where lanes merge and who owns the merge);
  and a **conflict watch** listing files/areas that multiple lanes touch (so owners rebase
  before merging).
- **Suggested branch** per phase (e.g. `phase-2-oauth-tokens`) so `kestral-pickup` and
  `kestral-sync` can claim a branch deterministically.

Be honest about parallelism: if the work is inherently sequential, say so and produce a
single-lane plan rather than inventing fake independence. Silent over-parallelization
causes merge pain.

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

### 6. Report + hand off to parallel work

Summarize so the user can fan out into Conductor worktrees:

> **Plan published:** [<Effort> — Multi-Phase Plan](doc-url) on [project](project-url) · N phases in M lanes.
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

## Cross-agent notes

The durable contract is the **Kestral plan document + phase tasks**, not this chat — so a
Codex agent in another worktree resumes from the same state a Claude Code agent wrote. Keep
the plan mechanics to Kestral MCP operations + git + local files only; do not rely on any
host-specific feature. Reference both invocation styles (`/name` for Claude Code,
`$name` for Codex).
