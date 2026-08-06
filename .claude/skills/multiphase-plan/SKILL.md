---
name: multiphase-plan
description: >-
  Turn a goal into a multi-phase implementation plan, marking any genuinely independent
  lanes that could run in separate Conductor worktrees (and staying fully sequential when
  the work doesn't parallelize), then register it on the local Loop daemon as the base
  shared source of truth — with an optional, opt-in publish to Kestral. Use when asked to
  "plan this out", "make a multi-phase plan", "break this into parallel phases", "put a
  plan on Kestral" (Kestral publish is opt-in), or before spinning up parallel worktrees
  for a larger effort.
argument-hint: "<goal, or existing Kestral project name> (optional)"
---

# Multi-Phase Plan

Produce a correct phased plan, and register it on the local **Loop daemon**
(http://localhost:7717) as the base shared source of truth so any coding agent — **Claude
Code or Codex**, in any worktree — can pick it up and continue; optionally link it to
**Kestral** as well. *Where the work genuinely allows it*, mark phases into **lanes** that
can be worked concurrently in separate worktrees (Conductor). Pairs with `loop-pickup`
(start a lane in a fresh worktree) and `loop-handoff` (reconcile progress and repush the
plan).

**Correctness before parallelism.** The job is a plan that reflects the real shape of the
work. Parallelism is an *observation* about that plan (some phases happen to be
independent), never a *goal* you bend the plan toward. A fully sequential, single-lane plan
is a normal, expected, good outcome — most plans are mostly sequential. More lanes is not
better.

## Prerequisites

`jq` and the **Loop daemon** (http://localhost:7717). The daemon is auto-ensured for you by
`loop-plan.sh register` / `loop-emit.sh`, so no manual setup is needed for a local-only plan.

The **Kestral** MCP server is needed **only when the user opts to link this plan to
Kestral**. When linking, auth is handled by the MCP connection (OAuth) — proceed directly.
If a call returns auth failure (401, `unauthorized`, or `Not authenticated`), tell the user
to reconnect through their host UI (Claude Code: `/mcp` → reconnect; Codex: Settings → MCP
Servers → Authenticate) and stop. Keep Kestral IDs internal in user-facing output; show
`slug - title` for tasks and readable names + URLs for projects/documents.

## Workflow

### 1. Understand the goal

The argument (or the conversation) is the goal. Build enough understanding to phase the
work well — **do not plan blind**:

- Explore the codebase for the areas the work touches (dispatch an `Explore`/general
  agent or read the relevant files). Note the modules, seams, and shared files that
  phases will contend over — this drives the parallelization analysis.
- *If the effort may already be Kestral-tracked* (the user named a project, or you'll likely
  link it), pull existing Kestral context: `search_operations`→`search_projects`/`find_documents`
  for a related project, brief, or prior plan (or invoke `kestral-context`). Skip this for a
  purely local plan.
- If the goal is thin or ambiguous, ask **2–4 sharp questions** before planning (scope,
  constraints, definition of done, hard sequencing). Offer `grilling` if the user wants
  the plan stress-tested first.

### 2. Draft the phased plan

Author the plan in the **canonical plan format** (see
`references/plan-format.md` — read it and follow it exactly; `loop-handoff` and
`loop-pickup` parse the same markers). The plan must decide:

- **Phases** — each a coherent, independently-reviewable unit of work with a clear
  *Done when* (acceptance criteria). Decompose the work into the phases it *naturally* has
  — the phases you'd write with no parallelism in mind at all. Order them; record
  `Depends on` edges. **Title each phase as an imperative outcome** ("Add OAuth token
  refresh endpoint"), never "Phase 1" or a vague label — the title seeds the Kestral task
  title and usually the PR title.
- **Slices, not layers** — for full-stack work, each phase is a **vertical slice**: the
  backend *and* the frontend of one user-visible capability ship in the same phase, never
  "all the backend phases, then all the frontend phases". (Within a slice the backend is
  still built first, the frontend taps in after — but as one phase, one branch, one PR.)
  If a slice is genuinely too big for one phase, split it into adjacent backend and
  frontend phases — but they stay in the **same lane**, share the **same `Suggested
  branch`**, and land in the **same PR**; mark each with "same PR as Phase N" in its
  Notes. A layer-only phase is legitimate only when the work itself is single-layer
  (pure API change, pure UI polish) or a shared foundation several slices build on.
- **Lanes** — **default: one lane, fully sequential.** A lane is a chain of phases one
  worktree owns start-to-finish. Only after the phases exist, look for independence and
  split into separate lanes — and only when they pass the **Independence test** below.
- **Parallelization guide** — *only if there are 2+ lanes.* For each lane, which worktree
  runs it and what it can start on immediately; the **integration points** (where lanes
  merge and who owns the merge); and a **conflict watch** listing files/areas that multiple
  lanes touch. If there's one lane, omit this section.
- **Verify** per phase — where one exists, a runnable **Verify:** command (exit 0 = pass)
  that checks the phase's *Done when*. Ask the user for the effort-wide verify command, and
  whether this effort will run under `loop-execute` — if yes, emit the **Loop config**
  section per `references/plan-format.md`.
- **Suggested branch** per phase — a **descriptive** `<type>/<imperative-outcome-slug>`
  (e.g. `feat/oauth-token-refresh`, `refactor/invoice-retry-state`), so `loop-pickup` and
  `kestral-sync` claim it deterministically and the worktree/commits/PR all read clearly.
  **Name the work, not the index:** never `phase-4`, `<effort>-phase-N`, or a bare number —
  the phase↔branch link is carried by the `phase:<N>` task tag and this plan doc, so it must
  not clutter the branch name. See "Naming" in `references/plan-format.md`. Exception:
  phases that split one vertical slice (see **Slices, not layers**) share a single branch —
  name it after the capability, not the layer.

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

### 3. Register on the daemon (the base backend)

**This always runs** — local-only is the default, every plan lives on the daemon. Write the
plan markdown to `.loop/plan.md` in the current worktree, prefixed with the canonical
`<!-- loop-plan -->` header (see `references/plan-format.md`). Mint the `planId`
(`loop-<effort-slug>-<date>-<HHMMSS|rand>`), set `daemon: http://localhost:7717`, and
**omit the four `kestral*` keys for now** — this plan starts unlinked:

    <!-- loop-plan
    planId: loop-<effort-slug>-<date>-<HHMMSS|rand>
    daemon: http://localhost:7717
    -->

Then register it:

    loop-plan.sh register            # defaults to --dir .loop, --plan-file .loop/plan.md

`register` POSTs `/register` to the daemon (auto-starting it if it isn't up) and prints the
`planId`; the plan lands with daemon status `planned`. This lets the current worktree start
immediately and lets `loop-handoff` repush from here. Mention the user can add `.loop/` to
`.gitignore` if they don't want the plan committed.

### 4. Confirm — and choose whether to link Kestral

The plan is now live locally. One gate before any Kestral write:

> Plan registered on the local daemon · planId **`<id>`**.
> Link this plan to **Kestral** too (creates a project + plan doc + phase tasks)?
> (default: **no** — local-only)

Link to Kestral **only** if the user says yes, or already asked for Kestral in the original
request (e.g. "put a plan on Kestral"). Otherwise stop here and go to **Report** — the
daemon plan + `.loop/plan.md` are the source of truth.

### 5. When linked to Kestral

Run the full Kestral flow (unchanged). Kestral stays authoritative for the plan doc + tasks;
the daemon is **always written too** (dual-write, Kestral second) so `.loop/plan.md` ends up
carrying the linked coordinates.

**Resolve the target project.** `search_operations`→`search_projects({ query })` with the
goal keywords.

- **Existing project fits** → use its `projectId` + `url` (confirm with the user which one).
- **No fit / new effort** → create one: `execute_operation("create_project", { title, description })`.
  For a from-scratch effort with lots of source context to import, prefer invoking
  `kestral-setup` (it builds the project + Project Brain from connected tools); come back
  here to attach the plan.

**Publish to Kestral:**

1. **Plan document:** `execute_operation("create_document", { title: "<Effort> — Multi-Phase Plan", content: "<canonical markdown>", projectId })`.
   Capture the returned document id (`workContextId`) and `url`.
2. **One parent effort task + phase subtasks** (1 plan = 1 task with N subtasks — never N
   top-level tasks):
   1. Parent: `execute_operation("create_task", { projectId, title, description, priority,
      tags: ["multiphase-plan"] })` — `title` = the imperative effort outcome, `description`
      = goal summary + lane overview + link to the plan doc. Capture its `id`/`slug`/`url`.
   2. One subtask per phase: `create_task` with `parentTaskId` = the parent's id, `title` =
      phase title, `description` = the phase's *Depends on* / *Parallelizable with* /
      *Touches* / *Done when* / *Suggested branch* + plan-doc link, `priority`, and
      `tags: ["phase:<N>", "lane:<X>"]` (Kestral has no native phase/dependency fields —
      tags + the plan doc carry that structure). Create subtasks individually: the
      `subtasks` param on `create_task` accepts bare title strings only, and
      `create_tasks_batch` can't attach to a parent. If the dedupe gate blocks a legitimate
      phase (near-title of existing work), retry with `duplicatePolicy: "create_anyway"` +
      `overrideReason`. Use `list_statuses` if you need a non-default starting status.
      Capture each returned `slug` + `url`.
3. **Back-link tasks into the plan:** `execute_operation("update_document", { workContextId, content })`
   — add an `**Effort task:**` line to the doc header linking the parent, and fill each
   phase's **Task:** line with `[<slug> - <title>](task-url)` now that the subtasks exist.
   The plan doc and the tasks must cross-reference.
4. **Trigger brain:** `execute_operation("trigger_brain_build", { projectId })` so the
   Project Brain absorbs the new plan + tasks.

**Write the linked coordinates back to the daemon.** Add the four `kestral*` keys —
`kestralProject`, `kestralProjectId`, `kestralWorkContextId`, `kestralDocUrl` — to the
`.loop/plan.md` header, then run `loop-plan.sh push` (re-reads the plan file and overwrites
the daemon copy) so the daemon carries the linked coordinates. The plan is now **linked**
(⇔ `kestralWorkContextId` present).

### 6. Report

State the shape honestly — do not imply parallelism that isn't there. Fill every `<…>` in
the templates below with the real captured value (planId, project name, doc / project URLs,
phase & lane counts) — never print a literal placeholder. State the **backend** honestly:
`local daemon` for an unlinked plan, `local daemon + Kestral` when linked (drop the
**Kestral:** line entirely for a local-only plan). The pickup/execute argument is the
**planId** (or effort name) for a local-only plan; for a linked plan the **Kestral project
name** works too.

**Single-lane (sequential) plan — the common case:**

> **Plan registered** · N phases, sequential.
> State backend: **local daemon** · planId `<id>` · Observatory http://localhost:7717
> **Kestral** *(linked only):* [<Effort> — Multi-Phase Plan](doc-url) on [<project name>](project-url)
>
> This work is sequential — each phase builds on the last, so it runs in one worktree in
> order. Start with **Phase 1**: run **`/loop-pickup <planId>`** (Codex: **`$loop-pickup`**;
> a linked plan's **<project name>** works too). When a phase advances, run
> **`/loop-handoff`** to repush the plan.
>
> **Run the loop:** **`/loop-execute <planId>`** (Codex: **`$loop-execute`**),
> hands-off once registered.

**Multi-lane plan — only when phases passed the Independence test:**

> **Plan registered** · N phases in M lanes.
> State backend: **local daemon** · planId `<id>` · Observatory http://localhost:7717
> **Kestral** *(linked only):* [<Effort> — Multi-Phase Plan](doc-url) on [<project name>](project-url)
>
> **Parallel lanes:**
> - **Lane A** (worktree 1): Phase 1 → Phase 3 — start now.
> - **Lane B** (worktree 2): Phase 2 — independent, start now.
> - **Integration:** Phase 4 merges A + B (single worktree, after both).
> - **Conflict watch:** Phases 2 & 3 both touch `src/api/*` — rebase before merge.
>
> In each new Conductor worktree, run **`/loop-pickup <planId>`** (Codex: **`$loop-pickup`**;
> a linked plan's **<project name>** works too) and claim its lane. When a lane advances, run
> **`/loop-handoff`** to reconcile and repush the plan.
>
> **Run the loop:** **`/loop-execute <planId>`** (Codex: **`$loop-execute`**) runs all lanes
> hands-off.

If you split into lanes, be ready to justify each split against the Independence test if the
user asks. If it was a close call, prefer reporting it as sequential and note the *optional*
parallelism rather than presenting it as the required structure.

## Cross-agent notes

The durable contract is the **daemon plan (planId) + `.loop/plan.md`** — and the **Kestral
plan document + phase tasks when linked** — not this chat, so a Codex agent in another
worktree resumes from the same state a Claude Code agent wrote. Keep the plan mechanics to
`loop-plan.sh` + Kestral MCP operations + git + local files only; do not rely on any
host-specific feature. Reference both invocation styles (`/name` for Claude Code,
`$name` for Codex).
