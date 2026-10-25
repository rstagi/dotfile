---
name: assess
description: >-
  Decide if an idea deserves a plan, before you write one. Collect evidence about the problem,
  the alternatives and the cost, shape the smallest useful version, and give one decision:
  go, clarify or kill. A go decision writes a handoff that multiphase-plan reads as its goal.
  Triggers: requests to "assess this idea", "is this worth building", "should we do X", "go or
  kill". Use it also when a goal for multiphase-plan is still an idea and not a defined effort. Do not
  use it for a defined task, a bug fix or a small change.
argument-hint: "<idea> [slug=<slug>]"
---

# Assess

A plan costs time to write, review and execute. Some ideas do not deserve that cost. This
skill finds that out first. It writes one decision file. It does not write code or a plan.

## Rules

- Do not write code or a plan. Stop at the decision.
- Every claim cites its source: a `file:line`, a URL, a command and its output, or a user
  answer. Mark a claim without a source as `assumption`.
- Search for a cheaper path first: an existing feature, a configuration, a library, a
  manual step or doing nothing. An idea with a cheaper path that is good enough is a `kill`.
- Give exactly one decision: `go`, `clarify` or `kill`.
- A `kill` with a clear reason is a good result. Do not change the evidence to get a `go`.
- Ask the user questions only in step 1, in one batch. A question that opens later goes into
  the `clarify` list.

## Workflow

1. **Intake.** Write the idea in one sentence. Make the slug from it when the argument has no
   `slug=`. Read the code that the idea touches. Then ask the user, in one batch, only what
   the code and the request do not answer:
   - Which problem does it solve, and for whom?
   - What does the user do today without it?
   - Why now? What happens if we do not do it?
   - How will we know that it worked?
   Give your recommended answer for each question.
2. **Research.** Collect evidence for and against the idea:
   - The code: what exists, what is near it, which seams it needs.
   - The data, when the project has analytics or logs: how often the problem occurs.
   - The web: prior art, libraries, known failures of the same approach. Use the web search
     tool. Keep each finding to one line with its URL.
3. **Define.** Write the problem, the user, the success measure and the non-goals. Write the
   success measure as a thing a person can check.
4. **Shape.** Describe the smallest version that tests the idea. Give its rough size in
   phases of 30 minutes or less, its main risks and its kill criteria. A kill criterion is a
   result that stops the work after it starts.
5. **Decide.** Apply these tests in this order:
   1. The problem has evidence, not only assumptions. If not: `clarify`, or `kill` when the
      evidence says the problem is rare or absent.
   2. No cheaper path is good enough. If one is: `kill`, and name the cheaper path.
   3. The smallest version is clear and has a success measure. If not: `clarify`.
   4. All tests pass: `go`.
6. **Write** the decision file. Show the decision and the file path to the user.

## Decision file

Write `.loop/assessments/<slug>.md`. When the file exists, update it. Do not run the whole
workflow again. Read the existing file and only fill the open items. Use this layout:

```markdown
# Assess: <idea in one sentence>

Decision: go|clarify|kill
Date: <YYYY-MM-DD>

## Problem
<problem, user, today's workaround, why now — each with its source>

## Evidence
- <claim> · <source>

## Alternatives
- <cheaper path> · <why it is or is not good enough>

## Smallest version
<scope, rough size in phases, success measure, non-goals>

## Risks and kill criteria
- <risk or kill criterion>

## Open questions
- <question that blocks a go> (clarify only, else `- none`)

## Handoff
<go only: the goal for multiphase-plan in 3 to 6 lines — outcome, scope, non-goals, success
measure. Else `- none`.>
```

## Next step

- `go`: tell the user to run `/multiphase-plan` with the decision file. It reads the
  `## Handoff` section as the goal.
- `clarify`: show the open questions. When the user answers, run `assess` again with the
  same slug.
- `kill`: show the reason and the cheaper path, if one exists. Stop.
