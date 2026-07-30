# Canonical Multi-Phase Plan Format

This is the shared contract between `multiphase-plan` (authors it), `kestral-pickup`
(reads it to claim a lane), and `kestral-handoff` (reconciles + repushes it). All three
skills — run by Claude Code **or** Codex, in any worktree — read and write this exact
shape. Keep the inline markers stable; they are parsed, not just displayed.

## Markers (must stay machine-parseable)

- Each phase heading ends with two bracket tags: `` `[lane: X]` `` and
  `` `[status: todo|in-progress|blocked|done]` ``.
- `lane:` groups phases that one worktree owns end-to-end. Same letter = same lane =
  sequential within that worktree. Different letters = independent = parallel worktrees.
- `status:` is the source of truth for progress in the document; it mirrors the linked
  Kestral task's status. `kestral-handoff` updates both together.

## Document template

> The template below shows four phases across two lanes purely to illustrate every marker.
> It is **not** a target shape — most real plans are a single lane (`[lane: A]` on every
> phase) with no "Parallel execution guide" section. Do not add lanes to match this example.

```markdown
# <Effort name> — Multi-Phase Plan

**Status:** planning | in progress | integrating | done
**Last updated:** <YYYY-MM-DD> by <host: Claude Code | Codex> (<worktree/branch>)
**Kestral project:** [<project name>](project-url)
**Plan doc:** [<title>](doc-url)  ·  workContextId: `<id>`

## Loop config

> Optional — present only when the effort will run under `kestral-loop`. Omit otherwise.

- **Integration branch:** `<type>/<effort-slug>`
- **Verify:** `<command>`   (effort-wide gate; per-phase **Verify:** lines override it)
- **PR:** _none yet_   (filled by kestral-loop)
- **Concurrency:** 3

## Goal
<What we're building and why. 2–4 sentences.>

## Approach & key decisions
<Architecture, key trade-offs, constraints, non-goals. Bullet list.>

## Phases

> Phase headings and branch names below are **descriptive** — they name what the phase
> *does*, not its position. The `Phase N` label lives only in the marker/tags, never in the
> branch, worktree, commit, or PR name. (See "Naming" in the Rules section.)

### Phase 1 — Add OAuth token refresh endpoint `[lane: A]` `[status: todo]`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** none
- **Parallelizable with:** Phase 2 (independent)
- **Suggested branch:** `feat/oauth-token-refresh`
- **Touches:** <files / modules / areas — for conflict awareness>
- **Done when:** <acceptance criteria, testable>
- **Verify:** <optional — runnable acceptance check, exit 0 = pass>
- **Notes:** <optional>

### Phase 2 — Migrate invoice retry to a state machine `[lane: B]` `[status: todo]`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** none
- **Parallelizable with:** Phase 1 (independent)
- **Suggested branch:** `refactor/invoice-retry-state`
- **Touches:** <...>
- **Done when:** <...>
- **Verify:** <optional — runnable acceptance check, exit 0 = pass>

### Phase 3 — Use refreshed tokens in the API client `[lane: A]` `[status: todo]`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** Phase 1
- **Parallelizable with:** Phase 2 (independent)
- **Suggested branch:** `feat/api-client-token-refresh`
- **Touches:** <...>
- **Done when:** <...>

### Phase 4 — Integrate auth + billing paths and run e2e `[lane: integration]` `[status: todo]`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** Phase 3, Phase 2
- **Parallelizable with:** none — integration point
- **Suggested branch:** `chore/integrate-auth-billing`
- **Touches:** <...>
- **Done when:** <both lanes merged and green>

## Parallel execution guide
- **Lane A** (worktree 1): Phase 1 → Phase 3. Start immediately.
- **Lane B** (worktree 2): Phase 2. Independent — start immediately.
- **Integration point:** Phase 4 merges Lanes A + B in a single worktree after both land.
- **Conflict watch:** Phases 2 and 3 both touch `src/api/*` — the second to merge rebases first.
- **Merge order:** Lane B → Lane A → integration (or note the real order).

## Progress log
- <YYYY-MM-DD> — <what moved, by which lane/worktree>. (appended by kestral-handoff)
```

## Local copy header

When a skill saves the plan to `.kestral/plan.md` in a worktree, the top of the file must
carry the Kestral coordinates so the other skills can repush without re-searching:

```markdown
<!-- kestral-plan
project: <project name>
projectId: <id>
workContextId: <plan document id>
docUrl: <doc-url>
-->
```

Keep this HTML comment as the first lines of `.kestral/plan.md`. `kestral-handoff` reads
`workContextId` from here to call `update_document`; `kestral-pickup` writes it after
downloading.

## Rules

- **No fake parallelism — default to one lane.** Decompose the work into the phases it
  naturally has first; only then look for independence. Split phases into separate lanes
  only when they pass all three independence checks (no dependency, low file contention, no
  coordination tax — see `multiphase-plan` step 2 → Independence test). Never reshape,
  split, or reorder phases, and never add stubs/mocks/frozen interfaces/flags, just to
  enable parallel work. A fully sequential, single-lane plan is the expected outcome for
  most efforts; more lanes is not better. When in doubt, keep it sequential.
- **Every phase has a testable *Done when*.** A phase without acceptance criteria can't be
  claimed or handed off cleanly. A runnable **Verify:** line (a command, exit 0 = pass) is
  what lets `kestral-loop` enforce the *Done when* mechanically.
- **Loop mode ships as ONE PR.** When a Loop config section exists, per-phase Suggested
  branches are short-lived lane branches cut from the integration branch tip and merged
  back; no per-phase PRs; `[status: done]` = merged into the integration branch.
- **Plan doc and Kestral tasks cross-reference.** Each phase's **Task:** line links its
  task; each task carries `tags: ["phase:<N>", "lane:<X>"]`.
- **Status lives in two places, kept in lockstep:** the `[status: …]` marker in the doc and
  the linked task's Kestral status. `kestral-handoff` updates both.
- **Naming: describe the work, never the index.** Phase titles, `Suggested branch`,
  worktrees, commits, and PRs must say *what the change does* — never `phase-4`,
  `product-split-phase-4`, `<effort>-phase-N`, or a bare number. The phase↔branch link is
  already carried by the `phase:<N>` task tag and the plan doc, so it does not belong in the
  branch name.
  - **Phase title** — an imperative outcome: "Add OAuth token refresh endpoint", not
    "Phase 1" or "Auth stuff". This title seeds the task title and usually the PR title.
  - **Suggested branch** — `<type>/<imperative-outcome-slug>`, kebab-case, where `<type>` is
    a conventional prefix (`feat`, `fix`, `refactor`, `chore`, `docs`, `test`). E.g.
    `feat/oauth-token-refresh`, `refactor/invoice-retry-state`. Keep it under ~50 chars, no
    phase number, no effort prefix.
  - Bad → good: `product-split-phase-4` → `chore/integrate-auth-billing`;
    `phase-2-oauth` → `feat/oauth-token-refresh`.
