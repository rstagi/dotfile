---
name: loop
description: >-
  Orchestrate a Loop plan on the ws tools. You run as the orchestrator session of a loop session.
  You choose what to run and when, inside the plan dependencies. Tools record the facts: phases,
  attempts, reviews, merges and gates. You write only your judgment (loop.note, loop.decision).
  Use it when `ws loop start <loop dir>` starts an orchestrator, or when asked to "run the loop"
  on a plan with `engine: skills`. Needs a loop session (WS_LOOP_ID), the ws CLI and the ws MCP tools.
---

# Loop

You are the orchestrator of one plan. The plan is `<loop dir>/plan.md`. The loop state is in the loop session.
The loop session has no process. You are a child session of it. You can end and a new orchestrator can continue.
Protocol details: `loop-execute/references/loop-protocol.md`. AR prompts: `loop-execute/references/ar-prompts.md`.
Phase 143 adds the gate explainer, the retrospective and the progress checkpoints.

## Rules

1. Tools write the facts. Write no `loop.phase`, `loop.attempt`, `loop.review`, `loop.merge` or `loop.gate` event.
2. Write `loop.decision` for each departure from a default below (`ws event loop.decision --body '{"topic":"<rule>","phase":<n>,"reason":"<why>"}'`). The reason is required. Use `loop.note` for other judgment.
3. Do not edit code. Runners edit code.
4. Do not ask the human in prose. Ask only through the cases named below, with `ws loop note` and a notification.
5. Use the installed runtime: the skills and Loop scripts in `~/dotfile`. Never run a tool from an integration worktree.

## Start

1. Run `loop-milestone.sh open --dir <loop dir>`. A second call changes nothing. Exit 3 means the loop is parked: end with `ws done --output handoff.md`.
2. Run `ws loop get <loop> --json`. Work on the first milestone that has open phases.
   For a plan after the milestone 6 gate, this is milestone 7.
3. Run the quota preflight: `ws usage --preflight --plan <loop dir>/plan.md --models-conf ~/dotfile/loop-models.conf`. It only reports. When it warns, write `loop.note` and continue.
4. Choose the ready phases: each `Depends on` phase is `done` (output `pr`: `merged`), and the phase belongs to the current milestone. Never start a phase of a later milestone.

## Spawn and wait

- Each spawn uses the `loop` object `{loop, phase, attempt}` and `depends_on` edges for the `Depends on` phases.
- Wait for several sessions with one call: MCP `wait` with `targets`, condition `exit`, and mode `any`. Choose the next action after each exit.
- Each work attempt runs `loop-runner.sh --loop-dir <loop dir> --worktree <wt> --run-dir <dir> --prompt-file <file> --chain task`. The tools record its result.
- The work attempt key is `phase:<n>:attempt:<k>`. The same key returns the first session and starts no second process.
- After an exit, read the result from `ws loop get <loop> --json`. A runner checkpoint is not a result. Start the next attempt with the next `<k>`. Use the escalation ladder of `loop-execute` (retry, stronger model, ask the human) as the default.

### Output `pr`

1. Run `loop-repo.sh lane --repo <slug> --run-id <loop id> --branch <Suggested branch> --from <integration worktree>`. It prints `{worktree, branch, base}`.
2. Call `phase_spawn` with the lane worktree as `repo`, the printed `base`, the `loop` object and the runner command.
3. A retry uses the same worktree. Run `loop-repo.sh lane` again to read the stored base.

### Output `file`, `doc` or `output.md`

Start with the keyed create (`session_create`, `headless`, `key`, `loop` object).
Set `cwd` to `<loop dir>/outputs/p<N>/`. Pass no repository and no base. Create the directory first.
The phase has no branch, no merge and no integration worktree. Its result makes it `done`.

## Merge a lane

A `pr` phase ends with a result `done`. Before each merge, run `ws loop check <loop>` (see Checks).

```sh
loop-merge.sh --worktree <integration worktree> --lane-branch <Suggested branch> --verify-cmd <Verify>
```

Start no other merge while a merge failure is open. Act on the exit code:

- Exit 0: the merge is pushed and recorded. Set `[status: done]` on the phase in `<loop dir>/plan.md`. Run `loop-plan.sh push`. Do this before you spawn a phase that depends on it. Then `loop-pickup --auto` in the next runner sees the dependency as done.
- Exit 2 (conflict): start one merge runner in the integration worktree with a keyed create. Then run `loop-merge.sh --worktree <integration worktree> --finish --verify-cmd <Verify>`. When the runner fails, run `loop-merge.sh --worktree <integration worktree> --abort`. Then ask the human (`ws loop pause`, notification).
- Exit 3 (no lane branch): run `loop-repo.sh lane` again. Start a new attempt of the phase.
- Exit 4 (merged-tree Verify failed): run `loop-merge.sh --worktree <integration worktree> --undo`. Start a new attempt of the phase. A later merge of the fixed lane brings all lane changes.
- Exit 1: ask the human. Do not retry.

## Reviews

- A review phase starts when each work phase that it depends on is `merged` (output `pr`) or `done` (other outputs).
- When each `pr` work phase of a milestone is `merged`, open one PR for each participating repository from its integration branch: `gh pr create --head <integration branch> --base <default branch>`. Do it once. Check `loop-repo.sh pr-state` or `gh pr list` first.
- Run each review with the `loop-review` skill. A `[review: standard|max]` tier and `[rounds: N]` come from the phase line.
- A blocking finding that stays after the last round is a stop. Ask the human.

## Merge gate

After the final reviews of a `[gate: merge]` phase:

1. Run `ws loop check <loop>`. On a hard violation, stop (see Checks).
2. Run `loop-milestone.sh park --dir <loop dir> --milestone <N> --prs '<json slug→PR url>'`.
3. Notify the human with `loop-notify.sh`. Give the PR links and the two resume commands: `loop-milestone.sh resume --dir <loop dir>`, then `ws loop start <loop dir>`.
4. Register no continuation. End with `ws done --output handoff.md`.

## Accept gate

After the final review of a `[gate: accept]` phase:

1. Run `loop-milestone.sh park --kind accept --phase <n> --artifacts '<json list of paths>'`.
2. Notify the human with the artifact paths.
3. Wait for `ws loop accept` or `ws loop reject`. They close the gate. Accept: continue. Reject: read the reason in `ws loop get <loop> --json`, add the fix to the plan and continue.

## Checks

- Run `ws loop check <loop>` before each merge and at each gate.
- A soft violation is a warning. Fix its cause, or write `loop.decision` with the rule as `topic` and the reason.
- A hard violation blocks new spawns. Stop. Notify the human. Wait until the human runs `ws loop ack <loop> --reason <text>`. Then check again.

## Recycle

Recycle yourself before your context fills up, and after each gate that does not park.

1. Write the state you need into `handoff.md` in your run directory: the current milestone, the open phases, the open failures, and the next action.
2. Register the continuation with `ws on me exit --spawn claude "Run the loop skill. Read handoff.md"`. The watched session is yourself. Phase 102 makes the loop session the parent of the new orchestrator.
3. Run `ws done --output handoff.md`.

Never register the exit trigger on the loop session, because that session has no process.
At a merge park, do not recycle: Register no continuation.
