---
name: checkpoint-30
description: Keep one agent task to 30-minute work blocks with a status report and a request for guidance before continuing. Use when the user invokes checkpoint-30 for interactive Claude or Codex work; Loop phase runners follow the same checkpoint through loop-runner.sh.
---

# 30-minute checkpoint

Apply this to the current task only. A Loop run gets a separate clock for each phase attempt; the overall plan and other lanes keep moving.

## Interactive task

1. Record the current wall-clock time when starting work and after each user reply. The deadline is 30 minutes later. Subtasks, tool calls, and retries do not reset it.
2. Check elapsed time between tool calls. Around 25 minutes, finish a safe step and prepare a short status. Avoid starting a command that could run past the deadline without a way to poll or stop it.
3. By the deadline, stop working and send the user: what is done, what remains, the current obstacle or decision, and the next action you recommend. Ask one concrete question when guidance would help; otherwise ask whether to continue the proposed next block. Wait for the user's reply before resuming this task. A completed task needs only its normal final report.
4. If an unexpected long tool call crosses the deadline, report immediately when control returns. Never reset the clock merely by sending a progress update.

This is a model instruction, not a wall-clock interrupt. An agent may miss a check while inside a long tool call or generation. Do not claim it is a hard guarantee.

## Loop phase runner

The orchestrator puts a checkpoint deadline in each phase prompt. Before that deadline, finish a safe step and write `$RUN_DIR/status.json` with `outcome: "question"`, a concise `summary`, and one concrete `question` for the orchestrator. End the attempt. The orchestrator answers and resumes the phase in a fresh attempt directory; it does not wait for the user or stop other lanes. If the phase is done first, use the normal `done` handoff.

`loop-runner.sh` enforces a 30-minute task-attempt cutoff and writes a fallback checkpoint when the agent misses the deadline. A fallback report reflects only observable worktree state; it cannot replace the agent's own account of its reasoning.
