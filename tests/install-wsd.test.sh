#!/bin/zsh
# Check the ws installer with a private home and fake external commands.
set -eu
ROOT="${0:A:h:h}"
python3 - "$ROOT" <<'PY'
import os
import plistlib
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="ws-install-", dir="/tmp") as directory:
    home = Path(directory)
    environment = dict(os.environ, HOME=str(home), WS_HOME=str(home / "custom ws"))
    output = subprocess.run([str(root / "install.sh"), "--dry-run", "ws"],
                            env=environment, capture_output=True, text=True, check=True).stdout
    for step in ("rust", "brew install rustup", "rustup default stable", "cargo build --release",
                 "ws-rs", "com.rstagi.wsd.plist", "KeepAlive", "launchctl bootstrap",
                 "launchctl enable", "launchctl kickstart"):
        assert step in output, (step, output)
    assert not list(home.iterdir()), "The dry run wrote files."
    (home / "dotfile/ws").mkdir(parents=True)
    (home / "dotfile/wsd/launchd").mkdir(parents=True)
    (home / "dotfile/wsd/target/release").mkdir(parents=True)
    (home / "dotfile/ws/ws.sh").write_text("#!/bin/sh\nexit 0\n")
    (home / "dotfile/ws/ws.sh").chmod(0o755)
    (home / "dotfile/wsd/target/release/ws").write_text("binary")
    template = root / "wsd/launchd/com.rstagi.wsd.plist"
    if template.exists():
        shutil.copy(template, home / "dotfile/wsd/launchd" / template.name)
    commands = home / "commands"
    commands.mkdir()
    for name in ("cargo", "launchctl", "grok"):
        path = commands / name
        path.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$HOME/' + name + '.calls"\n')
        path.chmod(0o755)
    environment.update(PATH=str(commands) + ":" + os.environ["PATH"],
                       WS_HOME=str(home / "custom & ws"), WS_STATE=str(home / "state"),
                       WS_SOCK=str(home / "daemon.sock"), WS_ROOT=str(home / "work <trees>"),
                       WS_TMUX_SOCKET="ws-private")
    definitions = (root / "install.sh").read_text().rsplit('main "$@"', 1)[0]
    script = definitions + '\nINTERACTIVE_MODE=false\ninstall_pkg_if_needed() { return 0; }\ninstall_ws\n'
    subprocess.run(["/bin/bash"], input=script, text=True, env=environment, check=True)
    binary = home / "custom & ws/bin/ws-rs"
    assert binary.read_text() == "binary"
    assert (home / "custom & ws/bin/ws").resolve() == (home / "dotfile/ws/ws.sh").resolve(), (home / "custom & ws/bin/ws").resolve()
    assert (home / ".local/bin/ws").resolve() == (home / "dotfile/ws/ws.sh").resolve(), (home / ".local/bin/ws").resolve()
    agent = plistlib.loads((home / "Library/LaunchAgents/com.rstagi.wsd.plist").read_bytes())
    assert agent["Label"] == "com.rstagi.wsd"
    assert agent["ProgramArguments"] == [str(binary), "daemon"], agent
    assert agent["RunAtLoad"] is True and agent["KeepAlive"] is True
    for key in ("HOME", "PATH", "WS_HOME", "WS_STATE", "WS_SOCK", "WS_ROOT", "WS_TMUX_SOCKET"):
        assert agent["EnvironmentVariables"][key] == environment[key]
    defaults = {key: value for key, value in environment.items() if key not in ("WS_ROOT", "WS_TMUX_SOCKET")}
    script = definitions + '\nINTERACTIVE_MODE=false\ninstall_ws_daemon\n'
    subprocess.run(["/bin/bash"], input=script, text=True, env=defaults, check=True, capture_output=True)
    agent = plistlib.loads((home / "Library/LaunchAgents/com.rstagi.wsd.plist").read_bytes())
    assert agent["EnvironmentVariables"]["WS_ROOT"] == str(home / ".ws/worktrees"), agent
    assert agent["EnvironmentVariables"]["WS_TMUX_SOCKET"] == "", agent
    assert (home / "cargo.calls").read_text().splitlines() == ["build --release"]
    calls = (home / "launchctl.calls").read_text()
    for operation in ("bootout", "bootstrap", "enable", "kickstart"):
        assert operation in calls, calls
print("PASS: ws dry run and sandbox installation")
PY
