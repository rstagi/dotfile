#!/usr/bin/env bash
# Background watcher: after timeout, remind to check waiting list periodically
set -euo pipefail

HOOK_INPUT=$(cat)
source "$(cd "$(dirname "$0")" && pwd)/claude-hooks-lib.sh"

SID=$(hook_field "session_id")
[ -z "$SID" ] && exit 0

DONEFILE="/tmp/claude-watcher-${SID}.done"
STARTFILE="/tmp/claude-watcher-${SID}.start"
rm -f "$DONEFILE"
date +%s > "$STARTFILE"

TIMEOUT="${CLAUDE_WATCHER_TIMEOUT:-60}"
INTERVAL="${CLAUDE_WATCHER_INTERVAL:-300}"

get_tasks() {
  [ -f "$WAITING_TASKS_FILE" ] || return 0
  grep -E '^\s*- \[ \]' "$WAITING_TASKS_FILE" | sed 's/^[[:space:]]*- \[ \] /• /' || true
}

# Background: initial wait, then recurring notifications
(
  # Initial timeout
  elapsed=0
  while [ $elapsed -lt "$TIMEOUT" ]; do
    sleep 1
    elapsed=$((elapsed + 1))
    [ -f "$DONEFILE" ] && exit 0
  done

  task_index=0
  while true; do
    all_tasks=$(get_tasks)
    if [ -n "$all_tasks" ]; then
      count=$(echo "$all_tasks" | wc -l | tr -d ' ')
      task=$(echo "$all_tasks" | sed -n "$((task_index % count + 1))p")
      notify "Claude is thinking..." "$task"
      task_index=$((task_index + 1))
    else
      notify "Claude is thinking..." "Check your waiting list!"
    fi

    # Wait for next interval
    waited=0
    while [ $waited -lt "$INTERVAL" ]; do
      sleep 1
      waited=$((waited + 1))
      [ -f "$DONEFILE" ] && exit 0
    done
  done
) &

exit 0
