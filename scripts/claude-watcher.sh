#!/usr/bin/env bash
# Background watcher: after 10s of Claude processing, remind to check waiting list
set -euo pipefail

HOOK_INPUT=$(cat)
source "$(cd "$(dirname "$0")" && pwd)/claude-hooks-lib.sh"

SID=$(hook_field "session_id")
[ -z "$SID" ] && exit 0

PIDFILE="/tmp/claude-watcher-${SID}.pid"
DONEFILE="/tmp/claude-watcher-${SID}.done"

# Kill previous watcher if still running
if [ -f "$PIDFILE" ]; then
  kill "$(cat "$PIDFILE")" 2>/dev/null || true
  rm -f "$PIDFILE"
fi
rm -f "$DONEFILE"

# Background: sleep then notify
(
  echo $$ > "$PIDFILE"
  trap 'rm -f "$PIDFILE"' EXIT

  sleep 30

  [ -f "$DONEFILE" ] && exit 0

  tasks=""
  if [ -f "$WAITING_TASKS_FILE" ]; then
    tasks=$(grep -E '^\s*- \[ \]' "$WAITING_TASKS_FILE" | head -3 | sed 's/^[[:space:]]*- \[ \] /• /' || true)
  fi

  if [ -n "$tasks" ]; then
    notify "Claude is thinking..." "$tasks"
  else
    notify "Claude is thinking..." "Check your waiting list!"
  fi
) &

exit 0
