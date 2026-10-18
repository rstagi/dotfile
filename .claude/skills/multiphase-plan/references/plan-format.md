# Canonical Multi-Phase Plan Format

This is the shared contract between `multiphase-plan` (authors it), `loop-pickup`
(reads it to claim a lane), and `loop-handoff` (reconciles + repushes it). All three
skills — run by Claude Code **or** Codex, in any worktree — read and write this exact
shape. Keep the inline markers stable; they are parsed, not just displayed.

The plan lives on disk in a flattened `.loop/` dir (this `plan.md` sits beside `state.json`).
Every plan is backed by the central Loop Observatory daemon (`http://localhost:7717`) — the
only backend.

Repository checkouts are deliberately absent from the plan. Plans carry GitHub
`owner/repo` slugs only; each machine resolves them through `~/.loop/repos.json` via
`loop-repo.sh map|get|list|unmap`. Every phase names exactly one repository. A lane may move
between repositories.

## Markers (must stay machine-parseable)

- Each work-phase heading ends with `` `[lane: X]` `` and
  `` `[status: todo|in-progress|blocked|done]` ``. A review phase adds
  `` `[kind: pr-review]` ``; phases without `kind:` are work phases.
- A review phase may add `` `[review: standard|max]` `` (default `standard`),
  `` `[rounds: N]` `` (default 2 outer rounds), and `` `[inner: N]` `` (default 2 inner
  exchanges per round). Both counts accept `1 ≤ N ≤ 5`. Tiers shallow and medium are
  rejected by execution paths; use standard or max for new plans.
  Standard rotates Sol / Opus / Grok; max rotates Astra / Fable / Grok, with reviewer R
  and critic C from different providers. Each inner exchange is R's review or response
  followed by C's audit. An opening approval agreed by C requires the third provider's
  confirmation before clean early stop. Disagreement at the inner cap writes disputed
  evidence for verification against the code. Only the fixer/planner edits the artifact,
  between outer rounds. PR review uses Opus fixes when needed and ends with one Opus
  verdict; `loop-review.sh next` advances the ledger-driven pipeline.
- A review phase that ends a non-final milestone adds `` `[gate: merge]` `` (see
  **Milestones** below); the tag is only valid together with `[kind: pr-review]`.
- `lane:` groups phases that one worktree owns end-to-end. Same letter = same lane =
  sequential within that worktree. Different letters = independent = parallel worktrees.
- `status:` is the source of truth for progress in the document; `loop-handoff` mirrors it to
  the daemon.
- `Repository:` is mandatory on every work phase when `## Repositories` exists and must match
  one declared slug. A PR-review phase covers every repository and omits `Repository:`.
  Repository `Verify:` is mandatory; a work phase `Verify:` overrides it.
- `Estimate:` is required on every work phase of a new Loop plan. It is a human-readable range,
  confidence, and basis; the parser ignores it. Its upper bound should normally be at most
  25 minutes to leave room before the runner's 30-minute checkpoint. State exceptions
  openly rather than claiming a bound the evidence cannot support.
- Legacy plans without `## Repositories` normalize to one synthetic `primary` repository;
  their scalar Loop config `Verify` / `PR` fields remain executable.
- Legacy plans may still carry `kestral*` header keys, `**Kestral project:**` /
  `**Plan doc:**` / `**Effort task:**` header lines, or `**Task:**` phase lines — readers
  ignore them.

## Milestones (optional)

One plan may carry several **milestones**: macro phases that each ship as their own PR per
participating repository and end with a merge gate. Phase numbers stay global and stable across
milestones.

- **Sections.** `## Milestone N — <outcome>` sections replace `## Phases`; a plain `## Phases` plan is one implicit milestone 1 (backward compatible — nothing below applies to it).
- **Branch lines.** Before its first `### Phase`, a milestone section may carry
  `- **Integration branch:** \`<branch>\`` and, per repository,
  `- **Integration branch (owner/repo):** \`<branch>\``.
  - Precedence: milestone repository line > milestone line > (milestone 1 only) `## Repositories` override > Loop config.
  - Milestones ≥2 must declare their branch (no global fallback).
  - A branch may not repeat for the same repository across milestones.
- **PR lines.** `- **PR:** …` (single repository) or `- **PR (owner/repo):** …` carry the
  per-milestone PR links. The `## Repositories` `PR:` line applies only to milestone-less plans.
- **Participants.** A milestone's participants are the repositories of its work phases.
  Prepare, PR creation, review, and the merge gate are scoped to participants.
- **Gate.** Every non-final milestone ends with a `` `[kind: pr-review]` `` `` `[gate: merge]` ``
  phase. The root phases of milestone N+1 must list the gate in `Depends on` explicitly (the
  parser warns, `loop-execute` preflight rejects if missing). Milestones always block the next;
  there is no stacking. After the gate review verdict is posted the loop parks in
  `awaiting-merge` (plan Status `awaiting merge`) until a human merges and runs
  `/loop-execute resume`.
- **Worktrees.** Milestone 1 keeps `…/<owner--repo>/integration`; milestones ≥2 use
  `…/<owner--repo>/integration-<branch-slug>-<hash8>` (`<hash8>` = 8-char sha1 of the exact
  branch), always created fresh from the fetched default
  branch (adoption of an existing branch/worktree applies to milestone 1 only).

### Archived milestones

A plan keeps full phase blocks only for milestones that have not shipped. The orchestrator and
the AR plan reviewers read the full plan at each step. A shorter plan uses less of their context.
After the gate PRs of milestone N merge, `loop-plan.sh archive --milestone N` moves milestone N
out of `plan.md`.

- **Archive file.** `milestones/m<N>.md` sits beside `plan.md` (in Loop mode:
  `.loop/milestones/m<N>.md`). It holds the milestone section (heading, branch and PR lines,
  phase blocks) and the progress-log entries of that milestone. The command moves them without
  changes.
- **Retro.** When `milestones/m<N>.retro.md` exists beside the plan, the archive file has the
  retro first, then `## Raw log`, then the moved section and progress-log entries without
  changes. The retro is the "Analysis and recap" that the `ship` explainer runner writes at
  the merge gate (`loop-execute` protocol → Milestone retrospective). After the write, the
  command removes the retro file. Its content now lives in the archive file.
- **Stub.** The milestone section in `plan.md` becomes a stub with this shape:

  ```markdown
  ## Milestone 1 — Backend ships behind a flag `[archived]`
  - **Integration branch:** `feat/token-refresh-backend`
  - **PR:** https://github.com/owner/api/pull/12
  - **Outcome:** Backend ships behind a flag
  - **Repositories:** `owner/api`
  - **Phases (all done):** 1, 2
  - **Gate:** Phase 2
  - **Archive:** [milestones/m1.md](milestones/m1.md)
  ```

  - The heading keeps its text and ends with `` `[archived]` ``.
  - The `Integration branch` and `PR` lines are the milestone's own lines, without changes.
  - `Outcome:` is one line. `--outcome <text>` sets it. The default is the Outcome bullet of
    the retro, else the milestone title.
  - `Repositories:` lists the participants.
  - `Phases (all done):` lists every phase number of the milestone in document order.
  - `Gate:` names the `[gate: merge]` review phase. The phase list contains it.
  - `Archive:` is a relative link to the archive file.
- **Progress log.** An entry is about milestone N when the text after its date and dash starts
  with `Phase <n>` (n is a phase of N), `Milestone N` or `MN`. The command moves these entries
  with their indented continuation lines. All other entries stay in `plan.md`.
- **Readers.** A phase that an `[archived]` stub lists is an existing phase with status `done`.
  - It needs no *Done when*, no `Repository:` and no `Estimate:`.
  - A `Depends on` edge to it is satisfied.
  - Its number stays reserved. Never reuse it.
  - Never claim, reopen, renumber or drop it. Never copy its block back into `plan.md`.
  - `.loop/state.json` keeps its entries.
  - The Loop daemon gives it the title `(archived)` and a display-only dependency chain in stub
    order.
- **Command rules.**
  - The command refuses with exit 3 in these conditions:
    - A phase of N is not `done`.
    - N has no `[gate: merge]` phase.
    - A participant of N has no PR link.
  - It reads the state of each PR link with the `loop-milestone.sh check` rule. An open PR
    gives exit 3. A PR that is closed and not merged gives exit 4. A `gh` error gives exit 1.
  - It is atomic. It writes the archive file first. It replaces `plan.md` last, with one
    rename. On any failure, `plan.md` stays unchanged.
  - It is idempotent. Without a pending retro file, a second run on an archived milestone
    writes nothing and pushes nothing.
  - An archived milestone with a pending retro file is not a no-op. The command attaches the
    retro to the existing archive file, updates the stub `Outcome:` line, removes the retro
    file and pushes. A newer retro replaces an attached retro.
  - After the write, it runs `loop-plan.sh push`.
  - A final milestone that has no `[gate: merge]` phase cannot be archived.

## Document template

> The template below shows four phases across two lanes purely to illustrate every marker.
> It is **not** a target shape — most real plans are a single lane (`[lane: A]` on every
> phase) with no "Parallel execution guide" section. Do not add lanes to match this example.

```markdown
# <Effort name> — Multi-Phase Plan

**Status:** planning | in progress | awaiting merge | integrating | done
**Last updated:** <YYYY-MM-DD> by <host: Claude Code | Codex> (<worktree/branch>)

## Loop config

> Optional — present only when the effort will run under `loop-execute`. Omit otherwise.

- **Integration branch:** `<type>/<effort-slug>`
- **Concurrency:** 3

Default the integration branch to the current checkout's branch when it is not the
repository's default branch (loop-execute then adopts that branch/worktree); otherwise
`<type>/<effort-slug>`.

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
> *does*, not its position. The `Phase N` label lives only in the heading, never in the
> branch, worktree, commit, or PR name. (See "Naming" in the Rules section.)

### Phase 1 — Add OAuth token refresh endpoint `[lane: A]` `[status: todo]`
- **Repository:** `owner/api`
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
- **Depends on:** none
- **Parallelizable with:** Phase 1 (independent)
- **Suggested branch:** `refactor/invoice-retry-state`
- **Touches:** <...>
- **Done when:** <...>
- **Estimate:** <min–max min · confidence · basis>
- **Verify:** <optional — runnable acceptance check, exit 0 = pass>

### Phase 3 — Use refreshed tokens in the API client `[lane: A]` `[status: todo]`
- **Repository:** `owner/web`
- **Depends on:** Phase 1
- **Parallelizable with:** Phase 2 (independent)
- **Suggested branch:** `feat/api-client-token-refresh`
- **Touches:** <...>
- **Done when:** <...>
- **Estimate:** <min–max min · confidence · basis>

### Phase 4 — Integrate auth + billing paths and run e2e `[lane: integration]` `[status: todo]`
- **Repository:** `owner/web`
- **Depends on:** Phase 3, Phase 2
- **Parallelizable with:** none — integration point
- **Suggested branch:** `chore/integrate-auth-billing`
- **Touches:** <...>
- **Done when:** <both lanes merged and green>
- **Estimate:** <min–max min · confidence · basis>

### Phase 5 — Review pull requests `[lane: review]` `[status: todo]` `[kind: pr-review]` `[review: standard]`
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

### Two-milestone variant

Replace `## Phases` with one section per milestone (phase numbers continue across them; only
the structure is shown):

```markdown
## Milestone 1 — Backend ships behind a flag
- **Integration branch:** `feat/token-refresh-backend`
- **PR:** _none yet_

### Phase 1 — Add OAuth token refresh endpoint `[lane: A]` `[status: todo]`
- **Repository:** `owner/api`
- **Depends on:** none
- ...

### Phase 2 — Review milestone 1 `[lane: review]` `[status: todo]` `[kind: pr-review]` `[gate: merge]`
- **Depends on:** Phase 1
- **Done when:** every participating repository PR has a non-blocking review verdict

## Milestone 2 — Web client uses refreshed tokens
- **Integration branch (owner/web):** `feat/token-refresh-client`
- **PR (owner/web):** _none yet_

### Phase 3 — Use refreshed tokens in the API client `[lane: A]` `[status: todo]`
- **Repository:** `owner/web`
- **Depends on:** Phase 2
- ...

### Phase 4 — Review milestone 2 `[lane: review]` `[status: todo]` `[kind: pr-review]`
- **Depends on:** Phase 3
- **Done when:** every participating repository PR has a non-blocking review verdict
```

The final milestone ends with the ordinary terminal review (no `[gate: merge]`).

## `.loop/plan.md` header

The first lines of `.loop/plan.md` are always a `loop-plan` HTML comment carrying the plan's
coordinates so the other skills can act on it without re-searching:

```markdown
<!-- loop-plan
planId: loop-<effort-slug>-<date>-<HHMMSS|rand>
daemon: http://localhost:7717
-->
```

Keep this comment as the first lines of `.loop/plan.md`. `planId` and `daemon` are **always**
present. `loop-handoff` reads this header to push to the daemon; `loop-pickup` writes it after
fetching.

## Rules

- **The plan is editable for its whole lifetime.** Re-read it at every orchestration boundary;
  execution and even a completed review do not freeze it. Phase numbers are stable identities,
  while document order is presentation/execution order, so IDs may be non-monotonic after an
  insertion. Allocate the next unused number; never renumber or reuse an existing phase once
  execution starts because notes, events, and state are keyed by that number.
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
- **Loop mode ships one PR per repository.** Per-phase Suggested branches are short-lived
  lane branches cut from that repository's integration tip and merged back; no per-phase
  PRs; `[status: done]` = merged into the phase repository's integration branch.
- **Status is kept in lockstep:** the `[status: …]` marker in the doc and the daemon
  (`loop-handoff` mirrors it there via `loop-plan.sh push`).
- **Naming: describe the work, never the index.** Phase titles, `Suggested branch`,
  worktrees, commits, and PRs must say *what the change does* — never `phase-4`,
  `product-split-phase-4`, `<effort>-phase-N`, or a bare number. The phase↔branch link is
  already carried by the plan doc, so it does not belong in the branch name.
  - **Phase title** — an imperative outcome: "Add OAuth token refresh endpoint", not
    "Phase 1" or "Auth stuff". This title usually seeds the PR title.
  - **Suggested branch** — `<type>/<imperative-outcome-slug>`, kebab-case, where `<type>` is
    a conventional prefix (`feat`, `fix`, `refactor`, `chore`, `docs`, `test`). E.g.
    `feat/oauth-token-refresh`, `refactor/invoice-retry-state`. Keep it under ~50 chars, no
    phase number, no effort prefix.
  - Bad → good: `product-split-phase-4` → `chore/integrate-auth-billing`;
    `phase-2-oauth` → `feat/oauth-token-refresh`.
