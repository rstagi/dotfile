#!/usr/bin/env bash
# Stop hook: kill watcher, notify "done" with elapsed time
set -euo pipefail

HOOK_INPUT=$(cat)
source "$(cd "$(dirname "$0")" && pwd)/claude-hooks-lib.sh"

SID=$(hook_field "session_id")
[ -z "$SID" ] && exit 0

kill_watcher "$SID"

# Compute elapsed time
elapsed_str=""
STARTFILE="/tmp/claude-watcher-${SID}.start"
if [ -f "$STARTFILE" ]; then
  start_ts=$(cat "$STARTFILE")
  now_ts=$(date +%s)
  elapsed_s=$((now_ts - start_ts))
  mins=$((elapsed_s / 60))
  secs=$((elapsed_s % 60))
  if [ $mins -gt 0 ]; then
    elapsed_str=" (${mins}m ${secs}s)"
  else
    elapsed_str=" (${secs}s)"
  fi
fi

full_summary=$(hook_field "last_assistant_message")
[ -z "$full_summary" ] && full_summary="Task completed"

notify "Claude done!${elapsed_str}" "$full_summary" "$full_summary"

cleanup_watcher "$SID"
