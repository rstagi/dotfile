#!/bin/zsh
# Check that after `install_ws_daemon` the launchd job holds the wsd socket lock.
# The fake launchctl starts a stray ws-rs daemon during the first kickstart, as a ws client does in the install window.
# The fake lsof reports the live holder of the lock. No test calls the real launchctl.
set -eu
ROOT="${0:A:h:h}"
python3 - "$ROOT" <<'PY'
import contextlib
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile

root = Path(sys.argv[1])

LAUNCHCTL = r'''#!/bin/sh
printf '%s\n' "$*" >> "$HOME/launchctl.calls"
spawn() { nohup "$HOME/ws-rs" 300 > /dev/null 2>&1 < /dev/null & echo $!; }
# True when a process other than the job holds the lock.
other_holds() {
  [ -e "$HOME/holder.pid" ] && [ "$(cat "$HOME/holder.pid")" != "$1" ] && kill -0 "$(cat "$HOME/holder.pid")" 2>/dev/null
}
case "$1" in
  bootout) rm -f "$HOME/loaded" ;;
  print)
    [ -e "$HOME/loaded" ] || exit 113
    echo "	pid = $(cat "$HOME/job.pid")" ;;
  bootstrap)
    touch "$HOME/loaded"
    spawn > "$HOME/job.pid" ;;
  kickstart)
    if [ "$2" = "-k" ]; then
      old="$(cat "$HOME/job.pid")"
      kill "$old" 2>/dev/null
      spawn > "$HOME/job.pid"
      # A restarted job takes the lock when no other process holds it.
      other_holds "$old" || cp "$HOME/job.pid" "$HOME/holder.pid"
    elif [ "$SCENARIO" = race ] && [ ! -e "$HOME/stray.pid" ]; then
      # A client starts its own daemon first. It holds the lock. The job exits.
      spawn > "$HOME/stray.pid"
      cp "$HOME/stray.pid" "$HOME/holder.pid"
    elif [ "$SCENARIO" = stuck ]; then
      # A holder that is not ws-rs: the installer cannot stop it.
      if [ ! -e "$HOME/holder.pid" ]; then
        nohup /bin/sleep 300 > /dev/null 2>&1 < /dev/null &
        echo $! > "$HOME/holder.pid"
      fi
    else
      cp "$HOME/job.pid" "$HOME/holder.pid"
    fi ;;
esac
exit 0
'''


@contextlib.contextmanager
def sandbox(scenario):
    """Yield (home, result) of one install_ws_daemon run. Kill every process of the sandbox afterwards."""
    with tempfile.TemporaryDirectory(prefix="ws-race-", dir="/tmp") as directory:
        home = Path(directory)
        (home / "dotfile/wsd/launchd").mkdir(parents=True)
        shutil.copy(root / "wsd/launchd/com.rstagi.wsd.plist", home / "dotfile/wsd/launchd")
        commands = home / "commands"
        commands.mkdir()
        # Every daemon of the test is a link to sleep named ws-rs.
        (home / "ws-rs").symlink_to("/bin/sleep")
        socket = home / "daemon.sock"
        (commands / "launchctl").write_text(LAUNCHCTL)
        (commands / "lsof").write_text(f'''#!/bin/sh
[ "$1" = "-t" ] && [ "$2" = "{socket}.lock" ] || exit 1
pid="$(cat "$HOME/holder.pid" 2>/dev/null)" || exit 1
kill -0 "$pid" 2>/dev/null && echo "$pid"
exit 0
''')
        for name in ("launchctl", "lsof"):
            (commands / name).chmod(0o755)
        environment = dict(os.environ, HOME=str(home), WS_HOME=str(home / "ws"), WS_STATE=str(home / "state"),
                           WS_SOCK=str(socket), SCENARIO=scenario, WS_LOCK_WAIT_TRIES="5",
                           PATH=str(commands) + ":" + os.environ["PATH"])
        definitions = (root / "install.sh").read_text().rsplit('main "$@"', 1)[0]
        try:
            result = subprocess.run(["/bin/bash"], input=definitions + "\ninstall_ws_daemon\n", text=True,
                                    env=environment, capture_output=True, timeout=60)
            yield home, result
        finally:
            for name in ("job.pid", "stray.pid", "holder.pid"):
                with contextlib.suppress(OSError, ValueError):
                    os.kill(int(Path(home / name).read_text()), signal.SIGKILL)


def alive(pid):
    try:
        os.kill(pid, 0)
    except OSError:
        return False
    state = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True).stdout
    return bool(state.strip()) and not state.strip().startswith("Z")


def pid_of(home, name):
    return int((home / name).read_text())


def verbs(home):
    return [line.split() for line in (home / "launchctl.calls").read_text().splitlines()]


# A clean install: the job holds the lock after one kickstart and nothing restarts it.
with sandbox("clean") as (home, result):
    assert result.returncode == 0, result
    assert pid_of(home, "holder.pid") == pid_of(home, "job.pid"), "The job must hold the lock."
    assert ["kickstart", "-k"] not in [call[:2] for call in verbs(home)], verbs(home)

# A stray daemon takes the lock during the restart: the installer stops it and restarts the job one time.
with sandbox("race") as (home, result):
    assert result.returncode == 0, result
    stray, job = pid_of(home, "stray.pid"), pid_of(home, "job.pid")
    assert pid_of(home, "holder.pid") == job, "The launchd job must hold the lock at the end."
    assert not alive(stray), "The stray ws-rs daemon still runs."
    assert alive(job), "The launchd job must run."
    restarts = [call for call in verbs(home) if call[:2] == ["kickstart", "-k"]]
    assert len(restarts) == 1, verbs(home)

# A holder that the installer cannot stop: one error line and exit 1, after one restart.
with sandbox("stuck") as (home, result):
    assert result.returncode == 1, result
    errors = [line for line in result.stderr.splitlines() if line.startswith("ws: ")]
    assert len(errors) == 1, result.stderr
    restarts = [call for call in verbs(home) if call[:2] == ["kickstart", "-k"]]
    assert len(restarts) == 1, verbs(home)
print("PASS: ws install leaves the socket lock with the launchd job")
PY
