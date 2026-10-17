#!/bin/zsh
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
source "$HERE/home-guard.sh"
TMP="$(mktemp -d /tmp/loop-ws.XXXXXX)"
TMP="${TMP:A}"
WS_TEST_DAEMON_PID=""
cleanup() {
  if [[ -n "$WS_TEST_DAEMON_PID" ]]; then
    kill "$WS_TEST_DAEMON_PID" 2>/dev/null
    wait "$WS_TEST_DAEMON_PID" 2>/dev/null
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT
home_guard_sandbox "$TMP/home"
export WS_HOME="$TMP/ws" WS_STATE="$TMP/state" WS_SOCK="$TMP/ws.sock"
export WS_NOTIFY=0 WS_TMUX_SOCKET="loop-ws-test-$$"
export LOOP_JEV_MODE=off LOOP_JEV_KEY_FILE="$TMP/no-key"
mkdir -p "$TMP/bin" "$TMP/work tree"
start_ws_test_daemon || exit 1
ln -s "$WS_RUST_BIN" "$TMP/bin/ws"
export PATH="$TMP/bin:$HERE/fake:$PATH"
export FAKE_CODEX_OUTCOME=done FAKE_CODEX_SESSION=engine-session
export FAKE_ENGINE_LOG="$TMP/engines.log"
print -r -- 'Synthetic prompt: $HOME `literal` "quoted"' > "$TMP/input.md"

run_attempt() {
  export FAKE_RUN_DIR="$TMP/$1"
  mkdir -p "$FAKE_RUN_DIR"
  zsh "$ROOT/loop-runner.sh" --worktree "$TMP/work tree" --run-dir "$FAKE_RUN_DIR" \
    --prompt-file "$TMP/input.md" --chain task --timeout 30 > "$TMP/$1.log" 2>&1
  RC=$?
}

echo "runner: Rust daemon owns the runner session"
run_attempt codex
assert_exit "$RC" 0 "runner completes through ws"
SID="$(jq -r '.wsSession // empty' "$FAKE_RUN_DIR/meta.json")"
assert_eq "$([[ "$SID" =~ '^[0-9A-Z]{26}$' ]] && echo yes || echo no)" yes "attempt records a ws session id"
if [[ -n "$SID" ]]; then
  RECORD="$(ws session get "$SID" --json)"
  assert_eq "$(print -r -- "$RECORD" | jq -r '.body.cwd')" "$TMP/work tree" "daemon keeps the worktree directory"
  assert_eq "$(print -r -- "$RECORD" | jq -r '.exit_code')" 0 "daemon records the engine exit code"
fi
assert_eq "$(jq -r '.sessionId' "$FAKE_RUN_DIR/meta.json")" engine-session "engine resume id stays separate"
assert_contains "$(cat "$FAKE_ENGINE_LOG")" 'mcp_servers.ws.command=' "runner adds prepared MCP arguments"

echo "orchestrator: state identifies its daemon-owned session"
mkdir -p "$TMP/loop"
print -r -- '{"milestone":2,"phases":{"31":{"status":"running"}}}' > "$TMP/loop/state.json"
export ENGINE_CMD="$HERE/fake/fake-engine.sh" FAKE_OUTCOMES=complete
export LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" LOOP_NOTIFY_SH="$HERE/fake/loop-notify.sh"
export EMIT_LOG="$TMP/emit.log" NOTIFY_LOG="$TMP/notify.log"
zsh "$ROOT/loop-orchestrator.sh" --dir "$TMP/loop" --run-id test-ws > "$TMP/orchestrator.log" 2>&1
assert_exit "$?" 0 "orchestrator completes through ws"
ORCH_SID="$(jq -r '.wsSession // empty' "$TMP/loop/state.json")"
assert_eq "$([[ "$ORCH_SID" =~ '^[0-9A-Z]{26}$' ]] && echo yes || echo no)" yes "state records the orchestrator session id"
assert_eq "$(jq -r '.phases["31"].status' "$TMP/loop/state.json")" running "state keeps existing phase data"
test_summary
