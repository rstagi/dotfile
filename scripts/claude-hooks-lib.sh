#!/usr/bin/env bash
# Shared helpers for Claude Code waiting-list hooks

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TELEGRAM_TARGET="283985425"
WAITING_TASKS_FILE="$HOME/.openclaw/workspace/waiting-tasks.md"

hook_field() {
  echo "$HOOK_INPUT" | jq -r ".$1 // empty"
}

kill_watcher() {
  local sid="$1"
  local pidfile="/tmp/claude-watcher-${sid}.pid"
  local donefile="/tmp/claude-watcher-${sid}.done"
  touch "$donefile"
  if [ -f "$pidfile" ]; then
    local pid
    pid=$(cat "$pidfile")
    kill "$pid" 2>/dev/null || true
    rm -f "$pidfile"
  fi
}

cleanup_watcher() {
  local sid="$1"
  rm -f "/tmp/claude-watcher-${sid}.pid" "/tmp/claude-watcher-${sid}.done"
}

notify() {
  local title="$1" body="$2" telegram_body="${3:-$2}"
  local short_body="${body:0:200}"
  osascript -e "display notification \"$short_body\" with title \"$title\"" 2>/dev/null &
  openclaw message send --channel telegram --target "$TELEGRAM_TARGET" --message "$title: $telegram_body" 2>/dev/null &
  wait
}
