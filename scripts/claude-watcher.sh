#!/usr/bin/env bash
# Background watcher: after 10s of Claude processing, remind to check waiting list
set -euo pipefail

HOOK_INPUT=$(cat)
source "$(cd "$(dirname "$0")" && pwd)/claude-hooks-lib.sh"

SID=$(hook_field "session_id")
[ -z "$SID" ] && exit 0

DONEFILE="/tmp/claude-watcher-${SID}.done"
rm -f "$DONEFILE"

# Background: poll every 1s, notify after 60s if not done
(
  elapsed=0
  while [ $elapsed -lt 60 ]; do
    sleep 1
    elapsed=$((elapsed + 1))
    [ -f "$DONEFILE" ] && exit 0
  done

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
