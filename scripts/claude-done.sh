#!/usr/bin/env bash
# Stop hook: kill watcher, notify "done" with git diff summary
set -euo pipefail

HOOK_INPUT=$(cat)
source "$(cd "$(dirname "$0")" && pwd)/claude-hooks-lib.sh"

SID=$(hook_field "session_id")
[ -z "$SID" ] && exit 0

kill_watcher "$SID"

full_summary=$(hook_field "last_assistant_message")
[ -z "$full_summary" ] && full_summary="Task completed"

notify "Claude done!" "$full_summary" "$full_summary"

cleanup_watcher "$SID"
