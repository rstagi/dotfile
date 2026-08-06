## HIL: Rebuild the cart state machine

**Phase:** 2 (lane B) · branch `refactor/cart-state`

The runner is blocked. Attempts so far:

| Attempt | Engine → model | Outcome |
|---|---|---|
| a1 | claude → opus  | done, but Verify failed (non-deterministic transitions) |
| a2 | claude → fable | blocked — upstream cart API contract missing |

**Blocking question:** the cart API contract (`src/cart/contract.ts`) does not exist yet.
Which path do we take?

- **A.** Freeze a minimal contract now from the storefront's current usage; note it as tech debt.
- **B.** Pause lane B until the platform team publishes the contract.
- **C.** Infer the contract from the payments-gateway integration in lane A.

**Recommendation:** A — unblocks now and is the cheapest to revise later.

Reply in the orchestrator chat, or write `hil/cart-state-machine.answer.md`.
