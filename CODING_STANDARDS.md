# Coding standards: fast and erasable

A plan or a PR passes on two tests. It does not break existing behavior. It is easy to remove. The defer policy of the repository and the owner decide which other concerns the PR fixes and which concerns go to follow-up work.

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

A review gives each finding two things: a **severity** and a **disposition**. The severity goes on the PR. The disposition goes only in the chat.

### Hard blocks

These 3 reasons always block a plan or a PR. Nobody defers a hard block:

1. **Security or safety.** Examples: a leaked secret, an auth or permission bypass, data exposure, and unsafe handling of user data. Also any change that can cause real harm in production.
2. **Unprotected breakage of existing behavior.** The PR changes current behavior, and no feature flag or equivalent off switch protects it. Or the PR breaks existing tests with the flag off.
3. **A failing deterministic check.** Examples: the architecture tests, the flag off/on test matrix, the monitoring check or the delete test. These block because CI fails, not because a reviewer decides so.

### Severity: major or minor

- `[major]`: a real defect, with evidence (a repro, a failing output or the code path). A hard block is always major. Other examples: wrong behavior or a data bug in new code, also in flagged code, a missing test for the main new behavior, and code that breaks a contract that the author wrote.
- `[minor]`: all other findings. Examples: names, style, extra abstractions, performance ideas, more tests for edge cases, cleaner refactors and documentation.
- Do not raise a severity. When you are not sure, the finding is minor. A finding with only reasoning and no evidence is minor.

### Disposition: fix now or defer

- A hard block is always "fix now".
- For each other finding, the agent proposes "fix now" or "defer" with a reason in one sentence. The defer policy of the repository gives the default proposal. The owner and the agent decide in the chat, one finding at a time when necessary.
- In a run with no owner in the chat (headless, Loop), the run fixes only the hard blocks. It lists each other finding with its proposed disposition for the owner at the next checkpoint. The owner decides there.

### Defer policy of a repository

| Policy | Default proposal | Repositories |
|---|---|---|
| `flag` | Defer a major behind an off-by-default feature flag. Fix now a major with no flag. Defer a minor. | Ratel Cloud |
| `strict` | Fix now each major, also behind a flag. Fix now a minor when the fix is small and in the scope of the PR. Defer only with a reason, and only when the owner agrees. | Beluga and the other open-source repositories |

Find the policy in this order: the line `Review defer policy: flag` or `Review defer policy: strict` in the repository CLAUDE.md or AGENTS.md, then the table above, then the owner. Until the owner answers, use `strict` for a public repository and `flag` for a private repository.

Process:

- An adversarial review has at most 2 rounds by default. A round that finds no blocking item ends the review. A blocking item is a finding with the disposition "fix now". A later round checks only the fix of the blocking items. It does not look for new findings to defer.
- Keep the deferred list short and concrete. Remove each item that does not deserve a follow-up ticket.
- Do not widen the scope of a PR to fix deferred items.
- Triage deferred issues one time each week, not in the PR.

Posting a review to GitHub:

- Post only after a human approves the preview in chat. This rule applies to each run, also to a headless run. A headless run writes the review to a file and does not post it.
- Put each issue in an inline comment on its diff line. Start each comment with its severity: `[major]` or `[minor]`.
- Never write the disposition on the PR: no "defer", no "follow-up", no "fix before the merge". Report the dispositions in the chat.
- Keep the review body short. Do not put the validation, the footprint or a list of the issues in it.
  - Write: "We found N major issues and M minor issues." Omit a part with a count of zero. With no issues, write: "We found no issues."
  - Write "issues", not "items".
- A posted review shows the findings only for a single-shot review or for the last round of an adversarial review. In an earlier round, the fixer fixes the blocking items at once, and nothing is posted.
- Put the proof of work in the PR description, as "Pull requests and proof of work" tells you. Do not put it in the review or in a PR comment.
- Never put screenshots of the review page in the PR.

## Pull requests and proof of work

- An agent opens each PR as a draft (`gh pr create --draft`). Only a human marks a PR ready. An agent never runs `gh pr ready`.
- An agent never posts a review before a human approves it in chat.
- An agent always produces proof of work for each PR.
  - Proof shows the changed behavior in an actual run: a screenshot, a screen recording or a terminal session of the real program.
  - A terminal GIF of the program at work is good proof. A recording that only shows a test suite or a unit test run is not proof. Do not show a test report as proof.
  - A change with no user interface gets a terminal session of the real command. A document change gets a screenshot of the rendered document.
  - Use test data. Never show secrets, keys or real user data.
  - Never commit proof files.
- An agent attaches the visual proof to the PR description, in one `## Proof` section. Convert each recording to a GIF so that GitHub shows it in the description.
- Work with no proof is not done. When you cannot produce proof, stop and tell a human why.

## Deterministic checks

A new repository sets up these checks in CI from the start:

- Architecture tests: slice boundaries, dependency direction and import cycles.
- The flag off/on test matrix.
- The monitoring check: with the flag on, each feature emits at least one `feature.<name>.` span and one product event.
- The delete test.

This repository does not add them. It has no feature slices and no CI workflow, and `ws` moves to its own repository later. Here, a reviewer reports a probable failure of these rules as a minor issue. A check blocks only when it exists and fails.
