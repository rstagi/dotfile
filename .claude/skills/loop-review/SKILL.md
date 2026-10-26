---
name: loop-review
description: >-
  Review a PR, a plan, a document or a file with the adversarial review (AR) protocol on the ws tools.
  A reviewer, a critic and a responder run as keyed headless ws sessions through loop-runner.sh.
  wsd records each step as a loop.review event. A fixer and one final review follow when a blocking finding stays.
  Use it for requests such as "review this PR with AR" or "run the review phase". The loop skill calls it for review phases.
  Requires a loop session (WS_LOOP_ID or a loop id), the ws CLI and the MCP tools of ws.
---

# Loop Review

Use this skill to review one target. The target is a PR, a plan, a document or a file.
The protocol is the AR protocol of `loop-execute` (`references/loop-protocol.md`, `references/ar-prompts.md`).
`loop-ar.sh` supplies the verdict, the prompt, the rotation and the disputed sidecar.
`step.sh` (in this skill directory) supplies the attempt number and the keyed create.

## Rules

1. You are the coordinator. You do not review, fix or post in your own session.
2. Each step is one fresh headless runner: `loop-runner.sh --publish-output <file> --review-key <step key>`.
3. You write no `loop.review` event. `wsd` records `started` and `exited` from the keyed create. `loop-runner.sh` and `loop-ar.sh` record the result. A result is a `loop.review {state: finished}` record.
4. A step stays pending until its result exists. After a result, start no new attempt for that step.
5. Freeze the head before round 1 (`git rev-parse HEAD` or the PR head SHA). Pass the same head to every step.
6. Write `loop.decision` for each departure from a default below. The reason is required. Without it, `ws loop check` reports the departure.

## Defaults

- Rotation: `source loop-ar.sh; ar_rotation <round>` gives reviewer R, critic C and the unused provider (GPT, Claude, Grok). There is no confirmation by a third provider.
- `[rounds: N]` is a maximum. The default is 2. `[inner: N]` defaults to 2. Both accept 1 to 5.
- A round where R and C agree that no blocking finding is open goes straight to the final review.
- Only these findings block: `security`, `unprotected-breakage`, `failing-check`. Each needs evidence. All other findings are deferred. The fixer fixes only blocking findings.
- Each step is a fresh call. The prompt inlines the prior exchange (`loop-ar.sh prompt`).
- At the inner cap without agreement, write the sidecar (`loop-ar.sh disputed --ledger <ledger> --review-key <step key>`). A disputed round is never clean.
- One final review posts the GitHub review. It uses an idempotency marker. It posts `REQUEST_CHANGES` for an open blocking finding and `APPROVE` otherwise. On a PR of the same author as the authenticated user, it posts `COMMENT` with a `Verdict:` line.

## Step key

`review:<phase>:<repo or none>:<role>:<round>:<inner>:attempt:<k>`

Roles: `review`, `respond`, `critic`, `fix`, `final`, `disputed`. Use `none` for a target with no repository.
Never build the key by hand. Run `step.sh state` and `step.sh run`.

## Run one step

1. Write the prompt file with `loop-ar.sh prompt --role <role> --framing <file> --ledger <ledger> --run-dir <dir>`.
2. Check the step:

   ```sh
   step.sh state <loop> <phase> <repo|none> <role> <round> <inner>
   ```

   - `terminal<TAB><verdict>`: the step has a result. Use the verdict. Start nothing.
   - `run<TAB><step key>`: the step needs a process. Go to step 3.
3. Start it:

   ```sh
   step.sh run <loop> <phase> <repo|none> <role> <round> <inner> <provider> -- \
     --worktree <wt> --run-dir <dir> --prompt-file <file> --chain review-<provider> \
     --review-tier <standard|max> --publish-output <dir>/report.md
   ```

   Use `--chain review-fix` for a fix and `--chain review-final` for the final review.
   `step.sh run` sends the keyed create. The same key returns the first session. No second process starts.
4. Wait: `ws wait <session> --for exit` (MCP: `wait`). Read the report with the MCP tool `session_output`.
5. Run `step.sh state` again.
   - `terminal`: take the verdict. Append the ledger row. Go to the next step.
   - `run` with the same key: the attempt has no exit record. Wait again.
   - `run` with a higher attempt: the attempt exited with no result. This covers a reviewer that exits with no report, a runner checkpoint and a crash. `step.sh run` starts one new process with the next attempt number.
6. Get the verdict of a reviewer or critic with `loop-ar.sh verdict <report> reviewer|critic`. The runner uses the same rule for the result record.

A final review that exits 10 with `COMMENT` is a result. Do not start a second final review.
A fix result is `PUSHED` or `NOOP` (`loop-ar.sh prove-fix`). A runner checkpoint is not a result.

## Targets

- PR: the framing file names the PR URL and the frozen head. R and C read the diff in a worktree at that head.
- Plan, document or file: the framing file names the absolute path and its SHA-256. The worktree is read-only. Phases with no PR use no fixer and no final GitHub review. The final review report is the result, and the verdict is `APPROVE` or `COMMENT`.

## Recovery

1. Read the loop log: `ws log <loop> --type loop.review` (MCP: `session_log`).
2. For each step in protocol order, run `step.sh state`. Skip each `terminal` step.
3. At the first `run` step, continue with "Run one step". Rebuild the ledger rows from the terminal records when the ledger file is missing.
4. The loop log is the source of truth. The ledger is a replay aid for `loop-ar.sh`.

## Finish

Report the final verdict and the review URL to the caller. Then, when you run as a session of your own, run `ws done --output <file>`.
