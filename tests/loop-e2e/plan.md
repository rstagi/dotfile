<!-- loop-plan
planId: e2e-skills-plan
-->

# Skills engine e2e plan — Multi-Phase Plan

**Status:** in progress
**Last updated:** 2026-10-26 by e2e harness

## Loop config

- **Integration branch:** `feat/e2e-m1`
- **Concurrency:** 2

## Repositories

### `acme/e2e`
- **Verify:** `true`

## Goal
Run three work phases in two lanes, one AR review and one merge gate.

## Milestone 1 — Three work phases ship
- **Integration branch (acme/e2e):** `feat/e2e-m1`

### Phase 1 — Add the alpha file `[lane: A]` `[status: todo]`
- **Repository:** `acme/e2e`
- **Depends on:** none
- **Suggested branch:** `feat/e2e-alpha`
- **Done when:** `alpha.txt` exists

### Phase 2 — Add the beta file `[lane: B]` `[status: todo]`
- **Repository:** `acme/e2e`
- **Depends on:** none
- **Suggested branch:** `feat/e2e-beta`
- **Done when:** `beta.txt` exists

### Phase 3 — Add the gamma file `[lane: A]` `[status: todo]`
- **Repository:** `acme/e2e`
- **Depends on:** Phase 1
- **Suggested branch:** `feat/e2e-gamma`
- **Done when:** `gamma.txt` exists

### Phase 4 — Review milestone 1 `[lane: review]` `[status: todo]` `[kind: pr-review]` `[gate: merge]` `[review: standard]` `[rounds: 1]`
- **Repository:** `acme/e2e`
- **Depends on:** Phase 2, Phase 3
