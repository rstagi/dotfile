# Skills engine e2e results

## Fake orchestrator (`zsh tests/loop-e2e/run.sh --all`, also run by `zsh tests/run.sh`)

2026-10-26: happy, verify-fail, crash and broken pass (28 assertions).

## Real orchestrator model (`--scenario happy --model claude-opus-5-5`)

Last run, 2026-10-26: **PASS.** 9 of 9 assertions passed. Duration: 239 s.

`ws loop check`: 0 hard violations, 0 soft violations (no `source: agent` violation, no `loop.decision` departure).

Command: `zsh tests/loop-e2e/run.sh --scenario happy --model claude-opus-5-5`

What the `loop` skill did on the model:
- Opened milestone 1 from `.loop/plan.md` only. Ran phases 1 and 2 in parallel and phase 3 after phase 1, each in
  its own lane worktree on its Suggested branch. Each passed on its first attempt and merged through `loop-merge.sh`.
- Opened the milestone PR with the fake `gh`.
- Ran the review phase: gpt reviewer `APPROVE`, claude critic `AGREE`, claude final `APPROVE`, each as a keyed
  step with a result record.
- Parked with `loop-milestone.sh park`: status `awaiting-merge`, notification sent, `ws done --output handoff.md`.
- Two notes of the model: `~/dotfile` does not exist under the sandbox HOME, so it ran the runtime from the worktree
  path in its prompt; the quota preflight had no quota data and it logged a `loop.note`.
- The milestone explainer runner (a fake engine with no job for it) wrote no props, narration or retrospective. The
  skill records `error.txt` and `retro-error.txt` and parks anyway.

Setup in the `--model` sandbox only: the sandbox HOME links `$HOME/Library/Keychains` (removed in teardown; the
harness never reads the Keychain secret), `LOOP_WS_SESSIONS=1`, the fake engines in `tests/loop-e2e/bin/`
(`engine.sh` for work phases and for the review, critic and final roles; the engine reads its job from the argv of
its runner session), a fake `gh`, and wrappers of the Loop scripts on PATH.

### Loop bugs found by the first real-model run (2026-10-26, run failed at the review)

1. FIXED. The merge command in `.claude/skills/loop/SKILL.md` had no `--phase`, `--repository` and `--run-id`, so
   `loop-merge.sh` recorded no `loop.merge` and the phase stayed `done`. The skill now passes the three flags to the
   merge, `--finish`, `--abort` and `--undo`. `tests/loop-skill.test.sh` checks the command.
2. FIXED. `.claude/skills/loop-review/step.sh` read `ws log` as a bare array, but `ws log` prints `{"entries":[...]}`.
   It now accepts both forms. `tests/loop-review-skill.test.sh` covers the object form.

No open Loop bug. The first run also failed for a harness reason (the fake engine could not find its run directory
under a ws session), fixed in `tests/loop-e2e/bin/engine.sh`.
