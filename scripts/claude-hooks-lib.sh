#!/usr/bin/env bash
# Shared helpers for Claude Code waiting-list hooks

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHANNELS_DIR="${CLAUDE_HOOK_CHANNELS_DIR:-$SCRIPT_DIR/channels}"
WAITING_TASKS_FILE="$HOME/.openclaw/workspace/waiting-tasks.md"

hook_field() {
  echo "$HOOK_INPUT" | jq -r ".$1 // empty"
}

kill_watcher() {
  local sid="$1"
  touch "/tmp/claude-watcher-${sid}.done"
}

cleanup_watcher() {
  local sid="$1"
  rm -f "/tmp/claude-watcher-${sid}.done" "/tmp/claude-watcher-${sid}.start"
}

alert_sound() {
  [ "${CLAUDE_HOOK_SOUND:-0}" = "1" ] || return 0
  afplay /System/Library/Sounds/Funk.aiff 2>/dev/null &
}

notify() {
  local title="$1" body="$2" telegram_body="${3:-$2}"

  if [ "${CLAUDE_HOOKS_DRY_RUN:-0}" = "1" ]; then
    echo "[dry-run] notify: $title | $body"
    return 0
  fi

  if [ -d "$CHANNELS_DIR" ]; then
    for ch in "$CHANNELS_DIR"/*.sh; do
      [ -x "$ch" ] && bash "$ch" "$title" "$body" "$telegram_body" &
    done
    wait
  fi
}
