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

## Goal
<What we're building and why. 2–4 sentences.>

## Approach & key decisions
<Architecture, key trade-offs, constraints, non-goals. Bullet list.>

## Phases

### Phase 1 — <title> `[lane: A]` `[status: todo]`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** none
- **Parallelizable with:** Phase 2 (independent)
- **Suggested branch:** `phase-1-<slug>`
- **Touches:** <files / modules / areas — for conflict awareness>
- **Done when:** <acceptance criteria, testable>
- **Notes:** <optional>

### Phase 2 — <title> `[lane: B]` `[status: todo]`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** none
- **Parallelizable with:** Phase 1 (independent)
- **Suggested branch:** `phase-2-<slug>`
- **Touches:** <...>
- **Done when:** <...>

### Phase 3 — <title> `[lane: A]` `[status: todo]`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** Phase 1
- **Parallelizable with:** Phase 2 (independent)
- **Suggested branch:** `phase-3-<slug>`
- **Touches:** <...>
- **Done when:** <...>

### Phase 4 — <title> `[lane: integration]` `[status: todo]`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** Phase 3, Phase 2
- **Parallelizable with:** none — integration point
- **Suggested branch:** `phase-4-<slug>`
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
  claimed or handed off cleanly.
- **Plan doc and Kestral tasks cross-reference.** Each phase's **Task:** line links its
  task; each task carries `tags: ["phase:<N>", "lane:<X>"]`.
- **Status lives in two places, kept in lockstep:** the `[status: …]` marker in the doc and
  the linked task's Kestral status. `kestral-handoff` updates both.
