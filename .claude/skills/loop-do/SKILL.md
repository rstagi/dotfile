---
name: loop-do
description: >-
  Do a request from start to end with a short plan in chat. The steps are: read, ask, plan,
  build, check the plan, open one draft PR with proof, review and report. It uses no scripts, no plan files and
  no board. It works in Claude Code and in Codex. Triggers: "loop-do", "just loop it", "run this
  through the loop", "no plan, just do it with the loop". The work can be a fix, a feature, a
  refactor or a document. Do not use it for a question that needs no work product.
argument-hint: "<request>"
---

# Loop Do

> Source of truth for these rules: [`CODING_STANDARDS.md`](../../../CODING_STANDARDS.md). When this skill and that file differ, the file wins. Update the file first.

Do the work yourself, in this chat. Do not call scripts. Do not send notifications. Do not use the Beluga board. Do not write plan or state files.

## 1. Read

1. Read the request.
2. Read the repository instructions: `CLAUDE.md`, `AGENTS.md` and `README.md`.
3. Read only the files that the request names or touches. Do not start a wide search.
4. Find the Verify command. Look in the repository instructions, then in the `package.json` scripts, the `Makefile`, the `justfile` or the CI workflow files.
   - Verify is the build and the fast static checks, for example typecheck and lint.
   - Verify never runs the full test suite. CI runs the full suite on the PR.
   - Never invent a Verify command.

## 2. Ask

Ask a question only when one of these unknowns blocks the work:

- You cannot find a Verify command.
- The request has two meanings that give different results.
- The request asks for an action that is destructive or that you cannot reverse.

Ask all the questions in one batch. Use `AskUserQuestion` when it is available, else plain text. Give a recommended default for each question. For each other unknown, choose a default and write it in the plan. When nothing blocks, ask nothing.

## 3. Plan

Write a short plan in chat. Do not ask the user to approve it. The plan has:

- **Goal.** The request in 1 to 3 sentences.
- **Done when.** 1 to 5 criteria. Each criterion must be testable.
- **Phases.** 1 to 3 phases. Give each phase an imperative title and the files that it touches.
- **Independent.** Mark two phases as independent only when they have no dependency and touch different files.
- **Defaults.** The defaults that you chose in step 2.

## 4. Build

1. Make one branch for the work: `<type>/<outcome-slug>`. Do not work on the default branch.
2. Do each phase. Follow the `engineering-standards` skill. Use the `tdd` skill for backend, business logic and libraries.
3. Run independent phases in parallel only in Claude Code:
   - Give each independent phase to one subagent in its own worktree.
   - Run at most 3 subagents at one time.
   - Merge each subagent branch into the work branch.
4. In Codex, or when one phase is enough, do the phases in sequence.
5. Run only the targeted tests of each change. Never run the full test suite.
6. Run the Verify command after each phase. When Verify fails, fix the failure before the next phase.
7. Commit after each phase. Use a short commit message.

## 5. Check the plan

Check each *Done when* criterion against the code at the head of the branch. Give each criterion one state:

- **met**: give evidence. Evidence is a file and a line, a test name or a command output.
- **partial**: tell what is missing.
- **missing**: tell what is missing.

When you are not sure, use **partial**. Fix each partial or missing criterion. Then check the criteria again.

## 6. Open the PR

1. Make proof of work. Proof is a screenshot, a screen recording or a GIF of a real run of the changed behavior. A run of the tests alone is not proof.
2. Never show secrets or real user data in the proof.
3. Push the branch.
4. Open one draft PR with `gh pr create --draft`.
5. Put the proof in one `## Proof` section of the PR description.

## 7. Review

1. Run the `pr-review` skill on the PR.
2. Fix the findings that block. Push the fixes.
3. Do at most 2 review rounds. A clean round stops the review.
4. Never post a review before a human approves it in chat.
5. Never run `gh pr ready`. Only a human marks a PR ready.
6. Never merge the PR.

## 8. Report

Give a short report in chat:

- the PR link,
- the state of each *Done when* criterion,
- the deferred findings, each in one line.

## Hard rules

- Never invent a Verify command.
- Never ask a question that the request, the code or a default answers.
- Never run the full test suite.
- Never force-push. Never merge.
- Each PR opens as a draft. Only a human marks it ready.
