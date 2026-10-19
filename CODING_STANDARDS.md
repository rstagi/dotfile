# Coding standards: fast and erasable

A plan or a PR passes on two tests. It does not break existing behavior. It is easy to remove. Write down all other concerns as deferred issues. Do not block on them.

These rules apply to new work. Existing code moves to this layout only in its own refactor PR.

## Minimal footprint

- Keep the number of edits to existing files small. The number of new files does not matter. New files go in the folder of the feature.
- Give a reason for each edit to an existing shared file.
- Each feature has one wiring point, for example a route registration, a dependency-injection binding or a plugin list. The flag check of the feature is at that wiring point.
- Do not put unrelated changes in a feature PR. Renames, refactors, formatting changes and dependency updates go in their own PR.
- When a feature needs a new seam in existing code, ship a small refactor PR with no behavior change first. Then the feature PR only adds code.
- When a PR edits more than 3 existing files outside its feature folder, the plan gives the reason. This is a point for discussion, not a block.

## Architecture: hexagonal, one slice per feature

- Each feature is a vertical slice: `feature/<name>/{domain,ports,adapters,tests}`.
- The domain is pure. It does no I/O.
- Dependencies point inward only: adapters → ports → domain.
- A feature does not import the internals of another feature. Shared code goes through a published port or a shared kernel.
- No import cycles.
- Architecture tests in CI enforce these rules. A reviewer opinion does not.

## Feature flags

- Each behavior change ships behind a flag that is off by default. An equivalent off switch is also good, for example a config toggle or a new endpoint that no caller uses yet.
- The code checks the flag one time, at the wiring point. Do not put flag checks in other places.
- The relevant tests run with the flag off and with the flag on.
- Each flag has an owner and a removal note. The removal of a flag is its own small PR.

## Test-driven development

- Write the test for domain logic first. Domain tests do no I/O. They use in-memory fakes for the ports.
- Each adapter has a contract test against its port.
- Tests live in the feature folder. When you remove the feature, you also remove its tests.

## Observability

- Add spans, structured logs and key metrics at the entry point of the feature and at the adapter boundaries. Do not add them in the domain core.
- Start each telemetry name with `feature.<name>.`. Then you can find the telemetry of a feature and remove it with the feature.
- Never log secrets or raw sensitive data.

## Delete test

- Remove the folder, the wiring line and the flag of a feature. When the build stays green and the existing tests pass, the feature is erasable.
- Add an automated delete test where it is practical.

## Review rules

Block a plan or a PR only for these 3 reasons:

1. **Security or safety.** Examples: a leaked secret, an auth or permission bypass, data exposure, and unsafe handling of user data. Also any change that can cause real harm in production.
2. **Unprotected breakage of existing behavior.** The PR changes current behavior, and no feature flag or equivalent off switch protects it. Or the PR breaks existing tests with the flag off.
3. **A failing deterministic check.** Examples: the architecture tests, the flag off/on test matrix or the delete test. These block because CI fails, not because a reviewer decides so.

All other items are deferred issues. They never block. Examples: names, style, extra abstractions, missing edge cases in flagged code, performance ideas, more tests, cleaner refactors and documentation. Put them in a short "Deferred issues" list and continue.

Process:

- An adversarial review has at most 2 rounds by default. A round that finds no blocking item ends the review. A later round checks only the fix of the blocking items. It does not look for new deferred issues.
- Keep the deferred list short and concrete. Remove each item that does not deserve a follow-up ticket.
- Do not widen the scope of a PR to fix deferred items.
- Triage deferred issues one time each week, not in the PR.
- Do not raise the severity of an item to put it in the blocking list. When you are not sure, the item is deferred.

## Deterministic checks

A new repository sets up these checks in CI from the start:

- Architecture tests: slice boundaries, dependency direction and import cycles.
- The flag off/on test matrix.
- The delete test.

This repository does not add them. It has no feature slices and no CI workflow, and `ws` moves to its own repository later. Here, a reviewer reports a probable failure of these rules as a deferred issue. A check blocks only when it exists and fails.
