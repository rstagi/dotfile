# Skills engine e2e results

## Fake orchestrator (`zsh tests/loop-e2e/run.sh --all`, also run by `zsh tests/run.sh`)

2026-10-26: happy, verify-fail, crash and broken pass (28 assertions).

## Real orchestrator model (`--scenario happy --model claude-opus-5-5`)

2026-10-26: **blocked.** The model cannot authenticate in the sandbox.

- Command: `zsh tests/loop-e2e/run.sh --scenario happy --model claude-opus-5-5`
- Error, exact: `Not logged in · Please run /login`
- Cause: the Claude login is in the macOS Keychain only. `~/.claude/.credentials.json` does not exist,
  so there is no credential file to copy into the sandbox HOME. A temporary HOME cannot read the Keychain entry.
- Not done: the harness did not read the Keychain secret. The run needs an approved credential for the sandbox
  (for example a token from `claude setup-token` in the environment of the harness).
- Not done: fake engines for the AR review roles behind `loop-runner.sh` (verdict token, publish output, status for
  `loop-review.sh record`). The `--model` path starts the `loop` skill and the phase runners are not yet faked there.
  This part needs a working model to check it.
- Result: no pass or fail for the `loop` skill. Phase 100 cannot use this run yet.
