# Canonical Multi-Phase Plan Format

This is the shared contract between `multiphase-plan` (authors it), `loop-pickup`
(reads it to claim a lane), and `loop-handoff` (reconciles + repushes it). All three
skills — run by Claude Code **or** Codex, in any worktree — read and write this exact
shape. Keep the inline markers stable; they are parsed, not just displayed.

The plan lives on disk in a flattened `.loop/` dir (this `plan.md` sits beside `state.json`).
Every plan is backed by the central Loop Observatory daemon (`http://localhost:7717`) — the
base backend; Kestral is an **opt-in linked** backend (plans are local-only by default).

Repository checkouts are deliberately absent from the plan. Plans carry GitHub
`owner/repo` slugs only; each machine resolves them through `~/.loop/repos.json` via
`loop-repo.sh map|get|list|unmap`. Every phase names exactly one repository. A lane may move
between repositories.

## Markers (must stay machine-parseable)

- Each work-phase heading ends with `` `[lane: X]` `` and
  `` `[status: todo|in-progress|blocked|done]` ``. A review phase adds
  `` `[kind: pr-review]` ``; phases without `kind:` are work phases.
- A review phase may add `` `[review: shallow|medium|max]` `` (default `medium`) and
  `` `[rounds: N]` `` (default per tier: shallow 1, medium 3, max 3; `1 ≤ N ≤ 5`). shallow = one
  Opus reviewer, Sonnet fixes; medium = Sol + Opus adversaries, Opus fixes; max = Astra + Fable
  adversaries, Opus fixes; every tier ends with one Opus verdict. `loop-review.sh stages` turns
  the pair into the run list.
- `lane:` groups phases that one worktree owns end-to-end. Same letter = same lane =
  sequential within that worktree. Different letters = independent = parallel worktrees.
- `status:` is the source of truth for progress in the document; `loop-handoff` mirrors it to
  the daemon always, and to the linked Kestral task when the plan is linked.
- `Repository:` is mandatory on every work phase when `## Repositories` exists and must match
  one declared slug. A PR-review phase covers every repository and omits `Repository:`.
  Repository `Verify:` is mandatory; a work phase `Verify:` overrides it.
- `Estimate:` is required on every work phase of a new Loop plan. It is a human-readable range,
  confidence, and basis; the parser ignores it. Its upper bound should normally be at most
  25 minutes to leave room before the runner's 30-minute checkpoint. State exceptions
  openly rather than claiming a bound the evidence cannot support.
- Legacy plans without `## Repositories` normalize to one synthetic `primary` repository;
  their scalar Loop config `Verify` / `PR` fields remain executable.

## Document template

> The template below shows four phases across two lanes purely to illustrate every marker.
> It is **not** a target shape — most real plans are a single lane (`[lane: A]` on every
> phase) with no "Parallel execution guide" section. Do not add lanes to match this example.

> **Linked-only header lines:** the `**Kestral project:**`, `**Plan doc:**`, and
> `**Effort task:**` lines below appear in the rendered plan ONLY when it is linked to
> Kestral; omit all three in local-only plans.

```markdown
# <Effort name> — Multi-Phase Plan

**Status:** planning | in progress | integrating | done
**Last updated:** <YYYY-MM-DD> by <host: Claude Code | Codex> (<worktree/branch>)
**Kestral project:** [<project name>](project-url) <!-- linked only -->
**Plan doc:** [<title>](doc-url)  ·  workContextId: `<id>` <!-- linked only -->
**Effort task:** [<slug> - <title>](task-url) — phases are its subtasks <!-- linked only -->

## Loop config

> Optional — present only when the effort will run under `loop-execute`. Omit otherwise.

- **Integration branch:** `<type>/<effort-slug>`
- **Concurrency:** 3

## Repositories

### `owner/api`
- **Verify:** `<command>`
- **Integration branch:** `<optional per-repository override>`
- **PR:** _none yet_

### `owner/web`
- **Verify:** `<command>`
- **PR:** _none yet_

## Goal
<What we're building and why. 2–4 sentences.>

## Approach & key decisions
<Architecture, key trade-offs, constraints, non-goals. Bullet list.>

## Phases

> Phase headings and branch names below are **descriptive** — they name what the phase
> *does*, not its position. The `Phase N` label lives only in the marker/tags, never in the
> branch, worktree, commit, or PR name. (See "Naming" in the Rules section.)

### Phase 1 — Add OAuth token refresh endpoint `[lane: A]` `[status: todo]`
- **Repository:** `owner/api`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** none
- **Parallelizable with:** Phase 2 (independent)
- **Suggested branch:** `feat/oauth-token-refresh`
- **Touches:** <files / modules / areas — for conflict awareness>
- **Done when:** <acceptance criteria, testable>
- **Estimate:** <min–max min · high|medium|low confidence · basis>
- **Verify:** <optional — runnable acceptance check, exit 0 = pass>
- **Notes:** <optional>

### Phase 2 — Migrate invoice retry to a state machine `[lane: B]` `[status: todo]`
- **Repository:** `owner/api`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** none
- **Parallelizable with:** Phase 1 (independent)
- **Suggested branch:** `refactor/invoice-retry-state`
- **Touches:** <...>
- **Done when:** <...>
- **Estimate:** <min–max min · confidence · basis>
- **Verify:** <optional — runnable acceptance check, exit 0 = pass>

### Phase 3 — Use refreshed tokens in the API client `[lane: A]` `[status: todo]`
- **Repository:** `owner/web`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** Phase 1
- **Parallelizable with:** Phase 2 (independent)
- **Suggested branch:** `feat/api-client-token-refresh`
- **Touches:** <...>
- **Done when:** <...>
- **Estimate:** <min–max min · confidence · basis>

### Phase 4 — Integrate auth + billing paths and run e2e `[lane: integration]` `[status: todo]`
- **Repository:** `owner/web`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** Phase 3, Phase 2
- **Parallelizable with:** none — integration point
- **Suggested branch:** `chore/integrate-auth-billing`
- **Touches:** <...>
- **Done when:** <both lanes merged and green>
- **Estimate:** <min–max min · confidence · basis>

### Phase 5 — Review pull requests `[lane: review]` `[status: todo]` `[kind: pr-review]` `[review: medium]`
- **Task:** [<slug> - <title>](task-url)
- **Depends on:** Phase 4
- **Parallelizable with:** none — review barrier
- **Done when:** every repository PR has a non-blocking review verdict

## Parallel execution guide
- **Lane A** (worktree 1): Phase 1 → Phase 3. Start immediately.
- **Lane B** (worktree 2): Phase 2. Independent — start immediately.
- **Integration point:** Phase 4 merges Lanes A + B in a single worktree after both land.
- **Conflict watch:** Phases 2 and 3 both touch `src/api/*` — the second to merge rebases first.
- **Merge order:** Lane B → Lane A → integration (or note the real order).

## Progress log
- <YYYY-MM-DD> — <what moved, by which lane/worktree>. (appended by loop-handoff)
```

## `.loop/plan.md` header

The first lines of `.loop/plan.md` are always a `loop-plan` HTML comment carrying the plan's
coordinates so the other skills can act on it without re-searching:

```markdown
<!-- loop-plan
planId: loop-<effort-slug>-<date>-<HHMMSS|rand>
daemon: http://localhost:7717
kestralProject: <name>            # these four keys present ONLY when the plan is linked to Kestral
kestralProjectId: <id>
kestralWorkContextId: <doc id>
kestralDocUrl: <url>
-->
```

Keep this comment as the first lines of `.loop/plan.md`. `planId` and `daemon` are **always**
present. The four `kestral*` keys appear **only** when the plan is linked to Kestral —
LINKED ⇔ `kestralWorkContextId` present. `loop-handoff` reads this header to push (to the
daemon always, to Kestral when linked); `loop-pickup` writes it after fetching.

## Rules

- **The plan is editable for its whole lifetime.** Re-read it at every orchestration boundary;
  execution and even a completed review do not freeze it. Phase numbers are stable identities,
  while document order is presentation/execution order, so IDs may be non-monotonic after an
  insertion. Allocate the next unused number; never renumber or reuse an existing phase once
  execution starts because notes, events, tasks, and state are keyed by that number.
- **Every executable plan ends with an explicit PR-review phase.** `loop-execute` owns
  `[kind: pr-review]` phases; lane runners never claim them. When work is added while the last
  review is still pending, place the new work block immediately before that review and update
  its `Depends on` closure—do not add another review. When work is added after the last review
  is `done`, keep that completed review in place, append the work after it, then append a new
  terminal PR-review phase. Never reopen or move a completed review.

- **No fake parallelism — default to one lane.** Decompose the work into the phases it
  naturally has first; only then look for independence. Split phases into separate lanes
  only when they pass all three independence checks (no dependency, low file contention, no
  coordination tax — see `multiphase-plan` step 2 → Independence test). Never reshape,
  split, or reorder phases, and never add stubs/mocks/frozen interfaces/flags, just to
  enable parallel work. A fully sequential, single-lane plan is the expected outcome for
  most efforts; more lanes is not better. When in doubt, keep it sequential.
- **Vertical slices, not layers.** For full-stack work a phase delivers the backend *and*
  frontend of one capability together — never a backend-only phase followed later by its
  frontend-only phase. If a slice is too big for one phase, its backend and frontend phases
  must sit in the same lane, share one `Suggested branch`, and land in the same PR — record
  "same PR as Phase N" in both phases' **Notes:**. Layer-only phases are fine only for
  genuinely single-layer work or a shared foundation several slices build on.
- **Every phase has a testable *Done when*.** A phase without acceptance criteria can't be
  claimed or handed off cleanly. A runnable **Verify:** line (a command, exit 0 = pass) is
  what lets `loop-execute` enforce the *Done when* mechanically.
- **Bound Loop work phases conservatively.** Use an evidence-based `Estimate:` on each work phase.
  Split phases whose likely upper bound exceeds 25 minutes without making artificial
  parallel lanes. The runner's 30-minute cutoff is a checkpoint, not proof that a phase
  will finish in one attempt.
- **One parent effort task per plan; work and review phases are its subtasks.** The plan maps
  to a single
  Kestral parent task (tag `multiphase-plan`, linked from the doc's `**Effort task:**`
  line); each phase is a subtask of it (`parentTaskId`), never a sibling top-level task.
- **Loop mode ships one PR per repository.** Per-phase Suggested branches are short-lived
  lane branches cut from that repository's integration tip and merged back; no per-phase
  PRs; `[status: done]` = merged into the phase repository's integration branch.
- **Plan doc and Kestral tasks cross-reference.** Each phase's **Task:** line links its
  subtask; each subtask carries `tags: ["phase:<N>", "lane:<X>"]`.
- **Status is kept in lockstep across its sinks:** the `[status: …]` marker in the doc and the
  daemon (`loop-handoff` mirrors it there via `loop-plan.sh push` always), plus the linked
  task's Kestral status when the plan is linked. `loop-handoff` updates all of them.
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
