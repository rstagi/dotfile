#!/bin/zsh
set -eu
ROOT="${0:A:h:h}"
python3 - "$ROOT" <<'PY'
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

root = Path(sys.argv[1])
binary = root / 'wsd/target/debug/ws'
assert binary.is_file(), 'Build wsd before the sandbox cleanup test.'
worker = r'''
source "$CANCEL_ROOT/tests/lib.sh"
ROOT="$CANCEL_ROOT"
TMP="$(mktemp -d /tmp/ws-cancel-worker.XXXXXX)"
SOCKET_DIR="$(mktemp -d /tmp/ws-cancel-socket.XXXXXX)"
export HOME="$TMP/home" WS_HOME="$TMP/home/.ws" WS_STATE="$TMP/state"
export WS_SOCK="$SOCKET_DIR/wsd.sock" WS_TMUX_SOCKET="$SOCKET_DIR/tmux.sock"
export GROK_HOME="$HOME/.grok" CODEX_HOME="$HOME/.codex" CLAUDE_CONFIG_DIR="$HOME/.claude"
mkdir -p "$HOME" "$WS_HOME"
worker_pid="" WS_TEST_DAEMON_PID=""
cleanup() {
  trap '' INT TERM HUP
  [[ -z "$worker_pid" ]] || kill -TERM "$worker_pid" 2>/dev/null
  [[ -z "$worker_pid" ]] || wait "$worker_pid" 2>/dev/null
  [[ -z "$WS_TEST_DAEMON_PID" ]] || kill -TERM "$WS_TEST_DAEMON_PID" 2>/dev/null
  [[ -z "$WS_TEST_DAEMON_PID" ]] || wait "$WS_TEST_DAEMON_PID" 2>/dev/null
  tmux -S "$WS_TMUX_SOCKET" kill-server 2>/dev/null || true
  rm -rf "$TMP" "$SOCKET_DIR"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
start_ws_test_daemon || exit 1
tmux -S "$WS_TMUX_SOCKET" -f /dev/null new-session -d -s cancel '/bin/sleep 300'
print -l -- "$WS_TEST_DAEMON_PID" "$TMP" "$WS_TMUX_SOCKET" "$SOCKET_DIR" > "$CANCEL_READY/$$"
/bin/sleep 300 &
worker_pid=$!
wait "$worker_pid"
'''

def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False

for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
    with tempfile.TemporaryDirectory(prefix='ws-cancel-', dir='/tmp') as tmp:
        fixture = Path(tmp)
        ready = fixture / 'ready'
        ready.mkdir()
        (fixture / 'home').mkdir()
        (fixture / 'tests').mkdir()
        (fixture / 'ws/tests').mkdir(parents=True)
        (fixture / 'ws/tests/run.sh').write_text(worker)
        shutil.copyfile(root / 'tests/run.sh', fixture / 'tests/run.sh')
        (fixture / 'tests/worker.test.sh').write_text(worker)
        script, count = 'tests/run.sh', 2
        env = {k: v for k, v in os.environ.items() if not k.startswith(('WS_', 'LOOP_'))}
        env.update(HOME=str(fixture / 'home'), TESTS_REAL_HOME=str(fixture / 'home'),
                   CANCEL_ROOT=str(root), CANCEL_READY=str(ready))
        records = []
        with (fixture / 'log').open('w+') as log:
            process = subprocess.Popen(['zsh', str(fixture / script)], env=env,
                                       stdout=log, stderr=log, start_new_session=True)
            try:
                deadline = time.monotonic() + 30
                while True:
                    records = [p.read_text().splitlines() for p in ready.iterdir()]
                    if len(records) == count and all(len(r) == 4 for r in records):
                        break
                    assert process.poll() is None and time.monotonic() < deadline, 'The test workers did not start.'
                    time.sleep(0.02)
                process.send_signal(sig)
                code = process.wait(timeout=30)
                assert code == 128 + sig, f'runner: expected signal exit, got {code}'
                for pid, directory, socket, notch in records:
                    assert not alive(int(pid)), f'runner: daemon {pid} survived {sig.name}'
                    assert not Path(socket).exists(), f'runner: the tmux socket remains'
                    assert not Path(directory).exists(), f'runner: the sandbox remains'
                    assert not Path(notch).exists(), f'runner: the socket directory remains'
                print(f'  ok: runner cleans its workers on {sig.name}', flush=True)
            except BaseException:
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    pass
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
                for record in records:
                    if len(record) != 4:
                        continue
                    pid, directory, socket, notch = record
                    subprocess.run(['tmux', '-S', socket, 'kill-server'], capture_output=True)
                    if alive(int(pid)):
                        os.kill(int(pid), signal.SIGKILL)
                    shutil.rmtree(directory, ignore_errors=True)
                    shutil.rmtree(notch, ignore_errors=True)
PY
