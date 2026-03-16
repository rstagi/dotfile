#!/usr/bin/env bash
# Shared helpers for Claude Code waiting-list hooks

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TELEGRAM_TARGET="${CLAUDE_TELEGRAM_TARGET:-}"
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
  rm -f "/tmp/claude-watcher-${sid}.done"
}

_sanitize_applescript() {
  printf '%s' "$1" | sed 's/["\\]/\\&/g'
}

notify() {
  local title="$1" body="$2" telegram_body="${3:-$2}"
  local short_body="${body:0:200}"
  local safe_title safe_body
  safe_title=$(_sanitize_applescript "$title")
  safe_body=$(_sanitize_applescript "$short_body")
  osascript -e "display notification \"$safe_body\" with title \"$safe_title\"" 2>/dev/null &
  if [ -n "$TELEGRAM_TARGET" ]; then
    openclaw message send --channel telegram --target "$TELEGRAM_TARGET" --message "$title: $telegram_body" 2>/dev/null &
  fi
  wait
}
