#!/bin/zsh
# Shared setup for the ws suite, sourced by ws/tests/run.sh: private HOME + tmux server (-L), fake
# agent bins, the real-HOME guard and the helper functions the area files use. Terminal workspace
# manager (worktree + one tmux window per agent session) under test: ws/ws.sh. Never touches
# the user's tmux server or agent config.
set -u
# Every ws tab exports WS_*: a suite launched from one must not inherit its identity.
for inherited in ${(k)parameters}; do
  [[ "$inherited" != WS_* ]] || unset "$inherited"
done
HERE="${${(%):-%x}:A:h}"
ROOT="${HERE:h:h}" # physical (:A): ws.sh records ${0:A}
source "$ROOT/tests/lib.sh"

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
# Private HOME + agent config roots (also for subshells overriding GROK_HOME/WS_HOME after
# `source "$WS"`); the EXIT trap fails the suite if real agent hooks/config changed.
source "$ROOT/tests/home-guard.sh"
home_guard_sandbox "$TMP/home"
real_home_snapshot="$(home_guard_snapshot)"
# Keep the default Unix socket path below macOS's 104-byte limit.
NOTCH_HOME="$(mktemp -d /tmp/ws-notch.XXXXXX)"
export WS_TMUX_SOCKET="ws-test-$$"
export WS_ROOT="$TMP/worktrees"
export WS_HOME="$TMP/wshome"
export WS_NOTIFY=0
export WS_NOTIFIER=auto
export WS_BORINGNOTCH_SOCKET="$TMP/unavailable.sock"
export WS_AGENT_SHELL="zsh -fc"
export LOOP_REPO_REGISTRY="$TMP/repos.json"
export GROK_HOME="$TMP/grokhome"
socket_listener_pid=""
trap '[[ -n "$socket_listener_pid" ]] && kill "$socket_listener_pid" 2>/dev/null; tmux -L "$WS_TMUX_SOCKET" kill-server 2>/dev/null; rm -rf "$TMP" "$NOTCH_HOME"; home_guard_check "$real_home_snapshot" || exit 1' EXIT
WS="$ROOT/ws/ws.sh"
export WS_TEST_CLI="$WS"
T() { tmux -L "$WS_TMUX_SOCKET" "$@"; }

# Fake agents print their argv, then stay alive until a line arrives (→ clean exit 0).
mkdir -p "$TMP/bin"
# Fake delivery TUI records bytes submitted outside bracketed paste, including multiline input.
cat > "$TMP/paste-agent.py" <<'PY_AGENT'
import json, os, subprocess, time, tty
delay = 0.6 # delayed startup exposes paste before the resumed TUI is ready
try: # one-shot override: a test's next TUI starts slower than the hook-lock timeout
    with open(os.environ["WS_HOME"] + "/tui-delay") as f: delay = float(f.read())
    os.remove(os.environ["WS_HOME"] + "/tui-delay")
except FileNotFoundError: pass
time.sleep(delay)
tty.setraw(0)
print("\033[?2004h", end="", flush=True)
print("PASTE-READY", flush=True)
buf = b""
while True:
    ch = os.read(0, 1)
    buf += ch
    if ch == b"\r" and not (b"\033[200~" in buf and b"\033[201~" not in buf):
        if not os.path.exists(os.environ["WS_HOME"] + "/no-submit-" + os.environ["WS_SESSION"]):
            subprocess.run([os.environ["WS_TEST_CLI"], "hook", "working"],
                           input=b'{"hook_event_name":"UserPromptSubmit"}', check=True)
        with open(os.environ["WS_HOME"] + "/pane-" + os.environ["WS_SESSION"] + ".jsonl", "a") as f:
            f.write(json.dumps({"raw": buf.decode(), "text": buf[:-1].replace(b"\033[200~", b"").replace(b"\033[201~", b"").decode()}) + "\n")
        buf = b""
PY_AGENT
for agent in claude codex grok; do
  cat > "$TMP/bin/$agent" <<EOF
#!/bin/sh
own_id=\$("\$WS_TEST_CLI" whoami)
echo "FAKE-$agent \$* WS=\$WS_WORKSPACE ID=\$WS_SESSION WHOAMI=\$own_id"
python3 -c 'import json,sys; json.dump(sys.argv[1:],sys.stdout)' "\$@" > "\$WS_HOME/argv-\$WS_SESSION.json"
[ "\$WS_WORKSPACE" != api/delivery ] || exec python3 "$TMP/paste-agent.py"
read line
if [ "\$line" = done ]; then
  printf 'agent result\\n' | "\$WS_TEST_CLI" done -
  echo DONE-RETURNED
  read line
fi
exit "\${line:-0}"
EOF
  chmod +x "$TMP/bin/$agent"
done
export PATH="$TMP/bin:$PATH"
T -f /dev/null new-session -d -s keepalive -x 200 -y 50
# Mirror common user configs (incl. rstagi's): 1-based windows/panes must not break targeting.
T set -g base-index 1
T set -g pane-base-index 1
T set -g default-size 250x50 # detached worktree sessions: avoid wrapped pane captures

make_repo() {
  local repo="$TMP/$1"
  git init -q -b main "$repo"
  git -C "$repo" config user.email test@example.com
  git -C "$repo" config user.name Test
  print one > "$repo/file.txt"
  print '.env*' > "$repo/.gitignore"
  git -C "$repo" add file.txt .gitignore
  git -C "$repo" commit -qm initial
  print "$repo"
}

# wait_pane <target> <needle> — poll pane content (agents start async)
wait_pane() {
  local i out
  for i in {1..30}; do
    out="$(T capture-pane -p -t "$1" 2>/dev/null)"
    [[ "$out" == *"$2"* ]] && { print -r -- "$out"; return 0; }
    sleep 0.1
  done
  print -r -- "$out"
}

# wait_until <cmd...> — poll until the command succeeds (max WS_TEST_WAIT s, default 10)
wait_until() {
  local i
  for i in {1..$(( ${WS_TEST_WAIT:-10} * 10 ))}; do "$@" && return 0; sleep 0.1; done
  return 1
}

# Read the context argument recorded by the fake executable, preserving argv boundaries.
# assert_agent_preamble <agent> <id> [spawned] — only spawned sessions are told to run `ws done`.
assert_agent_preamble() {
  local agent="$1" id="$2" spawned="${3:-}" actual="" flag expected
  local argv="$WS_HOME/argv-$id.json"
  wait_until test -s "$argv"
  if [[ "$agent" == codex ]]; then
    actual="$(jq -r '.[] | select(startswith("developer_instructions=")) | ltrimstr("developer_instructions=") | fromjson' "$argv")"
    assert_eq "$(jq -r '. as $argv | map(startswith("developer_instructions=")) | index(true) as $i | if $i == null then "" else $argv[$i - 1] end' "$argv")" \
      "-c" "codex preamble is a config override"
  else
    flag=--append-system-prompt
    [[ "$agent" != grok ]] || flag=--rules
    actual="$(jq -r --arg flag "$flag" 'index($flag) as $i | if $i == null then "" else .[$i + 1] end' "$argv")"
  fi
  expected="You are ws session #$id. Use the ws-sessions skill to coordinate sessions."
  [[ -z "$spawned" ]] ||
    expected="You are ws session #$id; run \`ws done\` when the user's task is complete. Use the ws-sessions skill to coordinate sessions."
  assert_eq "$actual" "$expected" "$agent session #$id receives its identity${spawned:+ and completion instruction} as context"
}

# Playback is detached, so wait for the fake afplay before reading what it played.
afplay_out() { wait_until test -s "$FAKE_AFPLAY_OUT"; cat "$FAKE_AFPLAY_OUT" 2>/dev/null; }
afplay_log() { wait_until test -s "$FAKE_AFPLAY_LOG"; cat "$FAKE_AFPLAY_LOG" 2>/dev/null; }

start_socket_listener() {
  /usr/bin/nc -l -U "$1" > "$2" 2> "$TMP/listener.err" &
  socket_listener_pid=$!
  wait_until test -S "$1"
  assert_exit "$?" "0" "fake notch socket is listening"
}

stop_socket_listener() {
  kill "$socket_listener_pid" 2>/dev/null
  wait "$socket_listener_pid" 2>/dev/null
  socket_listener_pid=""
}

windows_of() { T list-windows -a -F '#{window_id} #{@ws_path}' | awk -v p="$1" '$2 == p { print $1 }'; }
# slot_window <worktree> <slot> — window id of that agent session
slot_window() { T list-windows -a -F '#{window_id} #{@ws_path} #{@ws_slot}' | awk -v p="$1" -v s="$2" '$2 == p && $3 == s { print $1 }'; }
session_of() { T display -p -t "$1" '#{session_name}'; }
hook() { WS_WORKSPACE="$1" TMUX_PANE="$(T display -p -t "$2" '#{pane_id}')" "$WS" hook "$3"; }
