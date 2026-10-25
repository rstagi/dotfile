---
name: converge
description: >-
  Check each *Done when* criterion of a plan against the code at one head, and report the
  gaps. Each criterion gets one state: met, partial or missing, with evidence from the code,
  the tests or a command. Read-only: it does not edit code, the plan or the state.
  loop-execute runs it one time per review phase, before the reviewers. Use it also by hand
  before you open or merge a PR. Triggers: requests to "converge", "check the plan against the
  code", "did we build what we planned" or "find spec gaps".
argument-hint: "[<plan file>] [--review-phase N] [--worktree <path>]"
---

# Converge

A plan can pass every `Verify:` command and still not do what its *Done when* lines say. A
test can be missing. A runner can stop early or build a different thing. This skill finds
that gap. It compares each *Done when* criterion with the code at one head. It does not judge
code quality. The PR review does that.

## Inputs

- **Plan.** The default is `.loop/plan.md`. A PR body with acceptance criteria is also a
  plan. Use `gh pr view <n> --json body` to read it.
- **Scope.** With `--review-phase N`, the scope is each work phase in the `Depends on`
  closure of review phase N. Only the phases of the current milestone are in the scope.
  Without it, the scope is each work phase that is not an `[archived]` stub.
- **Code.** One worktree for each repository of the scope. The default is the current
  directory. loop-execute gives the integration worktrees and their heads.

## Rules

- Do not edit code, the plan or the state. Do not commit, push or post to GitHub.
- Split a *Done when* line into its separate criteria. One criterion is one claim that a
  person can check.
- Give each criterion exactly one state: `met`, `partial` or `missing`.
- Every `met` needs evidence. Evidence is a `file:line`, a test name that asserts the claim,
  or a command and its output. A file name alone is not evidence.
- A `partial` names the part that is missing.
- A `missing` names what a fix phase must build. Write it as one imperative outcome.
- When you are not sure, the state is `partial`, not `met`.
- Run a command only when it is read-only and finishes in less than 5 minutes. You can run the
  repository `Verify:` command.
- Do not report style, names or refactors. Do not report a criterion that the code meets in
  a different way than the plan suggests. Write that as a deviation.

## Workflow

1. Read the plan. List the phases in the scope. For each phase, read `Repository:`,
   `Touches:` and *Done when*.
2. Split each *Done when* into criteria. Number them `P<phase>.<k>`, for example `P3.2`.
3. For each criterion, read the code at the head. Find the evidence. Give the state.
4. Find unplanned work. This is a changed behavior that no criterion explains. Use
   `git diff --stat <base>...HEAD` for the list of changed files. Write each item as one line.
   Unplanned work is information for the reviewers. It is not a gap.
5. Write the report. Then write the verdict as the last line.

## Report

Write the report to `.loop/converge/p<N>.md`, where N is the review phase. Without
`--review-phase`, write it to `.loop/converge/manual.md`. Use this layout:

```markdown
# Converge: review phase <N>

Heads: <owner/repo>=<sha>, ...

| Criterion | Phase | State | Evidence or gap |
|-----------|-------|-------|-----------------|
| P3.1 <criterion text> | 3 | met | `src/auth/refresh.ts:42`, test `refreshes an expired token` |
| P3.2 <criterion text> | 3 | missing | Gap: add the 401 retry to the API client |

## Gaps

- P3.2 · Repository: <owner/repo> · Outcome: <imperative outcome> · Done when: <criterion>

## Partial

- P4.1 · <the part that is missing>

## Deviations

- P5.2 · <how the code meets the criterion in a different way>

## Unplanned work

- <file or behavior> · <one line>

Verdict: converged|gaps
```

Write `- none` under a section with no items. The last line is `Verdict: converged` when no
criterion is `missing`. Otherwise it is `Verdict: gaps`. A `partial` does not change the
verdict.

## Manual use

Without loop-execute, show the table and the gaps to the user. Then stop. Do not fix the
gaps. The user decides. Offer two next steps:

- Insert the gaps as phases with `loop-handoff`, or as tasks in the current branch.
- Accept a gap and add it to `## Deferred / Not now`.
