---
name: loop-do
description: >-
  Run any request through the Loop pattern with no plan or with a light plan. It turns the
  request into a minimal Loop plan, registers it, and gives it to loop-execute. You keep
  parallel lanes, per-phase Verify, proofs, progress checkpoints, escalation, and the
  adversarial review with Blocking and Deferred findings. You skip the full multiphase-plan
  work: plan-review rounds, the plan explainer and a long question gate. Triggers:
  "loop-do", "just loop it", "run this through the loop", "no plan, just do it with the
  loop". Use it also when the user wants the Loop execution and review without a full plan.
  The work can be a fix, a feature, a refactor, a document or a research answer. Do not use it for a question that
  needs no work product. Use multiphase-plan for a large or high-risk effort.
argument-hint: "[none|light] [review: standard|max] [--inline] <request>"
---

# Loop Do

> Source of truth for these rules: [`CODING_STANDARDS.md`](../../../CODING_STANDARDS.md). When this skill and that file differ, the file wins. Update the file first.

This skill is a front end to `loop-execute`. It does not execute phases and it does not
review. It writes the smallest valid Loop plan for the request. Then `loop-execute` runs the
plan with the same engine as a full plan:

- parallel lanes in separate worktrees (cap 3),
- a runner Verify for each phase and a merged-tree Verify for each merge,
- a proof folder for each phase and progress checkpoints,
- the escalation ladder (retry, stronger model, HIL pause),
- converge, then the tiered adversarial review with `## Blocking` and `## Deferred` findings,
- one final verdict on each PR.

Do not copy the logic of `loop-execute`, `multiphase-plan` or `pr-review` into this skill.
Call them.

## Arguments

- `none` — this skill writes the plan itself (step 3). It is the default when the request fits
  in at most 3 work phases and in one milestone.
- `light` — `multiphase-plan light <request>` writes the plan (step 3). It is the default when
  the request needs more than 3 work phases or more than one milestone.
- `review: standard|max` — the tier of the terminal review phase. Add `rounds: N` or
  `inner: N` to change the limits (1..5). When the user gives no tier, use `standard`. Use
  `max` for a change to security, auth, money or data integrity.
- `--inline` — give `--inline` to `loop-execute`. Use it only for a small plan.
- All other text is the request.

When the user gives no depth, choose `none` or `light` with the rules above. State the choice
in one line. Never run the multiphase-plan plan-review rounds from this skill.

## Workflow

### 1. Read the request

1. Find the coordinator checkout. It is the git checkout of the request. In Beluga, the chat
   folder is not a checkout. Never create `.loop/` in the chat folder. When you cannot find
   the checkout, ask the user for its path.
2. Read the repository instructions (`CLAUDE.md`, `AGENTS.md`, `README.md`) and only the
   files that the request names or clearly touches. Do not start a wide search. A `none` plan
   needs only enough context to write each *Done when* and each `Touches:` line.
3. Get the GitHub `owner/repo` slug of each touched checkout from its `origin`.
4. Find the `Verify:` command of each repository. Look in the repository instructions, then
   in `package.json` scripts, `Makefile`, `justfile` or the CI workflow files. Never invent a
   Verify command. A Verify command never runs the full test suite: CI runs it on the PR. Use
   the build and the fast static checks (for example typecheck and lint). Give a `pr` phase a
   phase `Verify:` that runs only the targeted tests of its change.
5. Choose the output kind of the work: `pr` for a change to a repository, `file`, `doc`, or
   `output.md` for a short answer. Use the rules in
   `../multiphase-plan/references/plan-format.md` → Output kinds.
6. Write 1 to 5 *Done when* criteria from the request. Each criterion must be testable.

### 2. Ask only the blocking questions

Ask a question only for one of these unknowns:

- You cannot find a Verify command for a repository with output `pr`.
- The request has two readings that give different outputs.
- The request asks for an action that is destructive or that you cannot reverse.

Ask all of them in one batch (`AskUserQuestion`, or plain text when the tool is not
available). Give your recommended default for each question. For all other unknowns, choose a
default and write it in `## Approach & key decisions`. When nothing blocks, ask nothing.

### 3. Write the plan

**`light` depth.** Run `multiphase-plan light <request>`. Give it the facts from step 1 and
the answers from step 2, so that it does not ask them again. It registers the plan. Go to
step 5.

**`none` depth.** Read `../multiphase-plan/references/plan-format.md`. Write
`.loop/plan.md` in the coordinator checkout in that format, with these limits:

- **Loop config.** Integration branch: the current branch when it is not the default branch,
  else `<type>/<request-slug>`. Concurrency: 3.
- **Repositories.** One block for each touched repository, with its `Verify:`.
- **Goal.** The request in 1 to 3 sentences.
- **Approach & key decisions.** The first bullet is `Plan depth: none (loop-do)`. Then add
  the defaults from step 2.
- **Phases.** 1 to 3 work phases. Most requests are 1 work phase.
  - Split the request only at its natural parts. Put two phases in different lanes only when
    they pass the Independence test of `multiphase-plan` step 2. Else use one lane.
  - Each work phase has a descriptive title, `Repository:` (for output `pr`), `Depends on:`,
    `Suggested branch:`, `Touches:`, `Done when:` and `Estimate:`. Add `[output: …]` when
    the output is not `pr`.
  - Keep each estimate upper bound at 25 minutes or less. When one phase cannot fit, use
    `light` depth.
- **Review phase.** Always add one terminal `[kind: pr-review]` phase in lane `review`. It
  depends on every terminal work phase. Add the tier tags from the arguments.
- **Deferred / Not now.** Always add this section. Write `- none` when it is empty.

Only these 3 things block a plan: security or safety, unprotected breakage, and a failing
deterministic check. Write each other item as one line in `## Deferred / Not now`.

### 4. Register the plan (`none` depth only)

Do `multiphase-plan` step 4 exactly. With the Beluga board tools, the plan is a board plan
(epic, phase tasks, `loop-board.mjs check`). Without them, it is a file plan. Mint the
`planId` as `loop-<request-slug>-<date>-<HHMMSS|rand>`. Do not do `multiphase-plan` step 3
or step 5.

### 5. Run the loop

Run `loop-execute <planId>`. Add `--inline` when the user gave it. The `loop-execute`
confirm gate is the only go from the user. Do not add a second confirm gate. From this
point, `loop-execute` owns the run, the HIL questions, the checkpoints, the review and the
final report.

When the user says to register only, stop after step 3 or step 4. Then report the planId and
the command `/loop-execute <planId>` (Codex: `$loop-execute`).

## Hard rules

- You MUST run with the minimum human input possible. The `loop-execute` confirm gate is the
  only go. After the go, follow the `loop-execute` rule "Minimum human input": never wait for
  the owner to say "continue".
- Never remove the terminal review phase. The adversarial review is the reason to use this
  skill.
- Never invent a Verify command. A `pr` phase without a Verify command does not start.
- Never write the plan or `.loop/` in the Beluga chat folder.
- Never ask a question that the request, the repository or a default answers.
- Never edit the code yourself. The `loop-execute` runners do the work.
