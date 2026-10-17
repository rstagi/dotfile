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
export WS_RUST_BIN
export WS_NOTIFY=0 WS_TMUX_SOCKET="loop-ws-test-$$"
export LOOP_WS_SESSIONS=1
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
  export FAKE_CONTEXT_DIR="$FAKE_RUN_DIR/context"
  shift
  mkdir -p "$FAKE_RUN_DIR"
  zsh "$ROOT/loop-runner.sh" --worktree "$TMP/work tree" --run-dir "$FAKE_RUN_DIR" \
    --prompt-file "$TMP/input.md" --chain task --timeout 30 "$@" < /dev/null > "$FAKE_RUN_DIR/runner.log" 2>&1
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

echo "runner: completed attempts remove the watchdog process group members"
cat > "$TMP/bin/sleep" <<'EOF'
#!/bin/zsh
[[ "$1" == 30 && -n "${FAKE_WATCHDOG_READY:-}" ]] && print -r -- "$$" > "$FAKE_WATCHDOG_READY"
exec /bin/sleep "$@"
EOF
chmod +x "$TMP/bin/sleep"
export FAKE_WATCHDOG_READY="$TMP/watchdog-ready"
LOOP_WS_SESSIONS=0 run_attempt watchdog-cleanup
watchdog_sleep="$(cat "$FAKE_WATCHDOG_READY")"
assert_eq "$(kill -0 "$watchdog_sleep" 2>/dev/null && echo alive || echo gone)" gone "runner reaps the watchdog sleep before it returns"
kill "$watchdog_sleep" 2>/dev/null
unset FAKE_WATCHDOG_READY
rm "$TMP/bin/sleep"

echo "runner: preparation preserves engine I/O and resume"
export FAKE_SENTINEL=$'literal $HOME `code`\nsecond line'
for engine in codex claude grok; do
  for mode in fresh resume; do
    role="$engine"
    [[ "$engine" == codex ]] && role=gpt
    args=(--chain "review-$role")
    [[ "$mode" == resume ]] && args=(--resume "old-$engine-session" --engine "$engine")
    run_attempt "$engine-$mode" "${args[@]}"
    assert_exit "$RC" 0 "$engine $mode completes"
    context="$FAKE_CONTEXT_DIR/$engine.json"
    SID="$(jq -r '.wsSession' "$FAKE_RUN_DIR/meta.json")"
    assert_eq "$(jq -r '.wsSession' "$context")" "$SID" "$engine receives its ws identity"
    assert_eq "$(jq -r '.cwd' "$context")" "$TMP/work tree" "$engine runs in the worktree"
    assert_eq "$(jq -r '.sentinel' "$context")" "$FAKE_SENTINEL" "$engine receives the caller environment"
    assert_eq "$(cat "$FAKE_CONTEXT_DIR/$engine.stdin")" "$(cat "$FAKE_RUN_DIR/prompt.md")" "$engine receives the prompt on stdin"
    assert_contains "$(cat "$FAKE_RUN_DIR/stderr.log")" 'fake engine stderr' "$engine stderr stays separate"
    expected_result=done
    [[ "$engine" == grok ]] && expected_result='grok done'
    assert_eq "$(jq -r 'select(.type=="result") | .result' "$FAKE_RUN_DIR/transcript.jsonl")" "$expected_result" "$engine transcript remains parseable"
    if [[ "$engine" == claude ]]; then
      settings="$(jq -r '.argv | index("--settings") as $i | .[$i+1]' "$context")"
      assert_eq "$(jq -r '.hooks.Stop[0].hooks[0].command | endswith("hook stop")' "$settings")" true "Claude uses prepared hooks"
    elif [[ "$engine" == codex ]]; then
      assert_eq "$(jq -r '[.argv[] | select(startswith("hooks.Stop="))] | length' "$context")" 1 "Codex uses prepared hooks"
      assert_eq "$(jq -r '[.argv[] | select(.=="--dangerously-bypass-approvals-and-sandbox")] | length' "$context")" 1 "Codex receives its permission flag once"
    else
      assert_eq "$(jq -r '[.argv[] | select(.=="--rules")] | length' "$context")" 1 "Grok receives prepared rules"
      assert_eq "$(jq -r '[.argv[] | select(.=="--always-approve")] | length' "$context")" 1 "Grok receives its permission flag once"
      assert_eq "$([[ -f "$GROK_HOME/hooks/ws.json" ]] && echo yes || echo no)" yes "Grok receives its hook file"
    fi
    if [[ "$mode" == resume ]]; then
      assert_contains "$(jq -r '.argv | join(" ")' "$context")" "old-$engine-session" "$engine keeps the engine resume id"
    fi
  done
done

echo "runner: API failure classification and status exits stay unchanged"
export FAKE_CODEX_OUTCOME=rate-limit
run_attempt fallback
assert_exit "$RC" 0 "rate-limited Codex falls back to Claude"
assert_eq "$(jq -s 'length' "$FAKE_RUN_DIR/ws-sessions.jsonl")" 2 "attempt records both runner sessions"
assert_eq "$(jq -r '.engine' "$FAKE_RUN_DIR/meta.json")" claude "metadata names the successful engine"
export FAKE_CLAUDE_OUTCOME=session-limit
run_attempt exhausted
assert_exit "$RC" 40 "rate-limited chain returns 40"
unset FAKE_CLAUDE_OUTCOME
export FAKE_CODEX_OUTCOME=done
for outcome expected in question 10 blocked 20 invalid 50; do
  export FAKE_STATUS_OUTCOME="$outcome"
  run_attempt "status-$outcome"
  assert_exit "$RC" "$expected" "$outcome keeps its runner exit code"
done
unset FAKE_STATUS_OUTCOME
run_attempt verify-failed --verify-cmd false
assert_exit "$RC" 12 "verification failure returns 12"

echo "runner: disabled, unavailable and legacy ws paths use direct engines"
for availability in disabled down legacy missing; do
  case "$availability" in
    disabled) export LOOP_WS_SESSIONS=0 ;;
    down) export WS_SOCK="$TMP/missing.sock" ;;
    legacy) rm "$TMP/bin/ws"; print -r -- '#!/bin/sh\nexit 1' > "$TMP/bin/ws"; chmod +x "$TMP/bin/ws" ;;
    missing) rm "$TMP/bin/ws" ;;
  esac
  run_attempt "direct-$availability"
  assert_exit "$RC" 0 "$availability runs directly"
  assert_eq "$(jq -r '.wsSession' "$FAKE_RUN_DIR/meta.json")" null "$availability has no runner ws session"
  assert_eq "$(jq -r '[.argv[] | select(startswith("mcp_servers.ws."))] | length' "$FAKE_CONTEXT_DIR/codex.json")" 0 "$availability has no prepared arguments"
  unset LOOP_WS_SESSIONS
  export WS_SOCK="$TMP/ws.sock"
done
ln -s "$WS_RUST_BIN" "$TMP/bin/ws"

echo "orchestrator: state identifies its daemon-owned session"
mkdir -p "$TMP/loop"
print -r -- '{"milestone":2,"phases":{"31":{"status":"running"}}}' > "$TMP/loop/state.json"
export ENGINE_CMD="$HERE/fake/fake-engine.sh" FAKE_OUTCOMES=complete
export LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" LOOP_NOTIFY_SH="$HERE/fake/loop-notify.sh"
export EMIT_LOG="$TMP/emit.log" NOTIFY_LOG="$TMP/notify.log"
export EFFORT_ORCHESTRATE=ultra FAKE_ENGINE_ARGS_LOG="$TMP/orchestrator-args.log"
export FAKE_ORCH_RUNNER="$ROOT/loop-runner.sh"
export FAKE_RUN_DIR="$TMP/orchestrated-attempt" FAKE_CONTEXT_DIR="$TMP/orchestrated-context"
export FAKE_RUNNER_WORKTREE="$TMP/work tree" FAKE_RUNNER_PROMPT="$TMP/input.md"
mkdir -p "$FAKE_RUN_DIR"
zsh "$ROOT/loop-orchestrator.sh" --dir "$TMP/loop" --run-id test-ws > "$TMP/orchestrator.log" 2>&1
assert_exit "$?" 0 "orchestrator completes through ws"
ORCH_SID="$(jq -r '.wsSession // empty' "$TMP/loop/state.json")"
assert_eq "$([[ "$ORCH_SID" =~ '^[0-9A-Z]{26}$' ]] && echo yes || echo no)" yes "state records the orchestrator session id"
assert_eq "$(jq -r '.phases["31"].status' "$TMP/loop/state.json")" running "state keeps existing phase data"
assert_contains "$(cat "$FAKE_ENGINE_ARGS_LOG")" '--effort max' "orchestrator keeps the caller effort override"
RUNNER_SID="$(jq -r '.wsSession' "$FAKE_RUN_DIR/meta.json")"
assert_eq "$(ws session get "$RUNNER_SID" --json | jq -r '.body.parent_session_id')" "$ORCH_SID" "runner session links to the orchestrator"
test_summary
