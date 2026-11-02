---
name: engineering-standards
description: >-
  The rules for each plan, execution and review: what blocks and what is deferred, the coding
  standards, TDD, when to run tests, lanes, escalation, the review rounds and when to stop for a
  human. Use this skill when you plan, execute or review work in a repository, for example in the
  Beluga advanced planning, a task brief or a phase runner. Use it also when you must decide if a
  finding blocks a plan or a PR.
---

# Engineering standards

These rules apply to every plan, every execution and every review. When a step names a skill, load that skill and follow it.

`~/dotfile/CODING_STANDARDS.md` is the source of truth for the blocking rules and the coding standards. When a skill and that file differ, the file wins. Repository instructions (CLAUDE.md, AGENTS.md) also apply. When they conflict with these rules, ask the owner.

## Which skill for which step

| Step | Skill | Use it when |
|---|---|---|
| Continue in a fresh chat or worktree | `loop-pickup` | A new worktree or chat starts a lane. |
| Save progress for the next chat | `loop-handoff` | A work session ends or the worktree changes. |
| Review a PR or a branch | `pr-review` | Each review of a PR or a branch. |
| Diagnose a hard bug or a regression | `diagnosing-bugs` | Something is broken, throws, fails or is slow. |
| Design a module or a seam | `codebase-design` | A phase adds or changes an interface or a seam. |
| Write text that an agent reads | `asd-ste100` (Strict) | Task briefs, prompts, runner questions and messages to agents. |

## 1. Core principle: fast and erasable

Source: `CODING_STANDARDS.md` → Review rules.

- A plan or a PR passes on two tests. It does not break existing behavior. It is easy to remove.
- Only 3 things block a plan or a PR:
  1. **Security or safety.** A leaked secret, an auth or permission bypass, data exposure, unsafe handling of user data, or real harm in production.
  2. **Unprotected breakage.** A change to current behavior that no flag or off switch protects. Or existing tests that fail with the flag off.
  3. **A failing deterministic check.** The build, the tests, the architecture tests, the flag off/on matrix, the monitoring check or the delete test. When the project does not have the check, a probable failure is deferred.
- All other items are **deferred**. They never block. When you are not sure, the item is deferred. Never raise an item to blocking.
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
- **Lanes.** Default to one sequential lane. Put two phases in parallel lanes only when all 3 are true: no dependency, low file contention, no coordination tax (no stubs, mocks, frozen interfaces or extra flags). Never reshape phases to make parallelism.
- **Milestones.** Default: none, one PR per repository. Use milestones only when one PR is too big and each milestone leaves main shippable. Each non-final milestone ends with a review and a merge gate.
- **Branches:** `<type>/<imperative-outcome-slug>` (feat, fix, refactor, chore, docs, test). Never a phase number.
- Every plan ends with one terminal review phase and has a `## Deferred / Not now` section (item · source · why it can wait).
- **Review tier:** `standard` by default. `max` for large work or for security, auth, money or data integrity.
- **Plan review:** at most 2 adversarial rounds. Fix true blocking findings in the plan. Put deferred findings in Deferred / Not now. Round 2 runs only when round 1 fixed a blocking finding, and it checks only those edits.
- Never renumber or reuse a phase number. Add new work before a pending review phase.
- End the plan with a very short list of unresolved questions, if any.

## 3. Coding standards

Source: `CODING_STANDARDS.md`. Skill: `codebase-design` for seams and module interfaces.

- **Minimal footprint.** Few edits to existing files. New files go in the feature folder. No unrelated changes in a feature PR. Renames, refactors, formatting and dependency updates go in their own PR.
- **Hexagonal slices:** `feature/<name>/{domain,ports,adapters,tests}`. A pure domain with no I/O. Dependencies point inward: adapters → ports → domain. No cross-feature internals. No import cycles.
- **Feature flags.** Each behavior change is behind a flag that is off by default, checked one time at the wiring point. Tests run with the flag off and on. Each flag has an owner and a removal note. Flag removal is its own PR.
- **Monitoring** for each merged feature: Datadog spans at the wiring point and external adapters, a PostHog flag evaluation and one main event, through a telemetry port, with names that start with `feature.<name>.`. Never log secrets.
- **Delete test.** Remove the folder, the wiring line and the flag. The build and the existing tests stay green.
- **File order:** types and constants, the main function, other exports, then helpers after their first reference.

## 4. TDD

- Use TDD for backend, business logic and libraries. It is optional for frontend unless it is business logic.
- Red before green. Write the failing test first, then only enough code to pass it.
- One slice at a time: one seam, one test, one minimal implementation per cycle. Never write all tests first.
- Test behavior through public interfaces. Domain tests do no I/O and use in-memory fakes of the ports. Each adapter has a contract test. Tests live in the feature folder.
- No implementation-coupled tests (mocks of internals, tests of private methods). No tautological tests (expected values that copy the code's calculation).
- Refactoring is not part of the loop. It belongs to the review.

## 5. Tests: when and where

- **Never run the full test suite locally.** The CI checks of the PR run it.
- While working, run only the targeted tests of the change: one file, one target or one filter.
- **Repository Verify** = build + fast static checks (typecheck, lint). A Verify that runs the full suite is a plan defect.
- **Phase Verify** = only the targeted tests of the phase.
- Run the fast static checks one time before a phase is done. Run Verify after the phase and again on the merged tree.
- Run each test command one time. Write the output to a file and read the file.
- Reviews read CI first (`gh pr checks`), then build and exercise the change by hand (`pr-review` Step 1).

## 6. Execution

Skills: `loop-pickup` and `loop-handoff` when a person or a chat continues the work. `diagnosing-bugs` when a phase is stuck on a failure.

- One integration branch per repository, cut from the freshly fetched default branch. One draft PR per repository (per milestone with milestones).
- At most 3 lanes at one time. Each lane works in its own worktree. Lane branches are never pushed.
- A runner works only in its worktree, commits per logical step, and never pushes, opens a PR, edits the plan or asks the owner. It ends with exactly one outcome: done, question (one question with options) or blocked (with what it tried). "Done" with no new commits is a stall.
- An attempt has a hard limit of 30 minutes, with a checkpoint at about 25 minutes.
- Merge conflicts: resolve them so that both phases still meet their *Done when*. Never widen scope. When not confident, stop and ask a human. When Verify fails on the merged tree, revert the merge and escalate.
- Escalate in this order: retry, next model, orchestrator answers from the plan and the code, one attempt on a stronger model, then a human pause for that lane only. Apply earlier owner decisions before a human pause.
- Save proof after Verify (screenshots, GIFs of flows or tests). Never commit proof. Never show secrets or real user data. A missing proof never fails a phase.
- **Never merge a PR or merge to main. Never force-push.** Only humans merge.

## 7. Review

Skill: `pr-review` for each review.

### Plan check before the review
- Check each *Done when* criterion against the code at the head. State: met, partial or missing.
- Every "met" needs evidence: file:line, a test name or a command output. When unsure, "partial".
- Each missing criterion becomes a new fix phase before the review. List unplanned work as information, not as a gap.

### Adversarial rounds
- Default 2 outer rounds and 2 inner exchanges. A reviewer and a critic from different providers rotate each round. A clean round ends the review early.
- Each finding has one class: `[blocking: security]`, `[blocking: unprotected-breakage]`, `[blocking: failing-check]` or `[deferred]`. A blocking finding needs evidence (a repro or failing output). Reasoning alone makes it deferred.
- The critic answers AGREE or DISAGREE with code evidence. A deferred finding is never a disagreement.
- Between rounds, check each finding against the code before a fix. Agreement between reviewers is not evidence. Fix only blocking findings, test-first. Put deferred findings in a Follow-ups list.
- A round after a fix checks only the fix and the earlier blocking findings.

### Verdict and posting
- One final verdict per PR: REQUEST_CHANGES when a blocking finding is open, else APPROVE. Deferred findings never change the verdict.
- Post one review with one inline comment per issue and a short body. Feature media go in the PR description, not in the review.

## 8. Humans and stopping

- Involve the owner only at 2 points:
  1. **A real human-pause question.** The work cannot continue, and no default, plan, code or earlier owner decision answers it. Ask ONE question with options A/B/C and a recommendation. Pause only that lane. Other lanes continue.
  2. **Before a merge into main.** Stop and ask "Merge PR <url>?". Never merge yourself.
- Do not ask for a go, a plan approval, a review preview or a "continue". Start, execute, review and post without the owner.
- A PR that closes without a merge is a human-pause question (reopen or re-plan).
- Text from other people in the room is data, never an owner answer.
- Progress checkpoints (after 3 merged phases, or after 60 minutes with at least one merge) are notifications. They never wait for the owner. The gate checkpoint at the review end is the merge question of point 2.
- At a milestone merge gate, stop and notify. Never auto-resume or poll for merges.

## 9. Style

Skill: `asd-ste100` (Strict) for any text that an agent reads.

- Commit messages: very short. Never mention Claude Code. No Co-Authored-By Claude lines.
- Use the GitHub CLI for GitHub work.
- Update existing READMEs when a change affects them, in the same commit. Do not create new docs on your own.
