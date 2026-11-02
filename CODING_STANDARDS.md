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

## Monitoring

Each feature that merges has monitoring, also a prototype behind a flag. Merged code reaches production, and the owner must see if the feature works and if people use it. Scripts, spikes and one-off tools that do not merge into the product do not need monitoring.

The minimum for each feature:

- **Application monitoring (Datadog).** A span at the wiring point, with the flag state as a tag. A span at each adapter boundary that calls an external system. Errors are recorded on the span. Logs are structured and have a `feature` tag.
- **Product monitoring (PostHog).** The flag is evaluated through PostHog, so that exposure is recorded. At least one event for the main user action of the feature.

Rules:

- Telemetry goes through a port, for example `Telemetry`, with Datadog and PostHog adapters. The domain core does not call telemetry. Tests use an in-memory fake of the port.
- Start each span, metric and event name with `feature.<name>.`. Then you can find the telemetry of a feature in Datadog and PostHog, and remove it with the feature.
- A test with the flag on asserts that the entry point emits at least one span and one product event. This is the monitoring check.
- The PR description links to the Datadog and PostHog views for the feature, for example a saved query filtered on `feature.<name>.`.
- Never log or send secrets or raw sensitive data. Product events identify the user only by the same ID that the flag targets.
- The minimum above blocks only through the monitoring check, when that check exists and fails. Extra metrics, dashboards and alerts are deferred issues.

## Delete test

- Remove the folder, the wiring line and the flag of a feature. When the build stays green and the existing tests pass, the feature is erasable.
- Add an automated delete test where it is practical.

## Review rules

Block a plan or a PR only for these 3 reasons:

1. **Security or safety.** Examples: a leaked secret, an auth or permission bypass, data exposure, and unsafe handling of user data. Also any change that can cause real harm in production.
2. **Unprotected breakage of existing behavior.** The PR changes current behavior, and no feature flag or equivalent off switch protects it. Or the PR breaks existing tests with the flag off.
3. **A failing deterministic check.** Examples: the architecture tests, the flag off/on test matrix, the monitoring check or the delete test. These block because CI fails, not because a reviewer decides so.

All other items are deferred issues. They never block. Examples: names, style, extra abstractions, missing edge cases in flagged code, performance ideas, more tests, cleaner refactors and documentation. Put them in a short "Deferred issues" list and continue.

Process:

- An adversarial review has at most 2 rounds by default. A round that finds no blocking item ends the review. A later round checks only the fix of the blocking items. It does not look for new deferred issues.
- Keep the deferred list short and concrete. Remove each item that does not deserve a follow-up ticket.
- Do not widen the scope of a PR to fix deferred items.
- Triage deferred issues one time each week, not in the PR.
- Do not raise the severity of an item to put it in the blocking list. When you are not sure, the item is deferred.

Posting a review to GitHub:

- Post only after the owner approves the preview. A headless run has no owner, so it posts without a preview.
- Put each issue in an inline comment on its diff line. Start each comment with its class: `[blocking: <category>]` or `[deferred]`.
- Keep the review body short. Do not put the validation, the footprint or a list of the issues in it.
  - For deferred issues, write: "We found N issues. We defer their fix to follow-up work."
  - For blocking issues, write: "We found N blocking issues. Fix them before the merge."
  - Write "issues", not "items".
- A posted review shows blocking issues only for a single-shot review or for the last round of an adversarial review. In an earlier round, the fixer fixes the blocking issues at once, and nothing is posted.
- Put the screenshots and the recordings of the feature in the PR description, not in the review. A terminal recording is correct when it shows the feature at work. A recording that only runs tests is not correct. When no media of the feature exists, add no media. Never put screenshots of the review page in the PR.

## Deterministic checks

A new repository sets up these checks in CI from the start:

- Architecture tests: slice boundaries, dependency direction and import cycles.
- The flag off/on test matrix.
- The monitoring check: with the flag on, each feature emits at least one `feature.<name>.` span and one product event.
- The delete test.

This repository does not add them. It has no feature slices and no CI workflow, and `ws` moves to its own repository later. Here, a reviewer reports a probable failure of these rules as a deferred issue. A check blocks only when it exists and fails.
