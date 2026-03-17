#!/usr/bin/env bash
# Notification hook: kill watcher, notify "needs input" with sound alert
set -euo pipefail

HOOK_INPUT=$(cat)
source "$(cd "$(dirname "$0")" && pwd)/claude-hooks-lib.sh"

SID=$(hook_field "session_id")
[ -z "$SID" ] && exit 0

kill_watcher "$SID"

ntype=$(hook_field "notification_type")
case "$ntype" in
  permission_prompt) msg="Needs permission approval" ;;
  idle_prompt)       msg="Waiting for your input" ;;
  *)                 msg="Needs your attention" ;;
esac

alert_sound
notify "Claude blocked" "$msg"

cleanup_watcher "$SID"
