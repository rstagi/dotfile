#!/usr/bin/env bash
# Health check: simulate hook events in dry-run mode
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
export CLAUDE_HOOKS_DRY_RUN=1

PASS=0 FAIL=0

run_hook() {
  local name="$1" script="$2" payload="$3"
  printf "%-30s" "$name..."
  if output=$(echo "$payload" | bash "$script" 2>&1); then
    echo "OK${output:+ — $output}"
    PASS=$((PASS + 1))
  else
    echo "FAIL — $output"
    FAIL=$((FAIL + 1))
  fi
}

MOCK_SESSION='{"session_id":"test-health-check","cwd":"/tmp"}'
MOCK_STOP='{"session_id":"test-health-check","last_assistant_message":"Done testing"}'
MOCK_NOTIFICATION='{"session_id":"test-health-check","notification_type":"permission_prompt"}'

echo "=== Hook Health Check ==="
echo ""

# Check scripts exist and are executable
echo "--- Script checks ---"
for s in claude-watcher.sh claude-done.sh claude-blocked.sh; do
  printf "%-30s" "$s exists..."
  if [ -x "$SCRIPT_DIR/$s" ]; then
    echo "OK"
    PASS=$((PASS + 1))
  elif [ -f "$SCRIPT_DIR/$s" ]; then
    echo "FAIL — not executable"
    FAIL=$((FAIL + 1))
  else
    echo "FAIL — missing"
    FAIL=$((FAIL + 1))
  fi
done
# Lib is sourced, not executed — just check it exists
printf "%-30s" "claude-hooks-lib.sh exists..."
if [ -f "$SCRIPT_DIR/claude-hooks-lib.sh" ]; then
  echo "OK"
  PASS=$((PASS + 1))
else
  echo "FAIL — missing"
  FAIL=$((FAIL + 1))
fi

# Check channels
echo ""
echo "--- Channel checks ---"
CHANNELS_DIR="${CLAUDE_HOOK_CHANNELS_DIR:-$SCRIPT_DIR/channels}"
if [ -d "$CHANNELS_DIR" ]; then
  for ch in "$CHANNELS_DIR"/*.sh; do
    [ -f "$ch" ] || continue
    chname=$(basename "$ch")
    printf "%-30s" "channel/$chname..."
    if [ -x "$ch" ]; then
      echo "OK"
      PASS=$((PASS + 1))
    else
      echo "FAIL — not executable"
      FAIL=$((FAIL + 1))
    fi
  done
else
  echo "No channels directory at $CHANNELS_DIR"
  FAIL=$((FAIL + 1))
fi

# Run hooks with mock payloads
echo ""
echo "--- Hook execution (dry-run) ---"

# Watcher spawns a background process — give it a moment then kill
printf "%-30s" "claude-watcher.sh..."
echo "$MOCK_SESSION" | bash "$SCRIPT_DIR/claude-watcher.sh" 2>&1
# Signal it to stop immediately
touch "/tmp/claude-watcher-test-health-check.done"
sleep 0.5
rm -f "/tmp/claude-watcher-test-health-check.done" "/tmp/claude-watcher-test-health-check.start"
echo "OK (background started and stopped)"
PASS=$((PASS + 1))

run_hook "claude-done.sh" "$SCRIPT_DIR/claude-done.sh" "$MOCK_STOP"
run_hook "claude-blocked.sh" "$SCRIPT_DIR/claude-blocked.sh" "$MOCK_NOTIFICATION"

# Summary
echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
