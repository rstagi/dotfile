#!/bin/zsh
# Test the skills controls with a live runner in a private daemon sandbox.
set -u
HERE="${0:A:h}"
ROOT="${HERE:h}"
source "$HERE/lib.sh"
source "$HERE/home-guard.sh"
TMP="$(mktemp -d /tmp/wsd-loop-controls.XXXXXX)"
TMP="${TMP:A}"
WS_TEST_DAEMON_PID="" RUNNER_PID="" GUARD_PID=""
cleanup() {
  if [[ -n "$GUARD_PID" ]]; then
    kill "$GUARD_PID" 2>/dev/null
    wait "$GUARD_PID" 2>/dev/null
  fi
  if [[ -n "$RUNNER_PID" ]]; then
    kill -KILL -- "-$RUNNER_PID" 2>/dev/null
    wait "$RUNNER_PID" 2>/dev/null
  fi
  if [[ -n "$WS_TEST_DAEMON_PID" ]]; then
    kill "$WS_TEST_DAEMON_PID" 2>/dev/null
    wait "$WS_TEST_DAEMON_PID" 2>/dev/null
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
home_guard_sandbox "$TMP/home"
unset WS_SESSION_ID WS_SESSION WS_LOOP_ID WS_LOOP_PHASE WS_LOOP_ATTEMPT TMUX TMUX_PANE
export WS_HOME="$TMP/ws" WS_STATE="$TMP/state" WS_SOCK="$TMP/ws.sock"
export WS_TMUX_SOCKET="$TMP/tmux.sock" WS_NOTIFY=0
export LOOP_CONTROL_POLL_SEC=0.05 LOOP_JEV_MODE=off LOOP_JEV_KEY_FILE="$TMP/no-key"
export PATH="$HERE/fake:$PATH"
mkdir -p "$TMP/worktree/.loop/runs/p7-a1"
LOOP_DIR="$TMP/worktree/.loop"
print -r -- 'Use the public API.' > "$TMP/prompt.md"
start_ws_test_daemon || exit 1
LOOP_ID="$("$WS_RUST_BIN" loop create --dir "$LOOP_DIR" --json | jq -r .id)"
export FAKE_RUN_DIR="$LOOP_DIR/runs/p7-a1" FAKE_ENGINE_LOG="$TMP/engines.log"
export FAKE_CODEX_OUTCOME=done FAKE_CODEX_SLEEP=120 FAKE_CODEX_SESSION=controls
python3 -c 'import os,sys; os.setsid(); os.execvp("zsh", ["zsh", *sys.argv[1:]])' \
  "$ROOT/loop-runner.sh" --worktree "$TMP/worktree" --run-dir "$FAKE_RUN_DIR" \
  --prompt-file "$TMP/prompt.md" --phase 7 --loop-dir "$LOOP_DIR" --timeout 60 \
  > "$TMP/runner.log" 2>&1 &
RUNNER_PID=$!
# The timer only prevents a hang. The engine log is the readiness event.
( timer=""
  trap '[[ -n "$timer" ]] && { kill "$timer" 2>/dev/null; wait "$timer" 2>/dev/null; }; exit 0' TERM INT HUP
  /bin/sleep 45 & timer=$!
  wait "$timer"
  kill -KILL -- "-$RUNNER_PID" 2>/dev/null
) &
GUARD_PID=$!
deadline=$(( SECONDS + 30 ))
until [[ -s "$FAKE_ENGINE_LOG" ]]; do
  if ! kill -0 "$RUNNER_PID" 2>/dev/null || (( SECONDS >= deadline )); then
    cat "$TMP/runner.log" >&2
    exit 1
  fi
  /bin/sleep 0.05
done
"$WS_RUST_BIN" loop pause "$LOOP_ID" 7 --json > "$TMP/pause.json" || exit 1
wait "$RUNNER_PID"; RC=$?
RUNNER_PID=""
kill "$GUARD_PID" 2>/dev/null
wait "$GUARD_PID" 2>/dev/null
GUARD_PID=""
assert_exit "$RC" 30 'the daemon pause stops a live runner attempt'
assert_eq "$(jq -r .engineExit "$FAKE_RUN_DIR/meta.json")" 30 'the runner records the pause'
assert_eq "$([[ -f "$LOOP_DIR/control/pause-7" ]] && echo yes)" yes 'the daemon writes the phase pause file'
"$WS_RUST_BIN" loop resume "$LOOP_ID" 7 --json > "$TMP/resume.json" || exit 1
assert_eq "$([[ -e "$LOOP_DIR/control/pause-7" ]] && echo yes || echo no)" no 'resume removes the phase pause file'
"$WS_RUST_BIN" loop model "$LOOP_ID" 7 claude:claude-opus-5-5 --json > "$TMP/model.json" || exit 1
unset FAKE_CODEX_SLEEP
export FAKE_RUN_DIR="$LOOP_DIR/runs/p7-a2"
mkdir -p "$FAKE_RUN_DIR"
: > "$FAKE_ENGINE_LOG"
zsh "$ROOT/loop-runner.sh" --worktree "$TMP/worktree" --run-dir "$FAKE_RUN_DIR" \
  --prompt-file "$TMP/prompt.md" --phase 7 --loop-dir "$LOOP_DIR" --timeout 30 \
  > "$TMP/retry.log" 2>&1
assert_exit "$?" 0 'the next attempt runs after resume'
assert_contains "$(head -1 "$FAKE_ENGINE_LOG")" 'claude -p --model claude-opus-5-5 ' 'the next attempt uses the selected model'
assert_eq "$(jq -r .modelOverride "$FAKE_RUN_DIR/meta.json")" claude:claude-opus-5-5 'the runner records the selected model'
test_summary
