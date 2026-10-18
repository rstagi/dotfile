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
source "$CANCEL_ROOT/ws/tests/helpers.sh"
export WS_STATE="$WS_HOME/state" WS_SOCK="$NOTCH_HOME/wsd.sock"
"$CANCEL_ROOT/wsd/target/debug/ws" daemon > "$TMP/daemon.log" 2>&1 &
ws_rust_daemon_pid=$!
WS_TEST_WAIT=30 wait_until test -S "$WS_SOCK" || exit 1
print -l -- "$ws_rust_daemon_pid" "$TMP" "$WS_TMUX_SOCKET" "$NOTCH_HOME" > "$CANCEL_READY/$$"
/bin/sleep 300 &
wait $!
'''

def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False

for boundary in ('helper', 'groups', 'runner'):
    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        with tempfile.TemporaryDirectory(prefix='ws-cancel-', dir='/tmp') as tmp:
            fixture = Path(tmp)
            ready = fixture / 'ready'
            ready.mkdir()
            (fixture / 'home').mkdir()
            (fixture / 'tests').mkdir()
            (fixture / 'ws/tests').mkdir(parents=True)
            (fixture / 'worker.sh').write_text(worker)
            (fixture / 'ws/tests/helpers.sh').write_text(worker)
            shutil.copyfile(root / 'ws/tests/run.sh', fixture / 'ws/tests/run.sh')
            shutil.copyfile(root / 'tests/run.sh', fixture / 'tests/run.sh')
            (fixture / 'tests/worker.test.sh').write_text(worker)
            script, count = {
                'helper': ('worker.sh', 1),
                'groups': ('ws/tests/run.sh', 4),
                'runner': ('tests/run.sh', 5),
            }[boundary]
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
                    assert code == 128 + sig, f'{boundary}: expected signal exit, got {code}'
                    for pid, directory, socket, notch in records:
                        assert not alive(int(pid)), f'{boundary}: daemon {pid} survived {sig.name}'
                        assert not Path(socket).exists(), f'{boundary}: the tmux socket remains'
                        assert not Path(directory).exists(), f'{boundary}: the sandbox remains'
                        assert not Path(notch).exists(), f'{boundary}: the socket directory remains'
                    print(f'  ok: {boundary} cleans its workers on {sig.name}', flush=True)
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
