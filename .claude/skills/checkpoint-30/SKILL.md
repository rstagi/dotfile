---
name: checkpoint-30
description: Keep one agent task to 30-minute work blocks with a status report and a request for guidance before continuing. Use when the user invokes checkpoint-30 for interactive Claude or Codex work.
disable-model-invocation: true
---

# 30-minute checkpoint

Apply this to the current task only.

## Interactive task

1. Record the current wall-clock time when starting work and after each user reply. The deadline is 30 minutes later. Subtasks, tool calls, and retries do not reset it.
2. Check elapsed time between tool calls. Around 25 minutes, finish a safe step and prepare a short status. Avoid starting a command that could run past the deadline without a way to poll or stop it.
3. By the deadline, stop working and send the user: what is done, what remains, the current obstacle or decision, and the next action you recommend. Ask one concrete question when guidance would help; otherwise ask whether to continue the proposed next block. Wait for the user's reply before resuming this task. A completed task needs only its normal final report.
4. If an unexpected long tool call crosses the deadline, report immediately when control returns. Never reset the clock merely by sending a progress update.

This is a model instruction, not a wall-clock interrupt. An agent may miss a check while inside a long tool call or generation. Do not claim it is a hard guarantee.
