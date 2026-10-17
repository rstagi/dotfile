#!/bin/zsh
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
source "$HERE/home-guard.sh"
TMP="$(mktemp -d /tmp/loop-ws-cancel.XXXXXX)"
TMP="${TMP:A}"
WS_TEST_DAEMON_PID="" RUNNER_PID="" GUARD_PID="" TRIGGER_PID=""
RUNNER_GROUPS=()

cleanup() {
  if [[ -n "$TRIGGER_PID" ]]; then
    kill "$TRIGGER_PID" 2>/dev/null
    wait "$TRIGGER_PID" 2>/dev/null
  fi
  if [[ -n "$GUARD_PID" ]]; then
    kill "$GUARD_PID" 2>/dev/null
    wait "$GUARD_PID" 2>/dev/null
  fi
  local group
  for group in "${RUNNER_GROUPS[@]}"; do kill -KILL -- "-$group" 2>/dev/null; done
  [[ -n "$RUNNER_PID" ]] && wait "$RUNNER_PID" 2>/dev/null
  python3 - "$TMP" <<'PY'
import json, os, pathlib, signal, sys
for ready in pathlib.Path(sys.argv[1]).glob('*/engine-ready.json'):
    try:
        os.killpg(json.loads(ready.read_text())['pgid'], signal.SIGKILL)
    except ProcessLookupError:
        pass
PY
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
unset WS_SESSION_ID WS_SESSION TMUX TMUX_PANE
export WS_HOME="$TMP/ws" WS_STATE="$TMP/state" WS_SOCK="$TMP/ws.sock"
export WS_RUST_BIN WS_NOTIFY=0 WS_TMUX_SOCKET="loop-ws-cancel-$$"
export LOOP_WS_SESSIONS=1 LOOP_CONTROL_POLL_SEC=0.05 LOOP_JEV_MODE=off
mkdir -p "$TMP/bin" "$TMP/worktree"
start_ws_test_daemon || exit 1
cat > "$TMP/bin/ws" <<'EOF'
#!/bin/zsh
if [[ "${WS_CANCEL_FAIL_STOP:-0}" == 1 && "$1 $2" == 'session stop' ]]; then
  print -u2 -- 'Synthetic stop API failure.'
  exit 1
fi
exec "$WS_RUST_BIN" "$@"
EOF
chmod +x "$TMP/bin/ws"
export PATH="$TMP/bin:$HERE/fake:$PATH"
print -r -- 'Synthetic cancellation test.' > "$TMP/input.md"

# Advance the timeout only after the test observes engine readiness.
cat > "$TMP/bin/sleep" <<'EOF'
#!/bin/zsh
if [[ "$1" == 10 ]]; then
  exit 0
elif [[ "$1" == <-> ]]; then
  read -r token < "$FAKE_RUN_DIR/timer.fifo"
else
  exec /bin/sleep "$@"
fi
EOF

# Both the engine and its child ignore TERM. Only the daemon owns their process group.
cat > "$TMP/bin/codex" <<'PY'
#!/usr/bin/env python3
import json, os, pathlib, signal, subprocess, sys
if sys.argv[1:3] == ['debug', 'models']:
    print('{"models":[{"slug":"gpt-6-sol","visibility":"list"}]}')
    sys.exit(0)
directory = pathlib.Path(os.environ['FAKE_RUN_DIR'])
ready = directory / 'engine-ready.json'
print('{"type":"thread.started","thread_id":"cancel-engine"}', flush=True)
previous = os.environ.get('WS_CANCEL_PREVIOUS')
if 'resume' in sys.argv or ready.exists():
    previous = str(ready)
if previous:
    old = json.loads(pathlib.Path(previous).read_text())
    try:
        os.killpg(old['pgid'], 0)
        gone = False
    except ProcessLookupError:
        gone = True
    exited = subprocess.run([os.environ['WS_RUST_BIN'], 'wait', old['session'],
                             '--for', 'exit', '--timeout', '0'], capture_output=True).returncode == 0
    (directory / 'previous-check.json').write_text(json.dumps({'gone': gone, 'exited': exited}))
    if 'resume' not in sys.argv:
        ready.write_text(json.dumps({'pgid': os.getpgrp(), 'session': os.environ['WS_SESSION_ID']}))
    outcome = 'done' if gone and exited else 'blocked'
    (directory / 'status.json').write_text(json.dumps({'outcome': outcome}))
    print('{"type":"result","result":"done"}', flush=True)
    sys.exit(0)
signal.signal(signal.SIGTERM, signal.SIG_IGN)
reader, writer = os.pipe()
if os.fork() == 0:
    os.close(reader)
    os.write(writer, b'ready')
    os.close(writer)
    while True:
        signal.pause()
os.close(writer)
os.read(reader, 5)
os.close(reader)
ready.write_text(json.dumps({'pgid': os.getpgrp(), 'session': os.environ['WS_SESSION_ID']}))
if os.environ.get('WS_CANCEL_DONE_EARLY') == '1':
    (directory / 'status.json').write_text('{"outcome":"done"}')
    subprocess.run([os.environ['WS_RUST_BIN'], 'done'], check=True, capture_output=True)
while True:
    signal.pause()
PY
chmod +x "$TMP/bin/codex" "$TMP/bin/sleep"

start_attempt() {
  export FAKE_RUN_DIR="$TMP/$1"
  mkdir -p "$FAKE_RUN_DIR" "$FAKE_RUN_DIR/control" "$FAKE_RUN_DIR/notes"
  mkfifo "$FAKE_RUN_DIR/timer.fifo"
  python3 -c 'import os, sys; os.setsid(); os.execvp("zsh", ["zsh", *sys.argv[1:]])' \
    "$ROOT/loop-runner.sh" --worktree "$TMP/worktree" --run-dir "$FAKE_RUN_DIR" \
    --prompt-file "$TMP/input.md" --chain task --phase 59 --loop-dir "$FAKE_RUN_DIR" \
    --timeout 60 > "$FAKE_RUN_DIR/runner.log" 2>&1 &
  RUNNER_PID=$!
  RUNNER_GROUPS+=("$RUNNER_PID")
  # This timer is a hang guard. Readiness comes from the engine file.
  ( timer=""
    trap '[[ -n "$timer" ]] && { kill "$timer" 2>/dev/null; wait "$timer" 2>/dev/null; }; exit 0' TERM INT HUP
    /bin/sleep 45 & timer=$!
    wait "$timer"
    timer=""
    kill -KILL -- "-$RUNNER_PID" 2>/dev/null
  ) &
  GUARD_PID=$!
  local deadline=$(( SECONDS + 30 ))
  until [[ -s "$FAKE_RUN_DIR/engine-ready.json" ]]; do
    if ! kill -0 "$RUNNER_PID" 2>/dev/null || (( SECONDS >= deadline )); then
      cat "$FAKE_RUN_DIR/runner.log" >&2
      return 1
    fi
    sleep 0.05
  done
}

finish_attempt() {
  wait "$RUNNER_PID"; RC=$?
  RUNNER_PID=""
  if [[ -n "$TRIGGER_PID" ]]; then
    kill "$TRIGGER_PID" 2>/dev/null
    wait "$TRIGGER_PID" 2>/dev/null
    TRIGGER_PID=""
  fi
  kill "$GUARD_PID" 2>/dev/null
  wait "$GUARD_PID" 2>/dev/null
  GUARD_PID=""
  if [[ "$RC" == 50 ]]; then
    cat "$FAKE_RUN_DIR/runner.log" "$FAKE_RUN_DIR/ws-stop.log" >&2
  fi
}

assert_stopped() {
  local sid="$(jq -r .session "$FAKE_RUN_DIR/engine-ready.json")"
  assert_eq "$(python3 - "$FAKE_RUN_DIR/engine-ready.json" <<'PY'
import json, os, sys
try:
    os.killpg(json.load(open(sys.argv[1]))['pgid'], 0)
    print('alive')
except ProcessLookupError:
    print('gone')
PY
)" gone "$1 removes the daemon-owned process group"
  WS_SESSION_ID="$sid" ws wait "$sid" --for exit --timeout 0 >/dev/null 2>&1
  assert_exit "$?" 0 "$1 records the exit event before returning"
}

echo "runner: pause waits for the daemon-owned process group"
start_attempt pause || exit 1
touch "$FAKE_RUN_DIR/control/pause-59"
finish_attempt
assert_exit "$RC" 30 "pause returns 30"
assert_stopped pause

echo "runner: timeout confirms exit before returning the checkpoint"
start_attempt timeout || exit 1
print -r -- expired > "$FAKE_RUN_DIR/timer.fifo" &
TRIGGER_PID=$!
finish_attempt
assert_exit "$RC" 10 "timeout preserves the checkpoint exit code"
assert_eq "$(jq -r .engineExit "$FAKE_RUN_DIR/meta.json")" 124 "timeout records engine exit 124"
assert_eq "$(jq -r .timedOut "$FAKE_RUN_DIR/meta.json")" true "timeout records the watchdog cutoff"
assert_stopped timeout

echo "runner: a retry after timeout starts after the previous group exits"
export WS_CANCEL_PREVIOUS="$FAKE_RUN_DIR/engine-ready.json"
start_attempt retry || exit 1
finish_attempt
assert_exit "$RC" 0 "retry completes"
assert_eq "$(jq -r .gone "$FAKE_RUN_DIR/previous-check.json")" true "retry observes the previous process group exit"
assert_eq "$(jq -r .exited "$FAKE_RUN_DIR/previous-check.json")" true "retry observes the previous exit event"
unset WS_CANCEL_PREVIOUS

echo "runner: a steering note waits for exit before resuming the engine"
start_attempt note || exit 1
print -r -- 'Use the updated synthetic instruction.' > "$FAKE_RUN_DIR/notes/59.md"
finish_attempt
assert_exit "$RC" 0 "steering resumes the engine"
assert_eq "$(jq -r .gone "$FAKE_RUN_DIR/previous-check.json")" true "resume observes the previous process group exit"
assert_eq "$(jq -r .exited "$FAKE_RUN_DIR/previous-check.json")" true "resume observes the previous exit event"
assert_eq "$(jq -s length "$FAKE_RUN_DIR/ws-sessions.jsonl")" 2 "steering creates two runner sessions"
assert_contains "$(cat "$FAKE_RUN_DIR/note-1.md")" 'Use the updated synthetic instruction.' "resume receives the new steering note"
assert_stopped steering

echo "runner: a done event cannot release an attempt with a live process group"
export WS_CANCEL_DONE_EARLY=1
start_attempt done-early || exit 1
finish_attempt
assert_exit "$RC" 0 "early done preserves the successful result"
assert_eq "$(jq -r .gone "$FAKE_RUN_DIR/previous-check.json")" true "retry after done waits for the previous group"
assert_eq "$(jq -r .exited "$FAKE_RUN_DIR/previous-check.json")" true "retry after done waits for the exit event"
assert_stopped 'early done'
unset WS_CANCEL_DONE_EARLY

echo "runner: a failed stop blocks the attempt instead of retrying"
export WS_CANCEL_FAIL_STOP=1
start_attempt stop-failed || exit 1
touch "$FAKE_RUN_DIR/control/pause-59"
finish_attempt
assert_exit "$RC" 20 "an unconfirmed stop blocks the attempt"
assert_eq "$(jq -r .outcome "$FAKE_RUN_DIR/status.json" 2>/dev/null)" blocked "an unconfirmed stop writes blocked status"
assert_eq "$(jq -s length "$FAKE_RUN_DIR/ws-sessions.jsonl")" 1 "an unconfirmed stop starts no retry"

test_summary
