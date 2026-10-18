<!-- loop-plan
planId: loop-archive-fixture-2026-10-01-000000
daemon: http://localhost:7717
-->

# Archive fixture — Multi-Phase Plan

**Status:** awaiting merge
**Last updated:** 2026-10-04 by Claude Code (fixture)

## Loop config

- **Integration branch:** `feat/m1`
- **Concurrency:** 2

## Repositories

### `acme/api`
- **Verify:** `npm test`
- **PR:** _none yet_

### `acme/web`
- **Verify:** `npm run check`
- **PR:** _none yet_

## Goal
Ship the core API and web shell, then connect them, then add rate limits.

## Approach & key decisions
- One sequential lane per milestone.

## Milestone 1 — Core API and web shell ship `[archived]`
- **Integration branch (acme/api):** `feat/m1-api`
- **Integration branch (acme/web):** `feat/m1-web`
- **PR (acme/api):** https://github.com/acme/api/pull/1
- **PR (acme/web):** https://github.com/acme/web/pull/2
- **Outcome:** The core API answers `GET /health` and the web shell renders. PRs acme/api#1 and acme/web#2 merged with the final verdict APPROVE.
- **Repositories:** `acme/api`, `acme/web`
- **Phases (all done):** 1, 2, 9, 3
- **Gate:** Phase 3
- **Archive:** [milestones/m1.md](milestones/m1.md)

## Milestone 2 — The web shell calls the API
- **Integration branch (acme/web):** `feat/m2-web`
- **PR (acme/web):** _none yet_

### Phase 4 — Call the API from the web shell `[lane: A]` `[status: in-progress]`
- **Repository:** `acme/web`
- **Depends on:** Phase 3
- **Suggested branch:** `feat/web-calls-api`
- **Done when:** the shell shows the API health
- **Estimate:** 10–20 min · medium confidence · one fetch

### Phase 10 — Show an error banner when the API is down `[lane: A]` `[status: todo]`
- **Repository:** `acme/web`
- **Depends on:** Phase 4
- **Suggested branch:** `feat/api-down-banner`
- **Done when:** the banner shows when the API is down
- **Estimate:** 10–20 min · medium confidence · one component

### Phase 5 — Review milestone 2 `[lane: review]` `[status: todo]` `[kind: pr-review]` `[gate: merge]`
- **Depends on:** Phase 10
- **Done when:** every participating repository PR has a non-blocking review verdict

## Milestone 3 — The API enforces rate limits
- **Integration branch (acme/api):** `feat/m3-api`
- **PR (acme/api):** _none yet_

### Phase 6 — Add rate limits `[lane: A]` `[status: todo]`
- **Repository:** `acme/api`
- **Depends on:** Phase 5
- **Suggested branch:** `feat/rate-limits`
- **Done when:** the API returns 429 over the limit
- **Estimate:** 15–25 min · medium confidence · middleware

### Phase 7 — Review milestone 3 `[lane: review]` `[status: todo]` `[kind: pr-review]`
- **Depends on:** Phase 6
- **Done when:** every participating repository PR has a non-blocking review verdict

## Parallel execution guide
- **Milestone 1:** Lane A (Phase 1) and Lane B (Phases 2, 9) run in parallel.

## Progress log
- 2026-10-01 — Plan drafted (Claude Code, main).
- 2026-10-04 — Phase 4 claimed (lane A, codex)
- 2026-10-04 — Phase 10 added to milestone 2 (Claude Code, gate review).
- 2026-10-04 — Phases 4 and 6 revised after the milestone 1 gate.
