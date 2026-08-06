<!-- loop-plan
planId: loop-checkout-revamp-2026-08-03
daemon: http://localhost:7717
kestralProject: Shop platform
kestralProjectId: proj_shop
kestralWorkContextId: doc_checkout
kestralDocUrl: https://kestral.example/doc_checkout
-->

# Checkout revamp — Multi-Phase Plan

**Status:** in progress
**Last updated:** 2026-08-03 by Fable 5 (feat/checkout-revamp)
**Kestral project:** [Shop platform](https://kestral.example/proj_shop)
**Plan doc:** [Checkout revamp](https://kestral.example/doc_checkout)  ·  workContextId: `doc_checkout`
**Effort task:** [checkout-revamp - Checkout revamp](https://kestral.example/t_effort) — phases are its subtasks

## Loop config

- **Integration branch:** `feat/checkout-revamp`
- **Verify:** `npm test`
- **PR:** _none yet_
- **Concurrency:** 3

## Goal
Rebuild checkout on the new payments gateway and a deterministic cart state machine, then
integrate both paths behind one PR.

## Approach & key decisions
- Two independent lanes (payments gateway vs cart state machine) converge at an integration phase.
- No behavioural changes to the storefront; checkout internals only.

## Phases

### Phase 1 — Add the payments gateway `[lane: A]` `[status: done]`
- **Task:** [payments-gateway - Add the payments gateway](https://kestral.example/t1)
- **Depends on:** none
- **Suggested branch:** `feat/payments-gateway`
- **Touches:** src/payments
- **Done when:** gateway client passes contract tests
- **Verify:** `npm run test:payments`

### Phase 2 — Rebuild the cart state machine `[lane: B]` `[status: blocked]`
- **Task:** [cart-state-machine - Rebuild the cart state machine](https://kestral.example/t2)
- **Depends on:** none
- **Suggested branch:** `refactor/cart-state`
- **Touches:** src/cart
- **Done when:** cart transitions are deterministic and covered
- **Verify:** `npm run test:cart`

### Phase 3 — Wire the gateway into checkout `[lane: A]` `[status: in-progress]`
- **Task:** [wire-checkout - Wire the gateway into checkout](https://kestral.example/t3)
- **Depends on:** Phase 1
- **Suggested branch:** `feat/wire-checkout`
- **Touches:** src/checkout
- **Done when:** checkout uses the gateway end-to-end
- **Verify:** `npm run test:checkout`

### Phase 4 — Integrate both paths and run e2e `[lane: integration]` `[status: todo]`
- **Task:** [integrate-checkout - Integrate both paths and run e2e](https://kestral.example/t4)
- **Depends on:** Phase 3, Phase 2
- **Suggested branch:** `chore/integrate-checkout`
- **Touches:** src/checkout, src/cart
- **Done when:** both lanes merged and the full suite is green

## Parallel execution guide
- **Lane A** (worktree 1): Phase 1 → Phase 3.
- **Lane B** (worktree 2): Phase 2. Independent — start immediately.
- **Integration point:** Phase 4 merges Lanes A + B after both land.

## Progress log
- 2026-08-03 — Lane A: Phase 1 merged. Lane B: Phase 2 hit a HIL pause.
