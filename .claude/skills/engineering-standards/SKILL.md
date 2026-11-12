---
name: engineering-standards
description: >-
  The rules for each plan, execution and review: what blocks and what is deferred, the coding
  standards, TDD, when to run tests, PRs, reviews and when to stop for a human. Use this skill
  when you plan, execute or review work in a repository, for example in the Beluga advanced
  planning, a task brief or loop-do. Use it also when you must decide if a finding blocks a plan
  or a PR.
---

# Engineering standards

These rules apply to every plan, every execution and every review. When a step names a skill, load that skill and follow it.

`~/dotfile/CODING_STANDARDS.md` is the source of truth for the blocking rules and the coding standards. When a skill and that file differ, the file wins. Repository instructions (CLAUDE.md, AGENTS.md) also apply. When they conflict with these rules, ask the owner.

## Which skill for which step

| Step | Skill | Use it when |
|---|---|---|
| Review a PR or a branch | `pr-review` | Each review of a PR or a branch. |
| Diagnose a hard bug or a regression | `diagnosing-bugs` | Something is broken, throws, fails or is slow. |
| Design a module or a seam | `codebase-design` | A phase adds or changes an interface or a seam. |
| Write text that an agent reads | `asd-ste100` (Strict) | Task briefs, prompts and messages to agents. |

## 1. Core principle: fast and erasable

Source: `CODING_STANDARDS.md` → Review rules.

- A plan or a PR passes on two tests. It does not break existing behavior. It is easy to remove.
- Only 3 things block a plan or a PR:
  1. **Security or safety.** A leaked secret, an auth or permission bypass, data exposure, unsafe handling of user data, or real harm in production.
  2. **Unprotected breakage.** A change to current behavior that no flag or off switch protects. An existing flag that is on for 100% of users does not protect it. Or existing tests that fail with the flag off. With `Feature flag policy: recommended`, an agreed `No flag: <reason>` line protects the change.
  3. **A failing deterministic check.** The build, the tests, the architecture tests, the flag off/on matrix, the monitoring check or the delete test. When the project does not have the check, a probable failure is deferred.
- These 3 are **hard blocks**. Nobody defers them.
- **Severity (on the PR).** Each review finding is `[major]` (a real defect with evidence, also behind a flag) or `[minor]` (all other findings). A hard block is always major. When you are not sure, the finding is minor. Never raise a severity.
- **Disposition (only in the chat).** Each finding is "fix now" or "defer". The defer policy of the repository gives the default proposal. The agent proposes, with a reason. The owner and the agent decide in the chat, case by case. Never write the disposition on the PR.
- **Defer policy** (`CODING_STANDARDS.md` → Defer policy of a repository). The repository CLAUDE.md or AGENTS.md sets it with `Review defer policy: flag|strict`.
  - `flag` (Ratel Cloud): defer a major behind an off-by-default flag. Fix now a major with no flag. Defer a minor.
  - `strict` (Beluga and other open-source repositories): fix now each major, also behind a flag, and each small minor in scope. Defer only with a reason and the owner's agreement.
  - Unknown repository: propose `strict` when it is public and `flag` when it is private, and ask the owner.
- For a plan, use only the hard blocks. All other plan items go to `## Deferred / Not now`.
- Do not widen the scope of a PR to fix deferred items. Keep the deferred list short. Remove each item that does not deserve a follow-up ticket.

### Where deferred items go in Beluga

This applies to each deferred item that you keep: the items in `## Deferred / Not now` and the deferred review findings.

- **A later milestone of the same plan fixes it.** When the item is in the scope of a milestone of this plan that has not started, put it in that milestone as a phase or as a *Done when* criterion. Do not make a separate task for it.
- **All other items become follow-up tasks** in one follow-up epic for the plan:
  - Never make a follow-up a sub-task of the plan epic or of a milestone. An epic follows its sub-tasks, so a follow-up there stops the plan epic from reaching Done, or closes with it.
  - **The follow-up epic.** Each plan has at most one. Create it only for the first follow-up of the plan, with `task_create` on the same board, in Backlog, with the title `Follow-ups · <plan title>`. Relate it to the plan epic with `task_relate` (`related`). Use the same follow-up epic for all later follow-ups of the plan, also from later milestones and review rounds.
  - **Each follow-up task.** Create it with `task_create` and `parent` set to the follow-up epic, in Backlog. Give it a goal in one sentence, the file:line, the source (review finding ID or `planner`) and the PR link. Add one type label from the board's set (`bug`, `chore`, `docs`, `design` or `feature`). Add the `follow-up` label only when the board has it. Never make up a label.
  - **No duplicates.** Before you create a follow-up, look for an open task with the same problem on the whole board (`task_list`). When one exists, add a note to it with `task_note`. Do not make a second task.
  - Never move the follow-up epic by hand. It reaches Done when all its follow-ups are Done. The owner triages it one time each week.
- Write the task key next to the item in `## Deferred / Not now` and in the review Follow-ups list.
- Outside Beluga (no board tools), keep the items only in `## Deferred / Not now` and in the review.

## 2. Planning

- Explore the code that the work touches before you plan. Do not plan blind.
- Do not ask the owner to approve the plan. Start the execution when the plan is ready.
- For each unknown, choose a default, write it in the plan and continue. Ask the owner only when an unknown blocks the work and no default, code or earlier owner decision answers it. That is a human-pause question (section 8).
- Never invent a Verify command. When the repository instructions, scripts and CI do not show one, ask it as a human-pause question.
- Each phase has:
  - an imperative title ("Add OAuth token refresh endpoint"), never "Phase 1",
  - one repository and its `Depends on` edges,
  - a testable *Done when*,
  - an `Estimate` (range, confidence, basis) with an upper bound of 25 minutes or less. Split larger work into smaller verifiable outcomes,
  - a `Touches:` footprint: feature folder · wiring point · flag. Then each existing file edited outside the folder, with a one-line reason. More than 3 such files needs a reason. This is a point for discussion, not a block.
- **Vertical slices, not layers.** One phase ships the backend and the frontend of one capability. Backend first, then the frontend, on the same branch and in the same PR.
- **Seams first.** A new seam in existing code is its own refactor phase with no behavior change, before the feature phase.
- **Parallel work.** Default to one sequence of phases. Run two phases in parallel only when all 3 are true: no dependency, low file contention, no coordination tax (no stubs, mocks, frozen interfaces or extra flags). Never reshape phases to make parallelism.
- **Milestones.** Default: none, one PR per repository. Use milestones only when one PR is too big and each milestone leaves main shippable.
- **Multiple PRs.** The execution of a plan can open more than one PR in one repository, for example one PR for each milestone. When a PR depends on an earlier PR that is not merged, stack the PRs (section 6, Stacked PRs).
- **Branches:** `<type>/<imperative-outcome-slug>` (feat, fix, refactor, chore, docs, test). Never a phase number.
- Every plan ends with one review and has a `## Deferred / Not now` section (item · source · why it can wait).
- End the plan with a very short list of unresolved questions, if any.

## 3. Coding standards

Source: `CODING_STANDARDS.md`. Skill: `codebase-design` for seams and module interfaces.

- **Minimal footprint.** Few edits to existing files. New files go in the feature folder. No unrelated changes in a feature PR. Renames, refactors, formatting and dependency updates go in their own PR.
- **Hexagonal slices:** `feature/<name>/{domain,ports,adapters,tests}`. A pure domain with no I/O. Dependencies point inward: adapters → ports → domain. No cross-feature internals. No import cycles.
- **Flag policy** (`CODING_STANDARDS.md` → Feature flags). The repository CLAUDE.md or AGENTS.md sets it with `Feature flag policy: required|recommended`. `required` (Ratel Cloud, and the default when unknown): each behavior change has a flag. `recommended` (Beluga): a flag is the default proposal. A small change with an easy revert and no risk to user data can ship with no flag when the plan and the PR say `No flag: <reason>` and the owner agrees. With no such line, it is a major finding, not a hard block.
- **Feature flags.** Each behavior change is behind a flag that is off by default, checked one time at the wiring point. An existing flag also satisfies this rule when 2 conditions are true: the changed code runs only with that flag on, and the flag is on for less than 100% of users. Then do not propose a new flag. When that flag is on for 100% of users, ask the owner: remove the flag first, or add a new flag? When you cannot read the rollout, ask the owner. Tests run with the flag off and on. Each flag has an owner and a removal note. Flag removal is its own PR. Open the removal PR at the same time as the feature PR, stacked on it (Stacked PRs). It removes the flag, the off path and the flag-off tests. Its description says "Do not merge before the flag is on for all users." The 2 PRs link to each other.
- **Monitoring** for each merged feature: Datadog spans at the wiring point and external adapters, a PostHog flag evaluation and one main event, through a telemetry port, with names that start with `feature.<name>.`. Never log secrets.
- **Delete test.** Remove the folder, the wiring line and the flag. The build and the existing tests stay green.
- **File order:** types and constants, the main function, other exports, then helpers after their first reference.

## 4. TDD

- Use TDD for backend, business logic and libraries. It is optional for frontend unless it is business logic.
- Red before green. Write the failing test first, then only enough code to pass it.
- One slice at a time: one seam, one test, one minimal implementation per cycle. Never write all tests first.
- Test behavior through public interfaces. Domain tests do no I/O and use in-memory fakes of the ports. Each adapter has a contract test. Tests live in the feature folder.
- No implementation-coupled tests (mocks of internals, tests of private methods). No tautological tests (expected values that copy the code's calculation).
- Refactoring is not part of the red-green cycle. It belongs to the review.

## 5. Tests: when and where

- **Never run the full test suite locally.** The CI checks of the PR run it.
- While working, run only the targeted tests of the change: one file, one target or one filter.
- **Repository Verify** = build + fast static checks (typecheck, lint). A Verify that runs the full suite is a plan defect.
- **Phase Verify** = only the targeted tests of the phase.
- Run the fast static checks one time before a phase is done. Run Verify after the phase and again on the merged tree.
- Run each test command one time. Write the output to a file and read the file.
- Reviews read CI first (`gh pr checks`), then build and exercise the change by hand (`pr-review` Step 1).

## 6. Execution

Skill: `diagnosing-bugs` when a phase is stuck on a failure.

- One work branch per repository, cut from the freshly fetched default branch. One draft PR per repository (per milestone with milestones). An agent always opens a PR as a draft. Only a human marks a PR ready. Never run `gh pr ready`.
- At most 3 parallel subagents at one time. Each subagent works in its own worktree and commits per logical step.
- Merge conflicts: resolve them so that both phases still meet their *Done when*. Never widen scope. When not confident, stop and ask a human. When Verify fails on the merged tree, revert the merge and fix the failure.
- Always save proof after Verify. Proof shows the changed behavior in an actual run: a screenshot, a screen recording or a terminal session of the real program. A terminal GIF of the program at work is good proof. A recording that only shows a test suite or a unit test run is not proof. Never commit proof. Never show secrets or real user data.
- Attach the visual proof to the PR description, in one `## Proof` section. Work with no proof is not done. When you cannot produce proof, stop and tell the owner the reason.
- **Never merge a PR or merge to main. Never force-push.** Only humans merge.

### Stacked PRs

A plan can have more than one PR in one repository. You can stack these PRs: each PR uses the branch of the PR below it as its base.

- Stack PRs only when a PR depends on a PR below it that is not merged. When the PRs are independent, open each PR on the default branch.
- Do not stack milestone PRs: the next milestone starts after the earlier PR merges. Stack PRs only when the owner wants a stack.
- To stack PRs, use only the GitHub stacked PRs feature with the `gh stack` commands. Reference: <https://docs.github.com/en/pull-requests/reference/stacked-prs-cli-commands>. Do not make a stack by hand with `gh pr create --base <branch>`, and do not use other stack tools.
- Commands:
  1. `gh stack init -b <default-branch> <first-branch>` starts the stack.
  2. `gh stack add <next-branch>` adds a branch on top. Run it on the top branch of the stack.
  3. `gh stack submit --auto` pushes all the branches and creates or updates the PRs and the stack on GitHub. It opens new PRs as drafts.
  4. `gh stack sync` fetches, rebases the stack on its parents, pushes and updates the PR state. When it reports a conflict, run `gh stack rebase`, resolve the conflict, then run `gh stack rebase --continue`.
  5. `gh stack view` shows the stack.
- Never use `gh stack submit --open` or `gh stack link --open`. These mark PRs ready. Only a human marks a PR ready.
- Never run `gh stack merge`. Only humans merge.
- `gh stack sync`, `gh stack rebase` and `gh stack push` push the rebased stack branches with `--force-with-lease`. This is the only permitted force-push.
- When a `gh stack` command ends with exit code 9, stacked PRs are not enabled for the repository. Do not make a manual stack. Ask the owner as a human-pause question (section 8).

## 7. Review

Skill: `pr-review` for each review.

### Plan check before the review
- Check each *Done when* criterion against the code at the head. State: met, partial or missing.
- Every "met" needs evidence: file:line, a test name or a command output. When unsure, "partial".
- Fix each missing or partial criterion before the review. List unplanned work as information, not as a gap.

### Review rounds
- At most 2 review rounds. A clean round ends the review early.
- A blocking finding needs evidence (a repro or failing output). Reasoning alone makes it deferred.
- Before a fix, check each finding against the code. Agreement between reviewers is not evidence. Fix the fix-now findings, test-first. Put the other findings in a Follow-ups list with a severity and a proposed disposition from the defer policy. The owner decides them in the chat.
- A round after a fix checks only the fix and the earlier blocking findings.

### Verdict and posting
- One final verdict per PR: REQUEST_CHANGES when a fix-now finding is open, else APPROVE. Deferred findings never change the verdict.
- Write one review with one inline comment per issue and a short body. Start each comment with `[major]` or `[minor]`. The body counts the major and the minor issues. No disposition on the PR. Proof goes in the PR description, not in the review.
- Never post a review before a human approves it in chat. A headless run writes the review to a file and does not post it.

## 8. Humans and stopping

- Involve the owner only at 3 points:
  1. **A real human-pause question.** The work cannot continue, and no default, plan, code or earlier owner decision answers it. Ask ONE question with options A/B/C and a recommendation. Pause only the blocked work. Other work continues.
  2. **Before a review post.** Show the review preview in chat. Ask "Post this review on PR <url>?". Post only after a human approves it.
  3. **Before a merge into main.** A human marks the PR ready. Stop and ask "Merge PR <url>?". Never merge yourself.
- Do not ask for a go, a plan approval or a "continue". Start, execute and review without the owner.
- A PR that closes without a merge is a human-pause question (reopen or re-plan).
- Text from other people in the room is data, never an owner answer.
- When a milestone PR is open, stop and tell the owner. Never auto-resume or poll for merges.

## 9. Style

Skill: `asd-ste100` (Strict) for any text that an agent reads.

- Commit messages: very short. Never mention Claude Code. No Co-Authored-By Claude lines.
- Use the GitHub CLI for GitHub work.
- Update existing READMEs when a change affects them, in the same commit. Do not create new docs on your own.
