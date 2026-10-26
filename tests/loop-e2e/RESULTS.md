# Skills engine e2e results

## Fake orchestrator (`zsh tests/loop-e2e/run.sh --all`, also run by `zsh tests/run.sh`)

2026-10-26: happy, verify-fail, crash and broken pass (28 assertions).

## Real orchestrator model (`--scenario happy --model claude-opus-5-5`)

2026-10-26: **FAIL at the AR review.** The loop did not reach the merge gate. Duration: 442 s.

Command: `zsh tests/loop-e2e/run.sh --scenario happy --model claude-opus-5-5` (one counted run; earlier runs
stopped on harness setup errors that are fixed).

Authentication works: the sandbox HOME links `$HOME/Library/Keychains` (the `--model` path only, removed in teardown).
The harness never reads the Keychain secret.

What the model did (the `loop` skill, with the ws MCP tools):
- Opened milestone 1 (`loop-milestone.sh open`) and ran phases 1 and 2 in parallel, then phase 3. Each ran on its
  Suggested branch in its own lane worktree and passed on its first attempt. Phases 2 and 3 were merged and pushed
  through `loop-merge.sh`. It opened the milestone PR with the fake `gh`.
- Started the review phase. The round 1 reviewer produced `APPROVE`, but no result record was written. It stopped
  at phase 4 by the skill's stop path: `ws loop pause`, a note, a notification, `handoff.md`, `ws done`.

`ws loop check` at the stop: 0 hard and 0 soft violations. The assertions of the run: parked at the gate FAIL,
every work phase merged FAIL (phase 1 shows `done`, not `merged`, see finding 1), state `awaitingMerge` FAIL,
lane worktrees pass, integration HEAD only through merges pass, no hard violation pass, no soft violation pass.

### Cause of the review failure (harness)

`loop-runner.sh --review-key` records the review result only after `--publish-output`, and that needs a runner under a
ws session (`LOOP_WS_SESSIONS` not 0). The sandbox keeps `LOOP_WS_SESSIONS=0` from `tests/lib.sh`.
- Attempt 1 (sessions off): the fake engine wrote the report, then the runner exited 50
  ("cannot publish output without a runner ws session").
- Attempt 2 (the model turned sessions on for this runner): `tests/loop-e2e/bin/engine.sh` finds its run directory,
  review key and report path from the argv of its parent runner. With sessions on, the engine is a child of `wsd`, so
  it finds none and the chain ends with exit 40.

Fix for the next attempt: set `LOOP_WS_SESSIONS=1` in the `--model` sandbox and make `engine.sh` read its job without
the process ancestry. The run directory is the directory of the open stdout file (`lsof -a -p $$ -d 1 -Fn`). The review
key and the report path can come from the keyed runner session in `ws session get --json`.

### Loop findings from the run (not fixed here)

1. The merge command in `.claude/skills/loop/SKILL.md` ("Merge a lane") has no `--run-id`, `--phase` or `--repository`
   flags. `loop-merge.sh` records `loop.merge` only with `--phase` and `--repository`, so phase 1 merged in git but stayed
   `done`, not `merged`. A second merge with the flags found it merged and recorded nothing.
2. `.claude/skills/loop-review/step.sh` reads `ws log <loop> --type loop.review` as a bare JSON array, but `ws log`
   prints `{"entries":[...]}`. The model worked around it with a `ws` wrapper on PATH.
3. The PR body of the model had an empty Verify command (a quoting slip of the model, not a tool bug).
