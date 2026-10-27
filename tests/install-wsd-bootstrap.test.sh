#!/bin/zsh
# Check that the wsd LaunchAgent install survives a slow launchd unload and a stray daemon.
# The fake launchctl keeps the job loaded for two `print` calls after bootout and fails the first bootstrap with error 5.
set -eu
ROOT="${0:A:h:h}"
python3 - "$ROOT" <<'PY'
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="ws-bootstrap-", dir="/tmp") as directory:
    home = Path(directory)
    (home / "dotfile/wsd/launchd").mkdir(parents=True)
    shutil.copy(root / "wsd/launchd/com.rstagi.wsd.plist", home / "dotfile/wsd/launchd")
    commands = home / "commands"
    commands.mkdir()
    (commands / "launchctl").write_text(r'''#!/bin/sh
printf '%s\n' "$*" >> "$HOME/launchctl.calls"
case "$1" in
  bootout) echo 2 > "$HOME/unload-delay" ;;
  print)
    delay="$(cat "$HOME/unload-delay" 2>/dev/null || echo 0)"
    if [ "$delay" -le 0 ]; then
      # After the bootstrap the job runs with a pid. Before it, the job is not loaded.
      [ -e "$HOME/job.pid" ] || exit 113
      echo "	pid = $(cat "$HOME/job.pid")"
      exit 0
    fi
    echo $((delay - 1)) > "$HOME/unload-delay" ;;
  bootstrap)
    if [ ! -e "$HOME/bootstrap-failed" ]; then
      touch "$HOME/bootstrap-failed"
      echo "Bootstrap failed: 5: Input/output error" >&2
      exit 5
    fi
    nohup /bin/sleep 300 > /dev/null 2>&1 < /dev/null &
    echo $! > "$HOME/job.pid" ;;
esac
exit 0
''')
    # The stray daemon is a real process named ws-rs. The decoy also holds the lock but has another name.
    stray_binary = home / "stray/ws-rs"
    stray_binary.parent.mkdir()
    shutil.copy("/bin/sleep", stray_binary)
    stray = subprocess.Popen([str(stray_binary), "300"])
    decoy = subprocess.Popen(["/bin/sleep", "300"])
    socket = home / "daemon.sock"
    (commands / "lsof").write_text(f'''#!/bin/sh
printf '%s\\n' "$*" >> "$HOME/lsof.calls"
[ "$1" = "-t" ] && [ "$2" = "{socket}.lock" ] || exit 1
echo {stray.pid}
echo {decoy.pid}
cat "$HOME/job.pid" 2>/dev/null
exit 0
''')
    for name in ("launchctl", "lsof"):
        (commands / name).chmod(0o755)
    environment = dict(os.environ, HOME=str(home), WS_HOME=str(home / "ws"), WS_STATE=str(home / "state"),
                       WS_SOCK=str(socket), PATH=str(commands) + ":" + os.environ["PATH"])
    definitions = (root / "install.sh").read_text().rsplit('main "$@"', 1)[0]
    try:
        result = subprocess.run(["/bin/bash"], input=definitions + "\ninstall_ws_daemon\n", text=True,
                                env=environment, capture_output=True, timeout=60)
        assert result.returncode == 0, result
        calls = [line.split()[0] for line in (home / "launchctl.calls").read_text().splitlines()]
        assert calls.count("bootstrap") == 2, calls
        assert calls.count("print") >= 3, calls
        bootout = calls.index("bootout")
        first_bootstrap = calls.index("bootstrap")
        unloaded = max(index for index, call in enumerate(calls) if call == "print" and index < first_bootstrap)
        assert bootout < unloaded < first_bootstrap, calls
        assert [call for call in calls if call != "print"][-1] == "kickstart", calls
        assert stray.wait(timeout=10) is not None, "The stray ws-rs daemon still runs."
        assert decoy.poll() is None, "The installer stopped a process that is not ws-rs."
    finally:
        if (home / "job.pid").exists():
            subprocess.run(["kill", (home / "job.pid").read_text().strip()])
        for process in (stray, decoy):
            if process.poll() is None:
                process.kill()
                process.wait()
print("PASS: wsd LaunchAgent install waits for the unload, retries bootstrap and stops a stray daemon")
PY
