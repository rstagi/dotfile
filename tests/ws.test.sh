#!/bin/zsh
# ws.sh — terminal workspace manager (worktree + one tmux window per agent session). Runs
# against a private tmux server (-L) with fake agent bins, so it never touches the user's.
set -u
# Every ws tab exports WS_*: a suite launched from one must not inherit its identity.
for inherited in ${(k)parameters}; do
  [[ "$inherited" != WS_* ]] || unset "$inherited"
done
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd -P)" # physical: ws.sh records ${0:A}
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
# Private HOME + agent config roots (also for subshells overriding GROK_HOME/WS_HOME after
# `source "$WS"`); the EXIT trap fails the suite if real agent hooks/config changed.
source "$HERE/home-guard.sh"
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
WS="$ROOT/ws.sh"
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

echo "ws skill: installer exposes the real session skill to Claude and Codex"
HOME="$TMP/skillhome" bash "$ROOT/install-agent-skills.sh" --no-ratel > "$TMP/skill-install.log" 2>&1
assert_exit "$?" "0" "shared skill installer succeeds in a private home"
for host in .claude .agents .codex; do
  assert_eq "$([[ "$TMP/skillhome/$host/skills/ws-sessions/SKILL.md" -ef "$ROOT/.claude/skills/ws-sessions/SKILL.md" ]] && print yes)" \
    "yes" "$host discovers the canonical ws-sessions skill"
done

REPO="$(make_repo api)"
print 'SECRET=1' > "$REPO/.env"

echo "ws new: creates worktree, copies env, opens a tmux session w/ one agent tab"
# An existing tmux server may predate the caller's WS_HOME override.
T set-environment -g WS_HOME "$TMP/stale-wshome"
"$WS" new --repo "$REPO" --branch feat/login --detach
assert_exit "$?" "0" "new succeeds"
T set-environment -g WS_HOME "$WS_HOME"
WT="$WS_ROOT/api/feat-login"
assert_eq "$(git -C "$WT" branch --show-current)" "feat/login" "worktree on new branch"
assert_eq "$(cat "$WT/.env" 2>/dev/null)" "SECRET=1" ".env copied from main checkout"
WIN="$(windows_of "$WT")"
assert_eq "${WIN:+found}" "found" "window tagged with worktree path"
assert_eq "$(session_of "$WIN")" "api/feat-login" "own tmux session named repo/branch"
assert_eq "$(T display -p -t "$WIN" '#{window_name}')" "claude" "agent tab named after agent"
assert_eq "$(T list-windows -t "$WIN" | wc -l | tr -d ' ')" "1" "session holds just the agent tab"
assert_eq "$(T list-panes -t "$WIN" | wc -l | tr -d ' ')" "1" "agent pane only"
out="$(wait_pane "$WIN" FAKE-claude)"
assert_contains "$out" "--dangerously-skip-permissions" "claude skips permissions"
assert_contains "$out" "--settings" "claude launched w/ ws hook settings"
assert_contains "$out" "WS=api/feat-login" "agent sees WS_WORKSPACE"

echo "ws sessions: new tabs have a global identity"
assert_contains "$out" "ID=1" "first agent gets global id in its environment"
assert_contains "$out" "WHOAMI=1" "agent can query its own id"
assert_agent_preamble claude 1
assert_contains "$("$WS" sessions)" $'1\t\tapi/feat-login\tclaude\tidle\tactive' "registry lists identity, workspace, agent, state and status"
env -u WS_SESSION "$WS" whoami 2>/dev/null
assert_exit "$?" "1" "whoami refuses outside a tracked session"

echo "ws name: labels address a session globally"
WS_SESSION=1 "$WS" name reviewer
assert_exit "$?" "0" "session can set its name"
assert_eq "$("$WS" sessions reviewer)" $'1\treviewer\tapi/feat-login\tclaude\tidle\tactive' "name resolves to the same registry entry"
"$WS" open reviewer --detach
assert_exit "$?" "0" "open accepts a session name"
"$WS" open 1 --detach
assert_exit "$?" "0" "open accepts a global id"
assert_eq "$(windows_of "$WT" | wc -l | tr -d ' ')" "1" "opening a live session focuses its existing tab"
WS_SESSION=1 "$WS" name 123 2>/dev/null
assert_exit "$?" "1" "numeric names cannot shadow ids"
assert_contains "$("$WS" sessions 1)" $'1\treviewer\t' "invalid rename preserves existing name"

echo "ws sessions: concurrent workspaces allocate distinct monotone ids"
global_pids=()
for i in {1..8}; do
  git -C "$REPO" worktree add -q -b "global-$i" "$WS_ROOT/api/global-$i" main
done
for i in {1..8}; do
  "$WS" add "api/global-$i" --detach > "$TMP/global-$i.log" 2>&1 &
  global_pids+=($!)
done
for pid in $global_pids; do
  wait "$pid"
  launch_exit=$?
  (( launch_exit == 0 )) || cat "$TMP"/global-*.log
  assert_exit "$launch_exit" "0" "concurrent workspace launches successfully"
done
assert_eq "$("$WS" sessions | awk -F '\t' '$3 ~ /^api\/global-/ {print $1}' | sort -n | tr '\n' ' ')" \
  "2 3 4 5 6 7 8 9 " "parallel allocation never duplicates or loses an id"
for i in {1..8}; do
  allocated_id="$("$WS" sessions | awk -F '\t' -v name="api/global-$i" '$3 == name {print $1}')"
  assert_contains "$(wait_pane "$(windows_of "$WS_ROOT/api/global-$i")" FAKE-claude)" \
    "WS=api/global-$i ID=$allocated_id WHOAMI=$allocated_id" "concurrent tab owns the identity of its workspace"
done
GLOBAL_SESSION_ID="$("$WS" sessions | awk -F '\t' '$3 == "api/global-1" {print $1}')"
GW="$(windows_of "$WS_ROOT/api/global-1")"
assert_contains "$(wait_pane "$GW" FAKE-claude)" "ID=$GLOBAL_SESSION_ID WHOAMI=$GLOBAL_SESSION_ID" "concurrent tab gets its own allocated identity"
WS_SESSION="$GLOBAL_SESSION_ID" "$WS" name reviewer 2>/dev/null
assert_exit "$?" "1" "duplicate session names are refused"
T kill-window -t "$GW"
assert_contains "$("$WS" sessions "$GLOBAL_SESSION_ID")" $'\tstopped\tactive' "killed session is stopped and remains active"
open_pids=()
for i in {1..16}; do
  "$WS" open "$GLOBAL_SESSION_ID" --detach > "$TMP/open-$i.log" 2>&1 &
  open_pids+=($!)
done
for pid in $open_pids; do
  wait "$pid"
  assert_exit "$?" "0" "concurrent resume succeeds"
done
for i in {1..16}; do
  assert_eq "$(cat "$TMP/open-$i.log")" "" "concurrent resume completes without errors"
done
assert_eq "$(windows_of "$WS_ROOT/api/global-1" | wc -l | tr -d ' ')" "1" "concurrent resume opens exactly one tab for a global id"
T kill-window -t "$(windows_of "$WS_ROOT/api/global-1")"
"$WS" open api/global-1 --detach
out="$(wait_pane "$(windows_of "$WS_ROOT/api/global-1")" FAKE-claude)"
assert_contains "$out" "ID=$GLOBAL_SESSION_ID WHOAMI=$GLOBAL_SESSION_ID" "workspace resume preserves global id without an agent resume id"
assert_eq "${out/*--resume*/resumed}" "$out" "global id is never passed as an agent resume id"
T send-keys -t "$(windows_of "$WS_ROOT/api/global-1")" Enter
registry_closed() { [[ "$("$WS" sessions "$1" | cut -f5,6)" == $'stopped\tclosed' ]]; }
wait_until registry_closed "$GLOBAL_SESSION_ID"
assert_exit "$?" "0" "clean exit closes the global record without deleting its identity"
"$WS" add api/global-1 --agent codex --detach
assert_contains "$(wait_pane "$(slot_window "$WS_ROOT/api/global-1" 1)" FAKE-codex)" "ID=10 WHOAMI=10" "reusing a workspace slot allocates a fresh global id"
"$WS" open "$GLOBAL_SESSION_ID" --detach
assert_exit "$?" "0" "closed global session can be reopened by id"
assert_contains "$(wait_pane "$(slot_window "$WS_ROOT/api/global-1" 2)" FAKE-claude)" \
  "ID=$GLOBAL_SESSION_ID WHOAMI=$GLOBAL_SESSION_ID" "closed resume preserves identity when its old slot is occupied"
assert_contains "$("$WS" sessions "$GLOBAL_SESSION_ID")" $'\tclaude\tidle\tactive' "reopened global record is active"
for i in {1..8}; do "$WS" rm "api/global-$i" --force --delete-branch >/dev/null; done

echo "ws sessions: legacy resume records acquire one stable global identity"
LEGACY_WT="$WS_ROOT/api/legacy"
git -C "$REPO" worktree add -q -b legacy "$LEGACY_WT" main
LEGACY_SF="$(git -C "$LEGACY_WT" rev-parse --absolute-git-dir)/ws-sessions"
printf '1\tclaude\tsid-legacy\n' > "$LEGACY_SF"
"$WS" open api/legacy --detach
assert_contains "$(wait_pane "$(slot_window "$LEGACY_WT" 1)" FAKE-claude)" "--resume sid-legacy" "legacy conversation resumes"
assert_contains "$(wait_pane "$(slot_window "$LEGACY_WT" 1)" FAKE-claude)" "ID=11 WHOAMI=11" "legacy tab acquires a global identity"
T kill-window -t "$(slot_window "$LEGACY_WT" 1)"
"$WS" open api/legacy --detach
assert_contains "$(wait_pane "$(slot_window "$LEGACY_WT" 1)" FAKE-claude)" "ID=11 WHOAMI=11" "legacy migration never allocates a second identity"
"$WS" rm api/legacy --force --delete-branch >/dev/null

echo "ws hook: agent events set window state; no-op outside a workspace"
state() { T display -p -t "$WIN" '#{@ws_state}'; }
assert_eq "$(state)" "idle" "fresh window starts idle"
hook api/feat-login "$WIN" working </dev/null
assert_eq "$(state)" "working" "working event"
hook api/feat-login "$WIN" waiting </dev/null
assert_eq "$(state)" "waiting" "waiting event"
env -u WS_WORKSPACE TMUX_PANE="$(T display -p -t "$WIN" '#{pane_id}')" "$WS" hook idle </dev/null
assert_exit "$?" "0" "hook outside workspace exits 0"
assert_eq "$(state)" "waiting" "hook outside workspace is a no-op"

echo "ws hook: tab stays working while background subagents run past the main turn"
sub() { print -r -- "{\"agent_id\":\"$2\"}" | hook api/feat-login "$WIN" "$1"; }
hook api/feat-login "$WIN" working </dev/null
sub subagent-start a1; sub subagent-start a2
hook api/feat-login "$WIN" idle </dev/null
assert_eq "$(state)" "working" "main Stop w/ subagents running stays working"
sub working a1
sub subagent-stop a1
assert_eq "$(state)" "working" "still working while one subagent runs"
sub subagent-stop a2
assert_eq "$(state)" "idle" "idle once the last subagent stops after main Stop"
sub subagent-start a3
hook api/feat-login "$WIN" working </dev/null
sub subagent-stop a3
assert_eq "$(state)" "working" "foreground subagent stop keeps main working"
hook api/feat-login "$WIN" idle </dev/null
assert_eq "$(state)" "idle" "main Stop w/o subagents goes idle"
assert_contains "$(jq -c '.hooks | keys' "$WS_HOME/claude-settings.json")" "SubagentStop" "claude settings hook subagent lifecycle"
hook api/feat-login "$WIN" waiting </dev/null

echo "ws hook: early hooks (before @ws_session is tagged) use the launch WS_SESSION"
T set -wu -t "$WIN" @ws_session
print -r -- '{"message":"early message"}' >> "$WS_HOME/sessions/1/inbox"
reply="$(print '{}' | WS_SESSION=1 hook api/feat-login "$WIN" stop)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason' 2>/dev/null)" "early message" "untagged window delivers via WS_SESSION"
: > "$WS_HOME/sessions/1/inbox"
T set -w -t "$WIN" @ws_session 1
hook api/feat-login "$WIN" waiting </dev/null

echo "ws list: rows for open and window-less workspaces"
git -C "$REPO" worktree add -q -b orphan "$WS_ROOT/api/orphan"
out="$("$WS" list)"
assert_contains "$out" $'waiting\tapi/feat-login\t'"$WT"$'\tclaude\t'"$WIN" "open session row w/ state, agent, window"
assert_contains "$out" $'stopped\tapi/orphan\t' "worktree without window is stopped"

echo "ws add: second session on the same worktree; resumed by id after a restart"
"$WS" add api/feat-login --agent codex --detach
W2="$(slot_window "$WT" 2)"
assert_eq "$(session_of "$W2")" "api/feat-login" "second session is a tab in the worktree's tmux session"
assert_eq "$(T display -p -t "$W2" '#{window_name}')" "codex#2" "tab named agent#slot"
assert_contains "$("$WS" list)" $'api/feat-login#2\t' "list names extra sessions repo/branch#slot"
out="$(wait_pane "$W2" FAKE-codex)"
assert_contains "$out" "--dangerously-bypass-approvals-and-sandbox" "codex bypasses approvals"
assert_contains "$out" "hooks.Stop" "codex gets Stop hook override"
CODEX_ID="$(T show -wqv -t "$W2" @ws_session)"
assert_agent_preamble codex "$CODEX_ID"
print '{"session_id":"sid-claude-1"}' | hook api/feat-login "$WIN" idle
print '{"session_id":"sid-codex-2"}' | hook api/feat-login "$W2" idle
SF="$(git -C "$WT" rev-parse --absolute-git-dir)/ws-sessions"
for i in {1..8}; do # parallel tool calls fire PostToolUse hooks concurrently
  print '{"session_id":"sid-claude-1"}' | hook api/feat-login "$WIN" working &
  print '{"session_id":"sid-codex-2"}' | hook api/feat-login "$W2" working &
done
wait
assert_eq "$(cut -f1,3 "$SF" | tr '\t\n' ': ')" "1:sid-claude-1 2:sid-codex-2 " "concurrent hooks keep every session row"
T kill-window -t "$WIN"; T kill-window -t "$W2"
rm -f "$WS_HOME/argv-1.json" "$WS_HOME/argv-$CODEX_ID.json"
assert_eq "$(windows_of "$WT")" "" "both windows gone (simulated reboot)"
assert_contains "$("$WS" list)" $'stopped\tapi/feat-login\t' "workspace shows stopped"
"$WS" open reviewer --detach
assert_exit "$?" "0" "named stopped session resumes independently"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" FAKE-claude)" "--resume sid-claude-1" "named resume uses its saved conversation id"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" FAKE-claude)" "ID=1 WHOAMI=1" "named resume preserves global identity"
assert_eq "$(slot_window "$WT" 2)" "" "resuming one id leaves other sessions stopped"
T kill-window -t "$(slot_window "$WT" 1)"
rm -f "$WS_HOME/argv-1.json"
"$WS" open api/feat-login --detach
wait_until test -n "$(slot_window "$WT" 2)"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" sid-claude-1)" "--resume sid-claude-1" "claude session resumed by id"
assert_contains "$(wait_pane "$(slot_window "$WT" 2)" sid-codex-2)" "resume sid-codex-2" "codex session resumed by id"
assert_agent_preamble claude 1
assert_agent_preamble codex "$CODEX_ID"
assert_eq "$(session_of "$(slot_window "$WT" 2)")" "api/feat-login" "restored into one worktree session"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" FAKE-claude)" "ID=1 WHOAMI=1" "workspace resume keeps the same named global identity"

echo "ws agent_command: prompts stay positional; codex hooks survive a spaced WS_BIN"
# agent_argv <agent> <resume> <prompt> — JSON argv of the generated launch command.
agent_argv() {
  ( source "$WS"
    WS_BIN="$TMP/ws bin/it's ws.sh" WS_HOME="$TMP/argv-home" GROK_HOME="$TMP/argv-grok"
    dump() { python3 -c 'import json,sys; json.dump(sys.argv[1:],sys.stdout)' "$@"; }
    claude() { dump "$@"; }; codex() { dump "$@"; }; grok() { dump "$@"; }
    eval "$(agent_command "$1" "$2" "$3" 9)" )
}
for a in claude codex grok; do
  assert_eq "$(agent_argv "$a" "" --help | jq -r '.[-2:] | join(" ")')" "-- --help" "$a option-like prompt follows --"
  assert_eq "$(agent_argv "$a" r1 completion | jq -r '.[-2:] | join(" ")')" "-- completion" "$a resume prompt follows --"
  assert_eq "$(agent_argv "$a" "" "" | jq -r 'index("--")')" "null" "$a without prompt has no --"
done
assert_eq "$(agent_argv codex r1 hi | jq -r '.[:2] | join(" ")')" "resume r1" "codex resume subcommand stays first"
hook_cmd="$(agent_argv codex "" "" | python3 -c '
import json, re, sys
arg = next(a for a in json.load(sys.stdin) if a.startswith("hooks.Stop="))
print(json.loads(re.search(r"command=(\"(?:[^\"\\\\]|\\\\.)*\")", arg).group(1)))')"
assert_eq "$(zsh -fc 'print -rl -- ${(Q)${(z)1}}' _ "$hook_cmd")" "$TMP/ws bin/it's ws.sh"$'\nhook\nstop' "codex hook command shell-quotes a spaced WS_BIN"
mkdir -p "$TMP/ws bin" && printf '#!/bin/sh\n' > "$TMP/ws bin/it's ws.sh" && chmod +x "$TMP/ws bin/it's ws.sh"
( source "$WS"
  WS_BIN="$TMP/ws bin/it's ws.sh" WS_HOME="$TMP/argv-home" GROK_HOOKS="$TMP/argv-grok/hooks/ws.json"
  write_claude_settings; write_grok_hooks )
for f in "$TMP/argv-home/claude-settings.json" "$TMP/argv-grok/hooks/ws.json"; do
  hook_cmd="$(jq -r '.hooks.Stop[0].hooks[0].command' "$f")"
  assert_eq "$(zsh -fc 'print -rl -- ${(Q)${(z)1}}' _ "$hook_cmd")" "$TMP/ws bin/it's ws.sh"$'\nhook\nstop' "${f:t} hook command shell-quotes a spaced WS_BIN"
done
echo "ws grok hooks: never wired to a missing/non-executable WS_BIN"
print -r -- '{"keep":true}' > "$TMP/argv-grok/hooks/keep.json"
for bad in "$TMP/ws bin/missing.sh" "$TMP/ws bin"; do
  err="$( ( source "$WS"; WS_BIN="$bad" GROK_HOOKS="$TMP/argv-grok/hooks/keep.json"; write_grok_hooks ) 2>&1 >/dev/null)"
  assert_exit "$?" "1" "grok hooks refuse WS_BIN ${bad:t}"
  assert_contains "$err" "not executable" "grok hooks explain refused WS_BIN ${bad:t}"
  assert_eq "$(cat "$TMP/argv-grok/hooks/keep.json")" '{"keep":true}' "grok hooks file untouched for WS_BIN ${bad:t}"
done

echo "ws grok: launched w/ --always-approve + global ws hook file; resumed by GROK_SESSION_ID"
"$WS" new --repo "$REPO" --branch grokky --agent grok --detach
GWT="$WS_ROOT/api/grokky"
GW="$(slot_window "$GWT" 1)"
assert_eq "$(T display -p -t "$GW" '#{window_name}')" "grok" "grok tab named after agent"
assert_contains "$(wait_pane "$GW" FAKE-grok)" "--always-approve" "grok skips approvals"
GROK_ID="$(T show -wqv -t "$GW" @ws_session)"
assert_agent_preamble grok "$GROK_ID"
GH="$GROK_HOME/hooks/ws.json"
assert_eq "$(jq -r '.hooks.StopCancelled[0].hooks[0].command' "$GH" 2>/dev/null)" "$WS hook cancel" "grok hook file: StopCancelled → turn-checked cancel"
assert_eq "$(jq -r '.hooks.Notification[0].matcher' "$GH" 2>/dev/null)" "permission_prompt" "grok hook file: permission prompt → waiting"
"$WS" add api/grokky --agent grok --detach
GW2="$(slot_window "$GWT" 2)"
assert_eq "$(T display -p -t "$GW2" '#{window_name}')" "grok#2" "second grok tab named grok#2"
GROK_SESSION_ID=sid-grok-2 hook api/grokky "$GW2" idle </dev/null
GROK_RESUME_ID="$(T show -wqv -t "$GW2" @ws_session)"
T kill-window -t "$GW"; T kill-window -t "$GW2"
rm -f "$WS_HOME/argv-$GROK_RESUME_ID.json"
"$WS" open api/grokky --detach
wait_until test -n "$(slot_window "$GWT" 2)"
assert_contains "$(wait_pane "$(slot_window "$GWT" 2)" sid-grok-2)" "--resume sid-grok-2" "grok resumed by GROK_SESSION_ID"
assert_agent_preamble grok "$GROK_RESUME_ID"
"$WS" add api/grokky --agent gemini --detach 2>/dev/null
assert_exit "$?" "1" "unknown agent refused"
"$WS" rm api/grokky --force --delete-branch

echo "ws add: defaults to the worktree you're in"
before="$(windows_of "$WT" | wc -l | tr -d ' ')"
(cd "$WT" && "$WS" add --agent codex --detach)
assert_eq "$(windows_of "$WT" | wc -l | tr -d ' ')" "$((before + 1))" "add w/o target uses current worktree"
T kill-window -t "$(slot_window "$WT" 3)"
session_forget() { local f="$(git -C "$WT" rev-parse --absolute-git-dir)/ws-sessions"; awk -F '\t' '$1 != 3' "$f" > "$f.tmp" && mv "$f.tmp" "$f"; }
session_forget

echo "ws: quitting an agent cleanly forgets its session"
W2="$(slot_window "$WT" 2)"
T send-keys -t "$W2" Enter
wait_until test -z "$(slot_window "$WT" 2)"
assert_eq "$(slot_window "$WT" 2)" "" "window closes when agent exits"
T kill-window -t "$(slot_window "$WT" 1)"
assert_eq "$(T has-session -t '=api/feat-login' 2>/dev/null && print alive)" "" "worktree session gone with its last tab"
"$WS" open api/feat-login --detach
sleep 0.3
assert_eq "$(windows_of "$WT" | wc -l | tr -d ' ')" "1" "only the surviving session is restored"

echo "ws pin: pinned workspaces listed first in the picker, grouped by repo"
REPO2="$(make_repo web)"
"$WS" new --repo "$REPO2" --branch zeta --detach
"$WS" pin api/orphan
hook api/feat-login "$(windows_of "$WT")" waiting </dev/null # waiting > idle: must not reorder repo groups
assert_contains "$("$WS" list)" $'stopped\tapi/orphan\t'"$WS_ROOT/api/orphan"$'\t-\t\t1' "list marks pinned"
rows="$("$WS" _rows)"
display="$(print -r -- "$rows" | cut -f1 | sed $'s/\e\\[[0-9;]*m//g')"
assert_eq "$(print -r -- "$rows" | cut -f2 | grep -c '^$')" "0" "every row is a selectable workspace (no header rows)"
assert_eq "$(print -r -- "$display" | grep -n 'orphan' | cut -d: -f1)" "1" "pinned workspace first"
assert_contains "$(print -r -- "$display" | sed -n 1p)" "📌" "pinned row marked"
assert_eq "$(print -r -- "$display" | grep -c orphan)" "1" "pinned workspace not repeated in its repo group"
assert_eq "$(print -r -- "$rows" | cut -f2 | sed -n '2,$p' | sed 's|.*/worktrees/||; s|/.*||' | tr '\n' ' ')" "api web " "rest grouped by repo, sorted"
"$WS" pin api/orphan
assert_eq "$("$WS" _rows | cut -f1 | grep -c '📌')" "0" "pin toggles off"

echo "ws pick: collapsed by default; ctrl-s toggles per-session rows; global summary"
"$WS" add web/zeta --agent codex --detach
ZWT="$WS_ROOT/web/zeta"
hook web/zeta "$(slot_window "$ZWT" 2)" working </dev/null
assert_eq "$("$WS" _rows | grep -cF "$ZWT")" "1" "collapsed: one row per worktree"
assert_contains "$("$WS" _summary | sed $'s/\e\\[[0-9;]*m//g')" "1 working" "summary counts working sessions"
assert_contains "$("$WS" _summary | sed $'s/\e\\[[0-9;]*m//g')" "idle" "summary counts idle sessions"
"$WS" _toggle_sessions
zrows="$("$WS" _rows | grep -F "$ZWT")"
zdisp="$(print -r -- "$zrows" | cut -f1 | sed $'s/\e\\[[0-9;]*m//g')"
assert_eq "$(print -r -- "$zrows" | wc -l | tr -d ' ')" "3" "worktree row + one row per session"
assert_contains "$(print -r -- "$zdisp" | sed -n 1p)" "2 sessions" "worktree row summarizes session count"
assert_contains "$(print -r -- "$zdisp" | sed -n 1p)" "◐" "worktree row shows most urgent state"
assert_contains "$(print -r -- "$zdisp" | grep 'codex#2')" "◐" "working session row shows working"
assert_contains "$(print -r -- "$zdisp" | grep -v 'codex#2' | sed -n 2p)" "○ claude" "idle session row shows idle"
assert_eq "$(print -r -- "$zrows" | grep 'codex#2' | cut -f3)" "$(slot_window "$ZWT" 2)" "session row targets its own tab"
assert_eq "$("$WS" _rows | grep -F "$WS_ROOT/api/orphan" | wc -l | tr -d ' ')" "1" "single-session/stopped worktree: no sub-rows"
"$WS" _toggle_sessions
assert_eq "$("$WS" _rows | grep -cF "$ZWT")" "1" "toggling again collapses"
T kill-window -t "$(slot_window "$ZWT" 2)"

echo "ws pick: first row is actionable (ctrl-a adds a session to it)"
cat > "$TMP/bin/fzf" <<'EOF'
#!/bin/sh
printf 'ctrl-a\n'; head -1
EOF
chmod +x "$TMP/bin/fzf"
first="$("$WS" _rows | head -1 | cut -f2)"
before="$(windows_of "$first" | wc -l | tr -d ' ')"
TMUX=fake "$WS" pick 2>/dev/null
assert_eq "$(windows_of "$first" | wc -l | tr -d ' ')" "$((before + 1))" "ctrl-a on the first row adds a session"
sed -i '' 's/ctrl-a/ctrl-t/' "$TMP/bin/fzf"
TMUX=fake "$WS" pick 2>/dev/null
assert_eq "$("$WS" list | awk -F '\t' -v p="$first" '$3 == p && $4 == "codex"' | wc -l | tr -d ' ')" "1" "ctrl-t on the first row adds a codex session"
sed -i '' 's/ctrl-t/ctrl-v/' "$TMP/bin/fzf"
TMUX=fake "$WS" pick 2>/dev/null
assert_eq "$("$WS" list | awk -F '\t' -v p="$first" '$3 == p && $4 == "grok"' | wc -l | tr -d ' ')" "1" "ctrl-v on the first row adds a grok session"

echo "ws open / pick: a running workspace opens on its last-used tab"
"$WS" add web/zeta --agent codex --detach
LW="$(slot_window "$ZWT" 3)"
T select-window -t "$LW"
TMUX=fake "$WS" open web/zeta 2>/dev/null
assert_eq "$(T display -p -t "$LW" '#{window_active}')" "1" "open keeps the last-used tab"
assert_eq "$("$WS" _rows | grep -F "$ZWT" | head -1 | cut -f3)" "$LW" "picker row targets the last-used tab"
T select-window -t "$(slot_window "$ZWT" 1)"
assert_eq "$("$WS" _rows | grep -F "$ZWT" | head -1 | cut -f3)" "$(slot_window "$ZWT" 1)" "follows the tab you switch to"
T kill-window -t "$LW"
session_forget_slot() { local f="$(git -C "$1" rev-parse --absolute-git-dir)/ws-sessions"; awk -F '\t' -v s="$2" '$1 != s' "$f" > "$f.tmp" && mv "$f.tmp" "$f"; }
session_forget_slot "$ZWT" 3

echo "ws pick: cursor starts on the current tab's row"
cat > "$TMP/bin/fzf" <<'EOF'
#!/bin/sh
cat > "$FAKE_FZF_IN"; printf '%s\n' "$@" > "$FAKE_FZF_ARGS"; exit 1
EOF
chmod +x "$TMP/bin/fzf"
export FAKE_FZF_IN="$TMP/fzf.in" FAKE_FZF_ARGS="$TMP/fzf.args"
start_pos() { grep '^start:' "$FAKE_FZF_ARGS" | sed 's/.*pos(\([0-9]*\)).*/\1/'; }
TMUX=fake TMUX_PANE="$(T display -p -t "$(slot_window "$ZWT" 1)" '#{pane_id}')" "$WS" pick
assert_eq "$(sed -n "$(start_pos)p" "$FAKE_FZF_IN" | cut -f2)" "$ZWT" "starts on the current workspace"
(cd "$TMP" && env -u TMUX -u TMUX_PANE WS_TMUX_SOCKET=none "$WS" pick 2>/dev/null)
assert_eq "$(start_pos)" "1" "outside tmux: first row"
unset FAKE_FZF_IN FAKE_FZF_ARGS

echo "ws unread: a tab that finishes (working → idle) unseen stays unread until visited; ctrl-u toggles"
unread() { T show -wqv -t "$1" @ws_unread; }
UW="$(slot_window "$ZWT" 1)"
hook web/zeta "$UW" working </dev/null
assert_eq "$(unread "$UW")" "" "working tab is not unread"
hook web/zeta "$UW" idle </dev/null
assert_eq "$(unread "$UW")" "1" "finished unseen → unread"
hook web/zeta "$UW" idle </dev/null
assert_eq "$(unread "$UW")" "1" "repeated idle keeps it unread"
assert_eq "$("$WS" list | awk -F '\t' -v p="$ZWT" '$3 == p { print $8 }')" "1" "list exposes unread"
assert_contains "$("$WS" _rows | grep -F "$ZWT" | cut -f1 | sed $'s/\e\\[[0-9;]*m//g')" "• zeta" "picker row marked unread"
assert_contains "$("$WS" _summary | sed $'s/\e\\[[0-9;]*m//g')" "1 unread" "summary counts unread"
"$WS" add web/zeta --agent codex --detach
UW2="$(slot_window "$ZWT" 3)"
T select-window -t "$UW2"
assert_eq "$(unread "$UW")" "1" "switching away keeps it unread"
assert_eq "$("$WS" _rows | grep -F "$ZWT" | head -1 | cut -f3)" "$UW" "workspace row targets the unread tab"
T select-window -t "$UW"
assert_eq "$(unread "$UW")" "" "visiting the tab marks it read"
"$WS" _unread "$UW"
assert_eq "$(unread "$UW")" "1" "_unread marks it unread again"
"$WS" _unread "$UW"
assert_eq "$(unread "$UW")" "" "_unread toggles back to read"
"$WS" _unread "$UW"
TMUX=fake "$WS" open web/zeta 2>/dev/null
assert_eq "$(unread "$UW")" "" "opening from ws marks it read"
T kill-window -t "$UW2"
session_forget_slot "$ZWT" 3

echo "ws claude: adopts a hand-opened tab as a tracked session; plain shell again after"
MW="$(T new-window -d -P -F '#{window_id}' -t "$(session_of "$(slot_window "$ZWT" 1)"):" -c "$ZWT" \
  "zsh -fc '\"$WS\" claude --model x; echo EXIT=\$?; sleep 30'")"
out="$(wait_pane "$MW" FAKE-claude)"
assert_contains "$out" "--settings" "agent launched w/ ws hooks"
assert_contains "$out" "--model x" "extra args passed through"
assert_contains "$out" "WS=web/zeta" "agent sees WS_WORKSPACE"
assert_agent_preamble claude "$(T show -wqv -t "$MW" @ws_session)"
assert_eq "$(T display -p -t "$MW" '#{@ws_path}')" "$ZWT" "tab tagged as a workspace session"
assert_eq "$(T display -p -t "$MW" '#{window_name}')" "claude#3" "tab named agent#slot"
assert_contains "$("$WS" list)" $'web/zeta#3\t' "listed as a session"
T send-keys -t "$MW" Enter
wait_pane "$MW" "EXIT=0" >/dev/null
assert_eq "$(T show -wqv -t "$MW" @ws_slot)" "" "untagged after the agent exits"
assert_eq "$(cut -f1 "$(git -C "$ZWT" rev-parse --absolute-git-dir)/ws-sessions" | grep -c '^3$')" "0" "clean exit forgets the session"
T kill-window -t "$MW"
(cd "$TMP" && TMUX_PANE=%0 env -u TMUX "$WS" claude 2>/dev/null)
assert_exit "$?" "1" "refused outside a workspace"

echo "ws pick: selecting a stopped workspace restores it"
# Fake fzf: empty --expect key line, then the row matching FAKE_FZF_PICK.
cat > "$TMP/bin/fzf" <<'EOF'
#!/bin/sh
printf '\n'; grep -- "$FAKE_FZF_PICK" | head -1
EOF
chmod +x "$TMP/bin/fzf"
FAKE_FZF_PICK=orphan TMUX=fake "$WS" pick 2>"$TMP/pick.err"
# (stderr also holds "no current client": the fake TMUX has no client to switch)
assert_eq "$(grep -c 'not found' "$TMP/pick.err")" "0" "picker keeps PATH intact (fzf/awk found)"
assert_eq "$(windows_of "$WS_ROOT/api/orphan" | wc -l | tr -d ' ')" "1" "picked workspace opened"
T kill-window -t "$(windows_of "$WS_ROOT/api/orphan")"

echo "ws new: honors .conductor/settings.toml include globs + setup"
REPO3="$(make_repo shop)"
mkdir -p "$REPO3/.conductor" "$REPO3/apps/ui"
print '.flags.local.json' >> "$REPO3/.gitignore"
print 'X=1' > "$REPO3/apps/ui/.env.local"
print '{}' > "$REPO3/.flags.local.json"
cat > "$REPO3/.conductor/settings.toml" <<'TOML'
file_include_globs = """
.env*
.flags.local.json
"""

[scripts]
setup = "touch SETUP-RAN"
TOML
"$WS" new --repo "$REPO3" --branch main-ish --detach
WT3="$WS_ROOT/shop/main-ish"
assert_eq "$(cat "$WT3/apps/ui/.env.local" 2>/dev/null)" "X=1" "nested .env copied"
assert_eq "$(cat "$WT3/.flags.local.json" 2>/dev/null)" "{}" "extra conductor glob copied"
wait_until test -f "$WT3/SETUP-RAN"
assert_exit "$?" "0" "conductor setup runs"
"$WS" new --repo "$REPO3" --branch main-ish --detach 2>/dev/null
assert_exit "$?" "1" "duplicate workspace refused"

echo "ws new: repo picker lists repos found under WS_REPO_ROOTS; typed path accepted"
mkdir -p "$TMP/dev/org"
DEVREPO="$(make_repo dev/org/svc)"
mkdir -p "$TMP/dev/org/svc/node_modules/dep" && git init -q "$TMP/dev/org/svc/node_modules/dep"
export WS_REPO_ROOTS="$TMP/dev $REPO3"
# Fake fzf: records its candidates, then prints FAKE_FZF_OUT (as fzf --print-query would).
cat > "$TMP/bin/fzf" <<'EOF'
#!/bin/sh
cat > "$FAKE_FZF_IN"
printf '%s\n' "$FAKE_FZF_OUT"
EOF
chmod +x "$TMP/bin/fzf"
export FAKE_FZF_IN="$TMP/fzf.in"
FAKE_FZF_OUT="$DEVREPO" "$WS" new --branch scanned --detach
assert_contains "$(cat "$FAKE_FZF_IN")" "$DEVREPO" "nested repo under a root listed"
assert_contains "$(cat "$FAKE_FZF_IN")" "$REPO3" "root that is itself a repo listed"
assert_eq "$(grep -c node_modules "$FAKE_FZF_IN")" "0" "node_modules repos skipped"
assert_eq "$([[ -d "$WS_ROOT/svc/scanned" ]] && print yes)" "yes" "picked scanned repo"
FAKE_FZF_OUT="${REPO3/$HOME/~}" HOME="$HOME" "$WS" new --branch typed --detach
assert_eq "$([[ -d "$WS_ROOT/shop/typed" ]] && print yes)" "yes" "typed path (no list match) accepted"
FAKE_FZF_OUT="$TMP/nope" "$WS" new --branch bad --detach 2>/dev/null
assert_exit "$?" "1" "typed non-repo path refused"
unset WS_REPO_ROOTS

echo "ws hook: notifies via terminal-notifier (sound, click focuses the tab) when unfocused"
cat > "$TMP/bin/afplay" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_AFPLAY_OUT"
printf '%s\n' "$@" >> "$FAKE_AFPLAY_LOG"
[ -n "${FAKE_AFPLAY_SLEEP:-}" ] && sleep "$FAKE_AFPLAY_SLEEP" && : > "$FAKE_AFPLAY_OUT.done"
exit "${FAKE_AFPLAY_EXIT:-0}"
EOF
chmod +x "$TMP/bin/afplay"
export FAKE_AFPLAY_OUT="$TMP/afplay.out"
export FAKE_AFPLAY_LOG="$TMP/afplay.log"
cat > "$TMP/bin/terminal-notifier" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_NOTIFIER_OUT"
EOF
chmod +x "$TMP/bin/terminal-notifier"
export FAKE_NOTIFIER_OUT="$TMP/notifier.out"
NWIN="$(slot_window "$WT" 1)"
WS_NOTIFY=1 WS_TERMINAL_BUNDLE=com.example.term hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 WS_TERMINAL_BUNDLE=com.example.term hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)"
assert_contains "$nout" "api/feat-login needs input" "message names workspace + state"
assert_eq "$(afplay_out)" "$ROOT/assets/ws/sounds/waiting.wav" "waiting plays the bundled rising cue by default"
assert_eq "$([[ "$nout" == *-sound* ]] && print yes)" "" "terminal-notifier sound suppressed to avoid double playback"
assert_contains "$nout" $'-activate\ncom.example.term' "click activates the terminal app"
assert_contains "$nout" "switch-client" "click switches tmux to the tab"
assert_contains "$nout" "$NWIN" "click targets the notifying tab"
assert_contains "$nout" $'-group\nws-'"$NWIN" "one notification per tab (replaced, not stacked)"
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
: > "$TMP/icon.png"
WS_NOTIFY=1 WS_NOTIFY_ICON_DONE="$TMP/icon.png" hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)"
assert_eq "$(afplay_out)" "$ROOT/assets/ws/sounds/done.wav" "done plays the bundled soft chime by default"
assert_contains "$nout" $'-contentImage\n'"$TMP/icon.png" "done icon attached (configurable)"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT")"
assert_eq "$(afplay_out)" "$ROOT/assets/ws/sounds/waiting.wav" "waiting sound resolves independently from done"
assert_contains "$nout" $'-contentImage\n'"$ROOT/assets/ws/waiting.png" "needs input uses the bundled waiting icon"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 WS_NOTIFY_SOUND_DONE=Pop hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT")"
assert_eq "$(afplay_out)" "/System/Library/Sounds/Pop.aiff" "done sound overridable with a system sound name"
assert_contains "$nout" $'-contentImage\n'"$ROOT/assets/ws/done.png" "done uses the bundled done icon"
echo "ws hook: a relative sound file with spaces resolves from the hook cwd"
cp "$ROOT/assets/ws/sounds/done.wav" "$TMP/custom done.wav"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
(cd "$TMP" && WS_NOTIFY=1 WS_NOTIFY_SOUND_DONE="custom done.wav" hook api/feat-login "$NWIN" idle </dev/null)
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_eq "$(afplay_out)" "custom done.wav" "relative sound path is passed as one argument"
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 hook api/feat-login "$NWIN" working </dev/null
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "no notification for working"
hook api/feat-login "$NWIN" idle </dev/null

echo "ws hook: custom adapter receives one v1 JSON event on stdin"
mkdir -p "$TMP/custom adapters"
cat > "$TMP/custom adapters/capture" <<'EOF'
#!/bin/sh
cat > "$FAKE_EVENT_OUT"
EOF
chmod +x "$TMP/custom adapters/capture"
export FAKE_EVENT_OUT="$TMP/event.json"
rm -f "$FAKE_AFPLAY_OUT"
hook api/feat-login "$NWIN" working </dev/null
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$([[ -e "$FAKE_AFPLAY_OUT" ]] && print yes)" "" "custom adapter owns sound playback"
jq -e --arg id "ws:$NWIN" '
  .v == 1 and .id == $id and .source == "ws" and
  .state == "waiting" and .prev == "working" and
  .title == "ws" and .message == "api/feat-login needs input" and
  .detail == "" and .agent == "claude" and .repo == "api" and .branch == "feat/login" and
  .focused == false and .sound == "waiting" and
  (.ts | type == "number") and .ts > 0 and
  (.actions | length == 1) and .actions[0].id == "focus" and .actions[0].label == "Focus tab" and
  (.actions[0].command | contains("select-window") and contains("switch-client"))
' "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "custom adapter receives the v1 schema and workspace metadata"
assert_eq "$(wc -l < "$FAKE_EVENT_OUT" 2>/dev/null | tr -d ' ')" "1" "event is one compact JSON line"
assert_contains "$(jq -r '.actions[0].command' "$FAKE_EVENT_OUT" 2>/dev/null)" "$NWIN" "event focus action targets the tab"
rm -f "$FAKE_EVENT_OUT" "$FAKE_NOTIFIER_OUT"
SPECIAL_WORKSPACE=$'api/feat-"quoted"\\line\nlogin'
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook "$SPECIAL_WORKSPACE" "$NWIN" idle </dev/null
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.message' "$FAKE_EVENT_OUT")" "$SPECIAL_WORKSPACE is idle" "JSON preserves quotes, backslashes and newlines"
assert_eq "$(jq -r '.id, .state, .prev, .sound' "$FAKE_EVENT_OUT")" $'ws:'"$NWIN"$'\nidle\nwaiting\ndone' "same tab identity persists across states"
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "custom adapter bypasses the built-in notifier"
rm -f "$FAKE_EVENT_OUT"
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle </dev/null
sleep 0.2
assert_eq "$([[ -e "$FAKE_EVENT_OUT" ]] && print yes)" "" "repeated state does not reach a custom adapter"
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" working </dev/null
wait_until test -s "$FAKE_EVENT_OUT"
jq -e '.state == "working" and .prev == "idle" and .focused == false and .sound == ""' \
  "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "custom adapter receives working transitions without a sound cue"
rm -f "$FAKE_EVENT_OUT"
WS_NOTIFY=0 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting </dev/null
sleep 0.2
assert_eq "$([[ -e "$FAKE_EVENT_OUT" ]] && print yes)" "" "WS_NOTIFY=0 disables custom adapters too"

echo "ws hook: events carry a hook-entry seq; stale or concurrent duplicate hooks are dropped"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_EVENT_OUT"
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" WS_TERMINAL_BUNDLE=com.example.term \
  hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_EVENT_OUT"
jq -e '(.seq | type == "number") and .seq == (.seq | floor) and .seq > 1000000000000000' \
  "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "event seq is integer microseconds since epoch"
assert_eq "$(T display -p -t "$NWIN" '#{@ws_seq}')" "$(jq -r '.seq' "$FAKE_EVENT_OUT" 2>/dev/null)" "window stores last applied seq"
assert_contains "$(jq -r '.actions[0].command' "$FAKE_EVENT_OUT" 2>/dev/null)" "open -b com.example.term" "focus action activates the terminal app"
rm -f "$FAKE_EVENT_OUT"
T set -w -t "$NWIN" @ws_seq 99999999999999999
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting </dev/null
sleep 0.2
assert_eq "$(T display -p -t "$NWIN" '#{@ws_state}')" "idle" "stale hook does not change state"
assert_eq "$([[ -e "$FAKE_EVENT_OUT" ]] && print yes)" "" "stale hook emits no event"
T set -wu -t "$NWIN" @ws_seq
cat > "$TMP/custom adapters/append" <<'EOF'
#!/bin/sh
cat >> "$FAKE_EVENT_LOG"
EOF
chmod +x "$TMP/custom adapters/append"
export FAKE_EVENT_LOG="$TMP/events.log"
rm -f "$FAKE_EVENT_LOG"
for i in 1 2 3 4 5 6; do
  WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/append" hook api/feat-login "$NWIN" waiting </dev/null &
done
wait
wait_until test -s "$FAKE_EVENT_LOG"
sleep 0.3
assert_eq "$(wc -l < "$FAKE_EVENT_LOG" 2>/dev/null | tr -d ' ')" "1" "simultaneous identical hooks emit exactly one event"
assert_eq "$(T display -p -t "$NWIN" '#{@ws_state}')" "waiting" "simultaneous hooks apply the state"
hook api/feat-login "$NWIN" idle </dev/null

echo "ws hook: seq is one realtime snapshot (no second-boundary skew)"
ws_fn() { ( source "$WS" && "$@" ) }
assert_eq "$(ws_fn realtime_us 1791187219.9999999)" "1791187219999999" "seq truncates to microseconds"
assert_eq "$(ws_fn realtime_us 1791187220.0000001)" "1791187220000000" "seq after the second boundary"
assert_eq "$(ws_fn realtime_us 1791187220.5)" "1791187220500000" "short fraction right-padded"

echo "ws hook: seq is captured at entry, before reading the payload"
hook api/feat-login "$NWIN" working </dev/null
mkfifo "$TMP/slow.in"
exec {slow_w}<>"$TMP/slow.in"
rm -f "$FAKE_EVENT_LOG"
# Children must not inherit the writer, or the slow hook never sees EOF.
(exec {slow_w}>&-; WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/append" hook api/feat-login "$NWIN" waiting <"$TMP/slow.in") &
slow_pid=$!
sleep 0.5 # older hook has taken its seq and blocks in payload read
(exec {slow_w}>&-; WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/append" hook api/feat-login "$NWIN" idle </dev/null)
exec {slow_w}>&-
wait "$slow_pid"
wait_until test -s "$FAKE_EVENT_LOG"
sleep 0.3
assert_eq "$(T display -p -t "$NWIN" '#{@ws_state}')" "idle" "newer hook wins over an older slow-payload hook"
assert_eq "$(jq -r '.state' "$FAKE_EVENT_LOG" 2>/dev/null)" "idle" "only the newer hook emits"

echo "ws hook: fails closed when the tab lock can't be taken"
hook api/feat-login "$NWIN" working </dev/null
HOOK_LOCK="${TMPDIR:-/tmp}/ws-hook-$WS_TMUX_SOCKET-$NWIN.lock"
rm -f "$TMP/locked" "$FAKE_EVENT_OUT"
zsh -fc 'zmodload zsh/system; zsystem flock -f fd "$1" && : > "$2" && sleep 4' _ "$HOOK_LOCK" "$TMP/locked" &
holder_pid=$!
wait_until test -e "$TMP/locked"
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle </dev/null
sleep 0.2
assert_eq "$(T display -p -t "$NWIN" '#{@ws_state}')" "working" "unlocked hook leaves state untouched"
assert_eq "$([[ -e "$FAKE_EVENT_OUT" ]] && print yes)" "" "unlocked hook emits nothing"
kill "$holder_pid" 2>/dev/null
wait "$holder_pid" 2>/dev/null
rm -f "$FAKE_EVENT_OUT"
hook api/feat-login "$NWIN" idle </dev/null

echo "ws notify: built-in adapter delivers per tab in seq order"
notify_event() {
  jq -nc --arg id "ws:$NWIN" --arg state "$1" --argjson seq "$2" \
    '{v:1, id:$id, source:"ws", state:$state, prev:"", title:"ws", message:("tab is " + $state),
      detail:"", focused:false, sound:"", actions:[{id:"focus", label:"Focus tab", command:"true"}], seq:$seq}'
}
deliver() { notify_event "$1" "$2" | WS_NOTIFIER=terminal-notifier ws_fn notify_adapter; }
NSEQ="$(ws_fn realtime_us "$(zsh -fc 'zmodload zsh/datetime; print $EPOCHREALTIME')")"
rm -f "$FAKE_NOTIFIER_OUT"
deliver waiting $(( NSEQ + 2 ))
assert_contains "$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "tab is waiting" "newer event posted"
rm -f "$FAKE_NOTIFIER_OUT"
deliver idle $(( NSEQ + 1 ))
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "older event arriving late is dropped"
deliver working $(( NSEQ + 4 ))
deliver idle $(( NSEQ + 3 ))
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "newer working (no banner) still supersedes an older idle"
deliver idle $(( NSEQ + 5 ))
assert_contains "$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "tab is idle" "next newer event posted"
rm -f "$FAKE_NOTIFIER_OUT"
deliver closed $(( NSEQ + 6 ))
nout="$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)"
assert_contains "$nout" "-remove" "closed removes the banner"
assert_contains "$nout" "ws-$NWIN" "closed targets the tab's banner group"
assert_eq "$([[ "$nout" == *-message* ]] && print yes)" "" "closed posts no new banner"
T set -wu -t "$NWIN" @ws_notify_seq
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"

echo "ws hook: Claude Notification includes its message as detail"
DETAIL=$'Approve "deploy"?\\line\nWaiting…'
hook api/feat-login "$NWIN" working </dev/null
jq -nc --arg message "$DETAIL" '{hook_event_name:"Notification", notification_type:"permission_prompt", message:$message}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.detail' "$FAKE_EVENT_OUT")" "$DETAIL" "Notification detail preserves quotes, backslashes, newlines and Unicode"

echo "ws hook: Claude Stop includes the last assistant message"
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"hook_event_name":"Stop","last_assistant_message":"Fixed the login flow."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.detail' "$FAKE_EVENT_OUT")" "Fixed the login flow." "Stop detail is the last assistant message"

echo "ws hook: Grok Stop camelCase lastAssistantMessage is detail"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"hookEventName":"Stop","lastAssistantMessage":"Grok finished."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.detail' "$FAKE_EVENT_OUT")" "Grok finished." "Grok Stop detail is lastAssistantMessage"

echo "ws hook: UserPromptSubmit includes the user's prompt"
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"hook_event_name":"UserPromptSubmit","prompt":"Fix login for guest users."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" working
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.detail' "$FAKE_EVENT_OUT")" "Fix login for guest users." "working detail is the submitted prompt"

echo "ws hook: detail is bounded to 1024 Unicode characters"
rm -f "$FAKE_EVENT_OUT"
jq -nc '{hook_event_name:"Stop", last_assistant_message:("Header: " + ("é" * 1016) + "TAIL")}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle
wait_until test -s "$FAKE_EVENT_OUT"
jq -e '(.detail | length) == 1024 and (.detail | startswith("Header: ") and endswith("é") and (contains("TAIL") | not))' \
  "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "detail truncates by characters without corrupting Unicode"

echo "ws hook: detail preserves trailing newlines"
rm -f "$FAKE_EVENT_OUT"
DETAIL=$'Please review.\n\n'
jq -nc --arg prompt "$DETAIL" '{hook_event_name:"UserPromptSubmit", prompt:$prompt}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" working
wait_until test -s "$FAKE_EVENT_OUT"
jq -e --arg detail "$DETAIL" '.detail == $detail' "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "detail preserves the submitted text's trailing newlines"

echo "ws hook: malformed or unavailable detail never drops the transition"
for payload in '{}' '{' '[]' 'null' '{} {}' '{"message":42,"last_assistant_message":[],"prompt":false}'; do
  hook api/feat-login "$NWIN" working </dev/null
  rm -f "$FAKE_EVENT_OUT"
  print -r -- "$payload" | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle
  assert_exit "$?" "0" "hook accepts unavailable detail: $payload"
  wait_until test -s "$FAKE_EVENT_OUT"
  jq -e '.state == "idle" and .prev == "working" and .detail == ""' "$FAKE_EVENT_OUT" >/dev/null 2>&1
  assert_exit "$?" "0" "transition survives unavailable detail: $payload"
done

echo "ws hook: non-text fields do not hide an available assistant message"
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"message":{},"last_assistant_message":"Available text.","prompt":"Lower-priority text."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.detail' "$FAKE_EVENT_OUT")" "Available text." "detail selects the first available text field"

echo "ws hook: Codex Stop and UserPromptSubmit include detail and tab metadata"
"$WS" add api/feat-login --agent codex --detach
CWIN="$("$WS" list | awk -F '\t' -v p="$WT" '$3 == p && $4 == "codex" { print $5; exit }')"
hook api/feat-login "$CWIN" working </dev/null
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"session_id":"sid-codex-details","turn_id":"turn-1","hook_event_name":"Stop","transcript_path":null,"last_assistant_message":"Codex finished."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$CWIN" idle
wait_until test -s "$FAKE_EVENT_OUT"
jq -e --arg id "ws:$CWIN" '
  .id == $id and .agent == "codex" and .repo == "api" and .branch == "feat/login" and
  .state == "idle" and .prev == "working" and .focused == false and .detail == "Codex finished."
' "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "Codex Stop detail belongs to the Codex tab"
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"hook_event_name":"UserPromptSubmit","prompt":"Continue the Codex task."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$CWIN" working
wait_until test -s "$FAKE_EVENT_OUT"
jq -e '.agent == "codex" and .state == "working" and .detail == "Continue the Codex task."' "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "Codex working detail is the submitted prompt"
hook api/feat-login "$NWIN" waiting </dev/null

echo "ws hook: explicit osascript selection bypasses terminal-notifier"
cat > "$TMP/bin/osascript" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_OSASCRIPT_OUT"
EOF
chmod +x "$TMP/bin/osascript"
export FAKE_OSASCRIPT_OUT="$TMP/osascript.out"
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 WS_NOTIFIER=osascript hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_contains "$(cat "$FAKE_OSASCRIPT_OUT")" "api/feat-login is idle" "osascript receives the event message"
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "explicit osascript ignores an installed terminal-notifier"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 WS_NOTIFIER=terminal-notifier hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_eq "$(afplay_out)" "$ROOT/assets/ws/sounds/waiting.wav" "explicit terminal-notifier plays the bundled waiting sound"
assert_eq "$([[ -e "$FAKE_OSASCRIPT_OUT" ]] && print yes)" "" "successful terminal-notifier does not fall back"

echo "ws hook: every built-in adapter honors per-state sound file overrides"
cp "$ROOT/assets/ws/sounds/waiting.wav" "$TMP/custom waiting.wav"
for adapter in auto terminal-notifier osascript; do
  for state in idle waiting; do
    hook api/feat-login "$NWIN" working </dev/null
    rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_OUT"
    WS_NOTIFY=1 WS_NOTIFIER="$adapter" WS_NOTIFY_SOUND_DONE="$TMP/custom done.wav" \
      WS_NOTIFY_SOUND_WAITING="$TMP/custom waiting.wav" hook api/feat-login "$NWIN" "$state" </dev/null
    if [[ "$adapter" == osascript ]]; then
      wait_until test -s "$FAKE_OSASCRIPT_OUT"
    else
      wait_until test -s "$FAKE_NOTIFIER_OUT"
    fi
    sound_file="$TMP/custom done.wav"
    [[ "$state" == waiting ]] && sound_file="$TMP/custom waiting.wav"
    assert_eq "$(afplay_out)" "$sound_file" "$adapter resolves $state override with spaces"
  done
done

echo "ws hook: osascript resolves system and user-library sound names"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 WS_NOTIFIER=osascript WS_NOTIFY_SOUND_WAITING=Ping hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_eq "$(afplay_out)" "/System/Library/Sounds/Ping.aiff" "waiting sound accepts a system name"
mkdir -p "$TMP/sound-home/Library/Sounds"
cp "$ROOT/assets/ws/sounds/done.wav" "$TMP/sound-home/Library/Sounds/Pop.aiff"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_OUT"
HOME="$TMP/sound-home" WS_NOTIFY=1 WS_NOTIFIER=osascript WS_NOTIFY_SOUND_DONE=Pop \
  hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_eq "$(afplay_out)" "$TMP/sound-home/Library/Sounds/Pop.aiff" "user-library sound takes precedence over a system sound"

echo "ws hook: unavailable sound and playback failure do not prevent notifications"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 WS_NOTIFY_SOUND_DONE="$TMP/missing.wav" hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_contains "$(cat "$FAKE_NOTIFIER_OUT")" "api/feat-login is idle" "missing sound still displays the notification"
assert_eq "$([[ -e "$FAKE_AFPLAY_OUT" ]] && print yes)" "" "missing sound is not played"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 FAKE_AFPLAY_EXIT=1 hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_contains "$(cat "$FAKE_NOTIFIER_OUT")" "api/feat-login needs input" "failed playback still displays the notification"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"

echo "ws hook: sound playback never delays the notification"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT" "$FAKE_AFPLAY_OUT.done"
WS_NOTIFY=1 FAKE_AFPLAY_SLEEP=3 hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_eq "$([[ -e "$FAKE_AFPLAY_OUT.done" ]] && print yes)" "" "notification posts while the sound is still playing"

echo "ws hook: unknown WS_NOTIFIER falls back to auto"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 WS_NOTIFIER=boringNotch hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_contains "$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "api/feat-login is idle" "typo'd adapter name still notifies"

echo "ws hook: sound names match any extension in the sound libraries"
hook api/feat-login "$NWIN" working </dev/null
cp "$ROOT/assets/ws/sounds/done.wav" "$TMP/sound-home/Library/Sounds/MyChime.caf"
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
HOME="$TMP/sound-home" WS_NOTIFY=1 WS_NOTIFY_SOUND_DONE=MyChime hook api/feat-login "$NWIN" idle </dev/null
assert_eq "$(afplay_out)" "$TMP/sound-home/Library/Sounds/MyChime.caf" "non-aiff user sound resolves by name"

echo "ws hook: osascript uses the event title"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_OSASCRIPT_OUT"
WS_NOTIFY=1 WS_NOTIFIER=osascript hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
oout="$(cat "$FAKE_OSASCRIPT_OUT" 2>/dev/null)"
assert_eq "$([[ "$oout" == *'title "ws"'* ]] && print yes)" "" "osascript title is not hard-coded"
assert_contains "$oout" $'api/feat-login is idle\nws' "osascript receives the event title as an argument"
rm -f "$FAKE_OSASCRIPT_OUT"

echo "ws hook: boringnotch sends one NDJSON event and leaves sound to the app"
NOTCH_SOCKET="$TMP/notch notify.sock"
NOTCH_EVENT="$TMP/notch.ndjson"
start_socket_listener "$NOTCH_SOCKET" "$NOTCH_EVENT"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"
print -r -- '{"message":"Approve deployment?\nTarget: staging"}' \
  | WS_NOTIFY=1 WS_NOTIFIER=boringnotch WS_BORINGNOTCH_SOCKET="$NOTCH_SOCKET" \
    hook api/feat-login "$NWIN" waiting
wait_until test -s "$NOTCH_EVENT"
assert_exit "$?" "0" "notch receives the event"
jq -e --arg id "ws:$NWIN" '
  .v == 1 and .id == $id and .state == "waiting" and .prev == "working" and
  .focused == false and .sound == "waiting" and .detail == "Approve deployment?\nTarget: staging" and
  (.actions[0].command | contains("select-window") and contains("switch-client"))
' "$NOTCH_EVENT" >/dev/null 2>&1
assert_exit "$?" "0" "notch receives the event with detail and focus action"
assert_eq "$(wc -l < "$NOTCH_EVENT" | tr -d ' ')" "1" "notch receives exactly one newline-terminated JSON object"
sleep 0.3
assert_eq "$([[ -e "$FAKE_AFPLAY_LOG" ]] && print yes)" "" "successful notch delivery leaves sound to the app"
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" || -e "$FAKE_OSASCRIPT_OUT" ]] && print yes)" "" "successful notch delivery bypasses macOS notifications"
stop_socket_listener
rm -f "$NOTCH_SOCKET" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"

echo "ws hook: boringnotch uses its default socket path"
DEFAULT_NOTCH_SOCKET="$NOTCH_HOME/Library/Application Support/boringNotch/notify.sock"
mkdir -p "${DEFAULT_NOTCH_SOCKET:h}"
start_socket_listener "$DEFAULT_NOTCH_SOCKET" "$NOTCH_EVENT"
HOME="$NOTCH_HOME" WS_NOTIFY=1 WS_NOTIFIER=boringnotch WS_BORINGNOTCH_SOCKET="" \
  WS_NOTIFY_SOUND_DONE="$TMP/custom done.wav" hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$NOTCH_EVENT"
jq -e '.state == "idle" and .prev == "waiting" and .sound == "done"' "$NOTCH_EVENT" >/dev/null 2>&1
assert_exit "$?" "0" "notch receives idle at the default socket under HOME"
sleep 0.3
assert_eq "$([[ -e "$FAKE_AFPLAY_LOG" ]] && print yes)" "" "notch delivery ignores ws sound overrides"
stop_socket_listener
rm -f "$DEFAULT_NOTCH_SOCKET" "$FAKE_AFPLAY_LOG"

echo "ws hook: boringnotch falls back to auto when the socket is missing"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 WS_NOTIFIER=boringnotch WS_BORINGNOTCH_SOCKET="$NOTCH_SOCKET" \
  hook api/feat-login "$NWIN" waiting </dev/null
assert_exit "$?" "0" "missing notch socket does not fail the hook"
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_contains "$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "api/feat-login needs input" "missing notch socket uses auto notification"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/waiting.wav" "missing socket fallback plays its cue exactly once"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"

echo "ws hook: boringnotch falls back when a socket has no listener"
start_socket_listener "$NOTCH_SOCKET" "$NOTCH_EVENT"
stop_socket_listener
assert_eq "$([[ -S "$NOTCH_SOCKET" ]] && print yes)" "yes" "stale socket remains without a listener"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 WS_NOTIFIER=boringnotch WS_BORINGNOTCH_SOCKET="$NOTCH_SOCKET" \
  hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_contains "$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "api/feat-login needs input" "unreachable notch socket uses auto notification"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/waiting.wav" "unreachable socket fallback plays its cue exactly once"
rm -f "$NOTCH_SOCKET" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"

echo "ws dismiss: clears an open notification once with a closed event"
APPEND="$TMP/custom adapters/append"
event_count() { wc -l < "$FAKE_EVENT_LOG" 2>/dev/null | tr -d ' '; }
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_EVENT_LOG"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_EVENT_LOG"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" "$WS" _dismiss "$NWIN"
assert_exit "$?" "0" "dismiss succeeds"
wait_until eval '(( $(event_count) >= 2 ))'
jq -se --arg id "ws:$NWIN" '
  .[1].v == 1 and .[1].id == $id and .[1].state == "closed" and .[1].prev == "waiting" and
  .[1].sound == "" and .[1].seq > .[0].seq and (.[1].actions | length) == 1
' "$FAKE_EVENT_LOG" >/dev/null 2>&1
assert_exit "$?" "0" "dismiss emits a newer closed event for the tab"
assert_eq "$(T display -p -t "$NWIN" '#{@ws_state}')" "waiting" "dismiss keeps the tab state"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" "$WS" _dismiss "$NWIN"
sleep 0.3
assert_eq "$(event_count)" "2" "second dismiss emits nothing"
KEEPALIVE_WIN="$(T display -p -t keepalive '#{window_id}')"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" "$WS" _dismiss "$KEEPALIVE_WIN"
assert_exit "$?" "0" "dismiss outside ws tabs is a no-op"
WS_NOTIFY=0 WS_NOTIFIER="$APPEND" hook api/feat-login "$NWIN" idle </dev/null
sleep 0.3
assert_eq "$(event_count)" "2" "no extra events from outside tabs or with WS_NOTIFY=0"

echo "ws dismiss: selecting the tab dismisses its notification"
NSESS="$(T display -p -t "$NWIN" '#{session_id}')"
OTHER_WIN="$(T new-window -d -P -F '#{window_id}' -t "$NSESS:")"
T select-window -t "$OTHER_WIN"
T set-environment -g WS_NOTIFY 1
T set-environment -g WS_NOTIFIER "$APPEND"
T set-environment -g FAKE_EVENT_LOG "$FAKE_EVENT_LOG"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_EVENT_LOG"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_EVENT_LOG"
T select-window -t "$NWIN"
wait_until eval 'jq -se "map(.state) == [\"waiting\", \"closed\"]" "$FAKE_EVENT_LOG" >/dev/null 2>&1'
assert_exit "$?" "0" "tab selection emits closed"
T kill-window -t "$OTHER_WIN"

echo "ws dismiss: agent exit dismisses its notification"
before="$(windows_of "$WT")"
"$WS" add api/feat-login --detach >/dev/null
XWIN="$(windows_of "$WT" | grep -vxF -- "$before" | head -1)"
wait_pane "$XWIN" FAKE-claude >/dev/null
rm -f "$FAKE_EVENT_LOG"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" hook api/feat-login "$XWIN" waiting </dev/null
wait_until test -s "$FAKE_EVENT_LOG"
T send-keys -t "$XWIN" Enter
wait_until eval 'jq -se --arg id "ws:$XWIN" "map(select(.id == \$id) | .state) == [\"waiting\", \"closed\"]" "$FAKE_EVENT_LOG" >/dev/null 2>&1'
assert_exit "$?" "0" "agent exit emits closed for its tab"
T set-environment -gu WS_NOTIFY
T set-environment -gu WS_NOTIFIER
T set-environment -gu FAKE_EVENT_LOG

echo "ws hook: failed terminal-notifier falls back to osascript"
cat > "$TMP/bin/terminal-notifier" <<'EOF'
#!/bin/sh
exit 1
EOF
WS_NOTIFY=1 WS_NOTIFIER=auto hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_contains "$(cat "$FAKE_OSASCRIPT_OUT")" "api/feat-login is idle" "rejected auto notification falls back with the same message"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/done.wav" "auto fallback plays the done cue exactly once"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"
WS_NOTIFY=1 WS_NOTIFIER=terminal-notifier hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_contains "$(cat "$FAKE_OSASCRIPT_OUT")" "api/feat-login needs input" "explicit terminal-notifier also falls back"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/waiting.wav" "explicit fallback plays the waiting cue exactly once"

echo "ws hook: boringnotch fallback also survives a rejected terminal notification"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"
WS_NOTIFY=1 WS_NOTIFIER=boringnotch hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_contains "$(cat "$FAKE_OSASCRIPT_OUT" 2>/dev/null)" "api/feat-login needs input" "notch fallback reaches osascript when terminal-notifier fails"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/waiting.wav" "nested notch fallback plays its cue exactly once"

echo "ws hook: auto uses osascript when terminal-notifier is absent"
mkdir -p "$TMP/fallback-bin"
for cmd in tmux jq git date cat cut afplay; do
  ln -s "$(command -v "$cmd")" "$TMP/fallback-bin/$cmd"
done
ln -s "$TMP/bin/osascript" "$TMP/fallback-bin/osascript"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"
PATH="$TMP/fallback-bin" WS_NOTIFY=1 hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_contains "$(cat "$FAKE_OSASCRIPT_OUT")" "api/feat-login is idle" "auto fallback works without any terminal-notifier executable"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/done.wav" "missing-notifier fallback plays the done cue exactly once"

echo "ws hook: focused transitions reach custom adapters"
mkfifo "$TMP/control.in"
exec {control_fd}<>"$TMP/control.in"
T select-window -t "$NWIN"
T -C attach-session -t "$(session_of "$NWIN")" < "$TMP/control.in" > "$TMP/control.out" 2>&1 &
tab_focused() { [[ "$(T display -p -t "$NWIN" '#{&&:#{window_active},#{session_attached}}')" == 1 ]]; }
wait_until tab_focused
assert_exit "$?" "0" "test client focuses the notifying tab"
rm -f "$FAKE_EVENT_OUT"
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_EVENT_OUT"
jq -e '.state == "waiting" and .prev == "idle" and .focused == true' "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "focused waiting transition reaches the custom adapter"
for state in working idle; do
  rm -f "$FAKE_EVENT_OUT"
  WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" "$state" </dev/null
  wait_until test -s "$FAKE_EVENT_OUT"
  jq -e --arg state "$state" '.state == $state and .focused == true' "$FAKE_EVENT_OUT" >/dev/null 2>&1
  assert_exit "$?" "0" "focused $state transition reaches the custom adapter"
done

echo "ws hook: boringnotch receives focused transitions with semantic sound keys (app plays them)"
for state in working waiting; do
  start_socket_listener "$NOTCH_SOCKET" "$NOTCH_EVENT"
  rm -f "$FAKE_AFPLAY_LOG" "$FAKE_NOTIFIER_OUT" "$FAKE_OSASCRIPT_OUT"
  WS_NOTIFY=1 WS_NOTIFIER=boringnotch WS_BORINGNOTCH_SOCKET="$NOTCH_SOCKET" \
    WS_NOTIFY_SOUND_WAITING="$TMP/custom waiting.wav" hook api/feat-login "$NWIN" "$state" </dev/null
  wait_until test -s "$NOTCH_EVENT"
  jq -e --arg state "$state" '.state == $state and .focused == true' "$NOTCH_EVENT" >/dev/null 2>&1
  assert_exit "$?" "0" "notch receives focused $state transition"
  jq -e --arg state "$state" '.sound == (if $state == "waiting" then "waiting" else "" end)' "$NOTCH_EVENT" >/dev/null 2>&1
  assert_exit "$?" "0" "focused $state event carries its semantic sound key"
  sleep 0.3
  assert_eq "$([[ -e "$FAKE_AFPLAY_LOG" ]] && print yes)" "" "ws plays no $state sound for the notch"
  assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" || -e "$FAKE_OSASCRIPT_OUT" ]] && print yes)" "" "focused notch delivery bypasses macOS notifications"
  stop_socket_listener
  rm -f "$NOTCH_SOCKET"
done
hook api/feat-login "$NWIN" idle </dev/null

echo "ws hook: macOS adapters and notch fallback suppress every focused transition"
for adapter in auto terminal-notifier osascript boringnotch; do
  rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_OUT"
  for state in waiting working idle; do
    WS_NOTIFY=1 WS_NOTIFIER="$adapter" hook api/feat-login "$NWIN" "$state" </dev/null
  done
  sleep 0.3
  assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" || -e "$FAKE_OSASCRIPT_OUT" || -e "$FAKE_AFPLAY_OUT" ]] && print yes)" "" \
    "$adapter suppresses notifications and sounds for focused idle, waiting and working"
done
T detach-client -s "$(session_of "$NWIN")"
exec {control_fd}>&-

echo "ws hook: macOS adapters and notch fallback suppress unfocused working transitions"
for adapter in auto terminal-notifier osascript boringnotch; do
  hook api/feat-login "$NWIN" idle </dev/null
  rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_OUT"
  WS_NOTIFY=1 WS_NOTIFIER="$adapter" hook api/feat-login "$NWIN" working </dev/null
  sleep 0.3
  assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" || -e "$FAKE_OSASCRIPT_OUT" || -e "$FAKE_AFPLAY_OUT" ]] && print yes)" "" \
    "$adapter suppresses notifications and sounds for unfocused working"
done

cat > "$TMP/bin/terminal-notifier" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_NOTIFIER_OUT"
EOF
hook api/feat-login "$NWIN" idle </dev/null

echo "ws _build-notifier: own-branded notifier app (name, bundle id, icon); used when present"
SRCAPP="$TMP/src/terminal-notifier.app"
mkdir -p "$SRCAPP/Contents/MacOS" "$SRCAPP/Contents/Resources"
cat > "$SRCAPP/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>fr.julienxx.oss.terminal-notifier</string>
<key>CFBundleName</key><string>terminal-notifier</string>
<key>CFBundleExecutable</key><string>terminal-notifier</string>
<key>CFBundleIconFile</key><string>Terminal</string>
</dict></plist>
EOF
cat > "$SRCAPP/Contents/MacOS/terminal-notifier" <<'EOF'
#!/bin/sh
{ echo "BRANDED"; printf '%s\n' "$@"; } > "$FAKE_NOTIFIER_OUT"
EOF
chmod +x "$SRCAPP/Contents/MacOS/terminal-notifier"
WS_NOTIFIER_SOURCE="$SRCAPP" "$WS" _build-notifier >/dev/null 2>&1
assert_exit "$?" "0" "builds the notifier app"
APP="$WS_HOME/ws.app"
assert_eq "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist" 2>/dev/null)" "sh.ratel.ws.notifier" "own bundle id"
assert_eq "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$APP/Contents/Info.plist" 2>/dev/null)" "ws" "named ws"
assert_eq "$([[ -s "$APP/Contents/Resources/Terminal.icns" ]] && file -b "$APP/Contents/Resources/Terminal.icns" | grep -c 'icon')" "1" "icon replaced with a real .icns"
assert_eq "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$SRCAPP/Contents/Info.plist")" "fr.julienxx.oss.terminal-notifier" "source app untouched"
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 hook api/feat-login "$(slot_window "$WT" 1)" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_eq "$(head -1 "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "BRANDED" "notifications sent through ws.app when built"
hook api/feat-login "$(slot_window "$WT" 1)" working </dev/null
rm -rf "$APP"

echo "ws merge: merges the PR, fast-forwards the main checkout, removes worktree + branch on confirm"
MREPO="$(make_repo merge-me)"
git init -q --bare "$TMP/merge-me.git"
git -C "$MREPO" remote add origin "$TMP/merge-me.git"
git -C "$MREPO" push -q origin main
git --git-dir="$TMP/merge-me.git" symbolic-ref HEAD refs/heads/main
git -C "$MREPO" fetch -q origin && git -C "$MREPO" remote set-head origin -a >/dev/null
"$WS" new --repo "$MREPO" --branch feat/ship --detach
MWT="$WS_ROOT/merge-me/feat-ship"
git -C "$MWT" commit -q --allow-empty -m "ship it"
git -C "$MWT" push -q origin feat/ship
# "GitHub merges the PR": land a commit on origin/main from elsewhere
UPSTREAM="$TMP/upstream-clone"
git clone -q "$TMP/merge-me.git" "$UPSTREAM"
git -C "$UPSTREAM" -c user.email=t@e -c user.name=T commit -q --allow-empty -m "squashed: ship it"
git -C "$UPSTREAM" push -q origin main
cat > "$TMP/bin/gh" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_GH_LOG"
[ "$1 $2" = "pr view" ] && echo "#7 Ship it (https://github.com/acme/merge-me/pull/7)"
exit 0
EOF
chmod +x "$TMP/bin/gh"
export FAKE_GH_LOG="$TMP/gh.log"
out="$(cd "$MWT" && print n | "$WS" merge 2>&1)"
assert_contains "$out" "merge PR #7 Ship it (https://github.com/acme/merge-me/pull/7)" "asks to confirm, naming the PR"
assert_eq "$(grep -c 'pr merge' "$FAKE_GH_LOG")" "0" "declined merge: PR not merged"
assert_eq "$(git -C "$MREPO" log -1 --format=%s main)" "initial" "declined merge: main untouched"
(cd "$MWT" && printf 'y\nn\n' | "$WS" merge) >/dev/null 2>&1
assert_contains "$(cat "$FAKE_GH_LOG")" "pr merge feat/ship --squash" "squash-merges the branch's PR"
assert_eq "$(git -C "$MREPO" log -1 --format=%s main)" "squashed: ship it" "main checkout fast-forwarded"
assert_eq "$([[ -d "$MWT" ]] && print yes)" "yes" "declined: worktree kept"
assert_eq "$(git -C "$MREPO" branch --list feat/ship | tr -d ' *+')" "feat/ship" "declined: branch kept"
(cd "$MWT" && printf 'y\ny\n' | "$WS" merge) >/dev/null 2>&1
assert_eq "$([[ -d "$MWT" ]] && print yes)" "" "confirmed: worktree removed"
assert_eq "$(git -C "$MREPO" branch --list feat/ship)" "" "confirmed: unmerged-by-ancestry (squashed) branch force-deleted"
assert_eq "$(git -C "$MREPO" ls-remote --heads origin feat/ship)" "" "confirmed: remote branch deleted"
"$WS" new --repo "$MREPO" --branch feat/dirty --detach
DWT="$WS_ROOT/merge-me/feat-dirty"
git -C "$DWT" push -q origin feat/dirty
print junk > "$DWT/stray.txt"
out="$(cd "$DWT" && printf 'y\ny\nn\n' | "$WS" merge 2>&1)"
assert_contains "$out" "stray.txt" "dirty after merge: lists the offending files"
assert_contains "$out" "force delete" "dirty after merge: asks to force delete"
assert_eq "$([[ -d "$DWT" ]] && print yes)" "yes" "dirty, force declined: worktree kept"
(cd "$DWT" && printf 'y\ny\ny\n' | "$WS" merge) >/dev/null 2>&1
assert_eq "$([[ -d "$DWT" ]] && print yes)" "" "dirty, force confirmed: worktree removed"
assert_eq "$(git -C "$MREPO" branch --list feat/dirty)" "" "dirty, force confirmed: branch deleted"
(cd "$MREPO" && env -u TMUX "$WS" merge </dev/null 2>/dev/null)
assert_exit "$?" "1" "merge outside a workspace refused"

echo "ws send: idle named target receives one bracketed multiline prompt with sender id"
"$WS" new --repo "$REPO" --branch delivery --detach
SEND_WT="$WS_ROOT/api/delivery"
SEND_WIN="$(windows_of "$SEND_WT")"
SEND_ID="$(T display -p -t "$SEND_WIN" '#{@ws_session}')"
assert_contains "$(wait_pane "$SEND_WIN" PASTE-READY)" "PASTE-READY" "fake delivery TUI ready"
WS_SESSION="$SEND_ID" "$WS" name receiver
MESSAGE=$'first line\nsecond "quoted" line $literal'
print -r -- "$MESSAGE" | WS_SESSION=1 "$WS" send receiver -
assert_exit "$?" "0" "send accepts stdin and target name"
wait_until test -s "$WS_HOME/pane-$SEND_ID.jsonl"
assert_eq "$(jq -r '.text' "$WS_HOME/pane-$SEND_ID.jsonl" 2>/dev/null)" $'[ws #1]\nfirst line\nsecond "quoted" line $literal' "sender prefix and multiline text reach pane intact"
assert_eq "$(jq -r '.raw | startswith("\u001b[200~") and endswith("\u001b[201~\r")' "$WS_HOME/pane-$SEND_ID.jsonl" 2>/dev/null)" "true" "paste is bracketed with separate Enter"
assert_eq "$(wc -l < "$WS_HOME/pane-$SEND_ID.jsonl" 2>/dev/null | tr -d ' ')" "1" "multiline message submits exactly once"
echo "ws send: working and waiting sessions consume inbox once through synchronous Stop hooks"
for agent in claude codex grok; do
  if [[ "$agent" == claude ]]; then
    BUS_WIN="$SEND_WIN"
  else
    "$WS" add api/delivery --agent "$agent" --detach
    BUS_WIN="$("$WS" list | awk -F '\t' -v p="$SEND_WT" -v a="$agent" '$3 == p && $4 == a {print $5; exit}')"
    assert_contains "$(wait_pane "$BUS_WIN" PASTE-READY)" "PASTE-READY" "$agent fake TUI ready"
  fi
  BUS_ID="$(T display -p -t "$BUS_WIN" '#{@ws_session}')"
  for busy_state in working waiting; do
    hook api/delivery "$BUS_WIN" "$busy_state" </dev/null
    before="$(cat "$WS_HOME/pane-$BUS_ID.jsonl" 2>/dev/null)"
    WS_SESSION=1 "$WS" send "$BUS_ID" "$agent $busy_state message"
    assert_exit "$?" "0" "$agent queues while $busy_state"
    assert_eq "$(cat "$WS_HOME/pane-$BUS_ID.jsonl" 2>/dev/null)" "$before" "$agent busy message never goes into pane input"
    reply="$(print '{"hook_event_name":"Stop","stop_hook_active":false,"stopHookActive":false}' | hook api/delivery "$BUS_WIN" stop)"
    assert_eq "$(print -r -- "$reply" | jq -r '.decision' 2>/dev/null)" "block" "$agent Stop requests continuation"
    assert_eq "$(print -r -- "$reply" | jq -r '.reason' 2>/dev/null)" "[ws #1]"$'\n'"$agent $busy_state message" "$agent Stop delivers sender-prefixed message"
    assert_eq "$(T display -p -t "$BUS_WIN" '#{@ws_state}')" "working" "$agent block keeps tab working"
    assert_eq "$([[ -s "$WS_HOME/sessions/$BUS_ID/inbox" ]] && print pending)" "" "$agent Stop consumes durable inbox"
    reply="$(print '{"hook_event_name":"Stop","stop_hook_active":true,"stopHookActive":true}' | hook api/delivery "$BUS_WIN" stop)"
    assert_eq "$reply" "{}" "$agent continued Stop never repeats consumed messages"
    assert_eq "$(T display -p -t "$BUS_WIN" '#{@ws_state}')" "idle" "$agent empty Stop marks idle"
  done
done
assert_eq "$(jq -r '.hooks.Stop[0].hooks[0].command' "$WS_HOME/claude-settings.json")" "$WS hook stop" "Claude Stop uses synchronous bus delivery"
CODEX_STOP_ID="$(T display -p -t "$(slot_window "$SEND_WT" 2)" '#{@ws_session}')"
assert_eq "$(python3 - "$WS_HOME/argv-$CODEX_STOP_ID.json" <<'PY_STOP'
import json, sys, tomllib
config = next(arg for arg in json.load(open(sys.argv[1])) if arg.startswith("hooks.Stop="))
print(tomllib.loads(config)["hooks"]["Stop"][0]["hooks"][0]["command"])
PY_STOP
)" "$WS hook stop" "Codex Stop uses synchronous bus delivery"
assert_eq "$(jq -r '.hooks.Stop[0].hooks[0].command' "$GROK_HOME/hooks/ws.json")" "$WS hook stop" "Grok Stop uses synchronous bus delivery"

pane_received() { [[ "$(tail -1 "$WS_HOME/pane-$1.jsonl" 2>/dev/null | jq -r '.text')" == "$2" ]]; }
echo "ws send: continuation Stops block only for new messages; cancellation pastes on verified idle"
for agent in claude codex grok; do
  BUS_WIN="$("$WS" list | awk -F '\t' -v p="$SEND_WT" -v a="$agent" '$3 == p && $4 == a {print $5; exit}')"
  BUS_ID="$(T display -p -t "$BUS_WIN" '#{@ws_session}')"
  hook api/delivery "$BUS_WIN" working </dev/null
  WS_SESSION=1 "$WS" send "$BUS_ID" "$agent guarded message"
  if [[ "$agent" == grok ]]; then active_payload='{"stopHookActive":true}'; else active_payload='{"stop_hook_active":true}'; fi
  before="$(cat "$WS_HOME/pane-$BUS_ID.jsonl" 2>/dev/null)"
  reply="$(print -r -- "$active_payload" | hook api/delivery "$BUS_WIN" stop)"
  assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty' 2>/dev/null)" "[ws #1]"$'\n'"$agent guarded message" "$agent active Stop blocks for new messages"
  assert_eq "$(cat "$WS_HOME/pane-$BUS_ID.jsonl" 2>/dev/null)" "$before" "$agent active Stop never pastes into the blocked TUI"
  assert_eq "$(T display -p -t "$BUS_WIN" '#{@ws_state}')" "working" "$agent continued delivery is working"
  reply="$(print -r -- "$active_payload" | hook api/delivery "$BUS_WIN" stop)"
  assert_eq "$reply" "{}" "$agent active Stop with an empty inbox allows stopping"
  hook api/delivery "$BUS_WIN" working </dev/null
  WS_SESSION=1 "$WS" send "$BUS_ID" "$agent cancelled message"
  hook api/delivery "$BUS_WIN" idle </dev/null
  wait_until pane_received "$BUS_ID" "[ws #1]"$'\n'"$agent cancelled message"
  assert_eq "$(tail -1 "$WS_HOME/pane-$BUS_ID.jsonl" 2>/dev/null | jq -r '.text')" "[ws #1]"$'\n'"$agent cancelled message" "$agent cancellation delivers inbox at idle"
  assert_eq "$([[ -s "$WS_HOME/sessions/$BUS_ID/inbox" ]] && print pending)" "" "$agent fallback consumes inbox"
done

echo "ws send: a background subagent postpones idle paste until the whole tab is idle"
hook api/delivery "$SEND_WIN" working </dev/null
print '{"agent_id":"delivery-bg"}' | hook api/delivery "$SEND_WIN" subagent-start
WS_SESSION=1 "$WS" send "$SEND_ID" "after background"
before="$(cat "$WS_HOME/pane-$SEND_ID.jsonl")"
hook api/delivery "$SEND_WIN" idle </dev/null
assert_eq "$(cat "$WS_HOME/pane-$SEND_ID.jsonl")" "$before" "background activity prevents fallback input"
print '{"agent_id":"delivery-bg"}' | hook api/delivery "$SEND_WIN" subagent-stop
wait_until pane_received "$SEND_ID" $'[ws #1]\nafter background'
assert_eq "$(tail -1 "$WS_HOME/pane-$SEND_ID.jsonl" | jq -r '.text')" $'[ws #1]\nafter background' "last subagent stop drains idle inbox"

# --- M3 stale cancel ---
echo "ws hook: a late Grok cancellation cannot idle a newer turn or paste its inbox"
M3_WIN="$(slot_window "$SEND_WT" 3)"
M3_ID="$(T display -p -t "$M3_WIN" '#{@ws_session}')"
print '{"hookEventName":"user_prompt_submit","promptId":"grok-old"}' | hook api/delivery "$M3_WIN" working
print '{"hookEventName":"user_prompt_submit","promptId":"grok-new"}' | hook api/delivery "$M3_WIN" working
WS_SESSION=1 "$WS" send "$M3_ID" "after the current turn"
M3_INBOX="$(cat "$WS_HOME/sessions/$M3_ID/inbox")"
M3_PANE="$(cat "$WS_HOME/pane-$M3_ID.jsonl" 2>/dev/null)"
print '{"hookEventName":"stop_cancelled","promptId":"grok-old","reason":"user_interrupt"}' | hook api/delivery "$M3_WIN" idle
assert_contains "$("$WS" sessions "$M3_ID")" $'\tgrok\tworking\tactive' "stale Grok cancel leaves the newer turn working"
assert_eq "$(cat "$WS_HOME/sessions/$M3_ID/inbox")" "$M3_INBOX" "stale Grok cancel keeps the inbox intact"
assert_eq "$(cat "$WS_HOME/pane-$M3_ID.jsonl" 2>/dev/null)" "$M3_PANE" "stale Grok cancel submits no pane input"
print '{}' | hook api/delivery "$M3_WIN" stop >/dev/null

echo "ws hook: the configured Codex Interrupt without a turn id cannot settle or paste"
M3_WIN="$(slot_window "$SEND_WT" 2)"
M3_ID="$(T display -p -t "$M3_WIN" '#{@ws_session}')"
M3_CANCEL_COMMAND="$(python3 - "$WS_HOME/argv-$M3_ID.json" <<'PY_M3'
import json, sys, tomllib
argv = json.load(open(sys.argv[1]))
config = next(arg for arg in argv if arg.startswith("hooks.Interrupt="))
print(tomllib.loads(config)["hooks"]["Interrupt"][0]["hooks"][0]["command"])
PY_M3
)"
print '{"hook_event_name":"UserPromptSubmit","turn_id":"codex-current"}' | hook api/delivery "$M3_WIN" working
WS_SESSION=1 "$WS" send "$M3_ID" "idless interruption"
M3_INBOX="$(cat "$WS_HOME/sessions/$M3_ID/inbox")"
M3_PANE="$(cat "$WS_HOME/pane-$M3_ID.jsonl" 2>/dev/null)"
print '{}' | WS_WORKSPACE=api/delivery TMUX_PANE="$(T display -p -t "$M3_WIN" '#{pane_id}')" zsh -c "$M3_CANCEL_COMMAND"
assert_eq "$(cat "$WS_HOME/sessions/$M3_ID/inbox")" "$M3_INBOX" "idless Codex Interrupt keeps queued messages"
assert_eq "$(cat "$WS_HOME/pane-$M3_ID.jsonl" 2>/dev/null)" "$M3_PANE" "idless Codex Interrupt never pastes"
assert_contains "$("$WS" sessions "$M3_ID")" $'\tcodex\tworking\tactive' "idless Codex Interrupt leaves the turn working"
print '{}' | hook api/delivery "$M3_WIN" stop >/dev/null

echo "ws hook: matching cancellations deliver once and cannot cancel the pasted turn"
for agent field slot in claude prompt_id 1 codex turn_id 2 grok promptId 3; do
  M3_WIN="$(slot_window "$SEND_WT" "$slot")"
  M3_ID="$(T display -p -t "$M3_WIN" '#{@ws_session}')"
  jq -nc --arg field "$field" '{hook_event_name:"UserPromptSubmit", ($field):"old"}' | hook api/delivery "$M3_WIN" working
  jq -nc --arg field "$field" '{hook_event_name:"UserPromptSubmit", ($field):"current | opaque"}' | hook api/delivery "$M3_WIN" working
  print '{"hook_event_name":"PreToolUse"}' | hook api/delivery "$M3_WIN" working
  jq -nc --arg field "$field" '{hook_event_name:"Interrupt", ($field):"old"}' | hook api/delivery "$M3_WIN" idle
  assert_contains "$("$WS" sessions "$M3_ID")" "$agent"$'\tworking\tactive' "$agent stale cancel with empty inbox keeps working"
  WS_SESSION=1 "$WS" send "$M3_ID" "$agent matching cancellation"
  M3_CANCEL_PAYLOAD="$(jq -nc --arg field "$field" '{($field):"current | opaque"}')"
  print -r -- "$M3_CANCEL_PAYLOAD" | hook api/delivery "$M3_WIN" cancel
  wait_until pane_received "$M3_ID" "[ws #1]"$'\n'"$agent matching cancellation"
  assert_exit "$?" "0" "$agent matching cancel pastes after idless tool hook"
  assert_eq "$([[ -s "$WS_HOME/sessions/$M3_ID/inbox" ]] && print pending)" "" "$agent matching cancel drains the inbox"
  WS_SESSION=1 "$WS" send "$M3_ID" "$agent next pasted turn"
  M3_INBOX="$(cat "$WS_HOME/sessions/$M3_ID/inbox")"
  M3_PANE="$(cat "$WS_HOME/pane-$M3_ID.jsonl")"
  print -r -- "$M3_CANCEL_PAYLOAD" | hook api/delivery "$M3_WIN" cancel
  assert_eq "$(cat "$WS_HOME/sessions/$M3_ID/inbox")" "$M3_INBOX" "$agent repeated cancel preserves the next turn's inbox"
  assert_eq "$(cat "$WS_HOME/pane-$M3_ID.jsonl")" "$M3_PANE" "$agent repeated cancel never pastes twice"
  print '{}' | hook api/delivery "$M3_WIN" stop >/dev/null
  jq -nc --arg field "$field" '{hook_event_name:"UserPromptSubmit", ($field):"current | opaque"}' | hook api/delivery "$M3_WIN" working
  print -r -- "$M3_CANCEL_PAYLOAD" | hook api/delivery "$M3_WIN" cancel
  assert_contains "$("$WS" sessions "$M3_ID")" "$agent"$'\tidle\tactive' "$agent matching cancel with empty inbox settles idle"
  jq -nc --arg field "$field" '{hook_event_name:"UserPromptSubmit", ($field):"current | opaque"}' | hook api/delivery "$M3_WIN" working
  print '{"hook_event_name":"UserPromptSubmit"}' | hook api/delivery "$M3_WIN" working
  print -r -- "$M3_CANCEL_PAYLOAD" | hook api/delivery "$M3_WIN" cancel
  assert_contains "$("$WS" sessions "$M3_ID")" "$agent"$'\tworking\tactive' "$agent new prompt without an id invalidates the previous turn"
  print '{}' | hook api/delivery "$M3_WIN" stop >/dev/null
done

# --- delivery robustness: session-end Stop ---
echo "ws hook: Grok session-end Stop leaves accepted work for resume"
ROBUST_WIN="$(slot_window "$SEND_WT" 3)"
ROBUST_ID="$(T display -p -t "$ROBUST_WIN" '#{@ws_session}')"
hook api/delivery "$ROBUST_WIN" working </dev/null
WS_SESSION=1 "$WS" send "$ROBUST_ID" "survive session end"
ROBUST_INBOX="$(cat "$WS_HOME/sessions/$ROBUST_ID/inbox")"
for reason in channel_closed shutdown; do
  reply="$(jq -nc --arg reason "$reason" '{hook_event_name:"Stop",reason:$reason}' | hook api/delivery "$ROBUST_WIN" stop)"
  assert_eq "$reply" "{}" "$reason allows session end without continuation"
  assert_eq "$(cat "$WS_HOME/sessions/$ROBUST_ID/inbox")" "$ROBUST_INBOX" "$reason retains the accepted inbox"
done
reply="$(print '{"hook_event_name":"Stop"}' | hook api/delivery "$ROBUST_WIN" stop)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'[ws #1]\nsurvive session end' "ordinary Stop still delivers retained work"
print '{}' | hook api/delivery "$ROBUST_WIN" stop >/dev/null
# --- end delivery robustness: session-end Stop ---

# --- delivery robustness: idle paste acknowledgement ---
echo "ws send: idle paste awaits a newer prompt submission before acknowledgement"
wait_until test -s "$WS_HOME/argv-$ROBUST_ID.json"
hook api/delivery "$ROBUST_WIN" idle </dev/null
: > "$WS_HOME/no-submit-$ROBUST_ID" # model a picker consuming Enter without a prompt hook
mkfifo "$TMP/robust-prompt.in"
exec {robust_w}<>"$TMP/robust-prompt.in"
(exec {robust_w}>&-; hook api/delivery "$ROBUST_WIN" working < "$TMP/robust-prompt.in") &
robust_stale_pid=$!
sleep 0.3
(exec {robust_w}>&-; WS_SESSION=1 "$WS" send "$ROBUST_ID" "await confirmation")
wait_until pane_received "$ROBUST_ID" $'[ws #1]\nawait confirmation'
assert_eq "$(T show -wqv -t "$ROBUST_WIN" @ws_state)" "idle" "paste alone never forces working"
ROBUST_PENDING="$(cat "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste" 2>/dev/null)"
assert_contains "$ROBUST_PENDING" "await confirmation" "unconfirmed paste stays durably in flight"
print -r -- '{"hook_event_name":"UserPromptSubmit"}' >&$robust_w
exec {robust_w}>&-
wait "$robust_stale_pid"
assert_eq "$(cat "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste" 2>/dev/null)" "$ROBUST_PENDING" "older prompt hook cannot ack a newer paste"
print '{"hook_event_name":"PreToolUse"}' | hook api/delivery "$ROBUST_WIN" working
assert_eq "$(cat "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste" 2>/dev/null)" "$ROBUST_PENDING" "tool hook cannot ack an unconfirmed paste"
print '{"hook_event_name":"UserPromptSubmit"}' | hook api/delivery "$ROBUST_WIN" working
assert_eq "$([[ -e "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste" ]] && print pending)" "" "newer prompt submission acknowledges the pasted batch"
assert_eq "$(T show -wqv -t "$ROBUST_WIN" @ws_state)" "working" "confirmed submission marks working"
print '{}' | hook api/delivery "$ROBUST_WIN" stop >/dev/null

echo "ws send: an overtaken prompt hook still confirms its earlier paste"
WS_SESSION=1 "$WS" send "$ROBUST_ID" "overtaken confirmation"
wait_until pane_received "$ROBUST_ID" $'[ws #1]\novertaken confirmation'
mkfifo "$TMP/robust-overtaken.in"
exec {robust_w}<>"$TMP/robust-overtaken.in"
(exec {robust_w}>&-; hook api/delivery "$ROBUST_WIN" working < "$TMP/robust-overtaken.in") &
robust_stale_pid=$!
sleep 0.5
print '{"hook_event_name":"PreToolUse"}' | hook api/delivery "$ROBUST_WIN" working
print -r -- '{"hook_event_name":"UserPromptSubmit"}' >&$robust_w
exec {robust_w}>&-
wait "$robust_stale_pid"
assert_eq "$([[ -e "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste" ]] && print pending)" "" "newer prompt acknowledges even when another hook overtakes its state update"
print '{}' | hook api/delivery "$ROBUST_WIN" stop >/dev/null

echo "ws send: swallowed Enter times out and requeues without forcing working"
WS_PASTE_ACK_TIMEOUT=1.5 WS_SESSION=1 "$WS" send "$ROBUST_ID" "swallowed Enter"
wait_until pane_received "$ROBUST_ID" $'[ws #1]\nswallowed Enter'
ROBUST_PANE="$(cat "$WS_HOME/pane-$ROBUST_ID.jsonl")"
WS_SESSION=1 "$WS" send "$ROBUST_ID" "queued after paste"
assert_eq "$(cat "$WS_HOME/pane-$ROBUST_ID.jsonl")" "$ROBUST_PANE" "sends during unconfirmed paste queue without a second Enter"
wait_until test -s "$WS_HOME/sessions/$ROBUST_ID/inbox"
wait_until test ! -e "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste"
assert_exit "$?" "0" "timeout requeues the unconfirmed batch"
assert_eq "$(jq -r '.message' "$WS_HOME/sessions/$ROBUST_ID/inbox")" $'[ws #1]\nswallowed Enter\n[ws #1]\nqueued after paste' "timeout retains both messages in delivery order"
assert_eq "$(T show -wqv -t "$ROBUST_WIN" @ws_state)" "idle" "timeout leaves the unconfirmed tab idle"
reply="$(print '{}' | hook api/delivery "$ROBUST_WIN" stop)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'[ws #1]\nswallowed Enter\n\n[ws #1]\nqueued after paste' "later Stop delivers the timed-out batch and queued sends"
rm -f "$WS_HOME/no-submit-$ROBUST_ID"
print '{}' | hook api/delivery "$ROBUST_WIN" stop >/dev/null
# --- end delivery robustness: idle paste acknowledgement ---

echo "ws send: concurrent senders retain every queued message"
hook api/delivery "$SEND_WIN" working </dev/null
send_pids=()
for i in {1..8}; do
  WS_SESSION=1 "$WS" send "$SEND_ID" "concurrent-$i" > "$TMP/send-$i.log" 2>&1 &
  send_pids+=($!)
done
for pid in $send_pids; do wait "$pid"; assert_exit "$?" "0" "concurrent send succeeds"; done
reply="$(print '{}' | hook api/delivery "$SEND_WIN" stop)"
assert_eq "$(print -r -- "$reply" | jq -r '[.reason | scan("concurrent-[1-8]")] | length')" "8" "one Stop delivers all concurrent messages"
for i in {1..8}; do assert_contains "$reply" "concurrent-$i" "queued sender $i survives"; done
assert_eq "$(print '{}' | hook api/delivery "$SEND_WIN" stop)" "{}" "concurrent inbox never repeats"

echo "ws send: done sessions warn; force resumes closed spawned sessions by retained id"
for agent in claude codex grok; do
  BUS_WIN="$("$WS" list | awk -F '\t' -v p="$SEND_WT" -v a="$agent" '$3 == p && $4 == a {print $5; exit}')"
  BUS_ID="$(T display -p -t "$BUS_WIN" '#{@ws_session}')"
  META="$WS_HOME/sessions/$BUS_ID/meta"
  jq '.status = "done" | .spawned = true | .resume_id = "resume-delivery"' "$META" > "$META.fixture"
  mv "$META.fixture" "$META"
  T kill-window -t "$BUS_WIN"
  out="$(WS_SESSION=1 "$WS" send "$BUS_ID" "wake again" 2>&1)"
  assert_exit "$?" "1" "$agent done send requires force"
  assert_contains "$out" "done" "$agent done target warns"
  assert_contains "$out" "--force" "$agent warning explains override"
  assert_eq "$("$WS" sessions "$BUS_ID" | cut -f5)" "stopped" "$agent rejected send never resumes target"
  out="$(WS_SESSION=1 "$WS" send "$BUS_ID" "wake again" --force 2>&1)"
  forced_exit=$?
  assert_exit "$forced_exit" "0" "$agent force send resumes and delivers"
  assert_contains "$out" "done" "$agent force send still warns"
  BUS_WIN="$(jq -r '.window' "$META")"
  if (( forced_exit == 0 )); then
    wait_until pane_received "$BUS_ID" $'[ws #1]\nwake again'
  fi
  assert_eq "$(tail -1 "$WS_HOME/pane-$BUS_ID.jsonl" | jq -r '.text')" $'[ws #1]\nwake again' "$agent resumed TUI receives message after delayed startup"
  assert_contains "$(T capture-pane -p -t "$BUS_WIN" 2>/dev/null)" "resume-delivery" "$agent resumes its recorded conversation"
  assert_eq "$("$WS" sessions "$BUS_ID" | cut -f6)" "done" "$agent force preserves declared done status"
done

echo "ws send: terminal sender and validation"
hook api/delivery "$BUS_WIN" idle </dev/null
env -u WS_SESSION "$WS" send --force "$BUS_ID" "from terminal" 2>/dev/null
assert_exit "$?" "0" "force is also accepted before target"
wait_until pane_received "$BUS_ID" $'[ws terminal]\nfrom terminal'
assert_eq "$(tail -1 "$WS_HOME/pane-$BUS_ID.jsonl" | jq -r '.text')" $'[ws terminal]\nfrom terminal' "untracked sender has terminal prefix"
"$WS" send no-such-session text 2>/dev/null
assert_exit "$?" "1" "unknown target refused"
print -n '' | "$WS" send "$BUS_ID" - 2>/dev/null
assert_exit "$?" "1" "empty stdin refused"
"$WS" send "$BUS_ID" 2>/dev/null
assert_exit "$?" "1" "missing message refused"

echo "ws send: startup safety also applies to a newly opened live target"
"$WS" add api/delivery --agent claude --detach
FRESH_WIN="$(slot_window "$SEND_WT" 4)"
FRESH_ID="$(T display -p -t "$FRESH_WIN" '#{@ws_session}')"
WS_SESSION=1 "$WS" send "$FRESH_ID" "fresh target"
assert_exit "$?" "0" "send waits for a new target's TUI before submitting"
wait_until pane_received "$FRESH_ID" $'[ws #1]\nfresh target'
assert_eq "$(tail -1 "$WS_HOME/pane-$FRESH_ID.jsonl" 2>/dev/null | jq -r '.text')" $'[ws #1]\nfresh target' "startup input is never discarded by terminal initialization"

echo "ws send: a slow older Stop cannot overwrite a newer pasted turn"
hook api/delivery "$FRESH_WIN" idle </dev/null
mkfifo "$TMP/send-slow.in"
exec {send_slow_w}<>"$TMP/send-slow.in"
(exec {send_slow_w}>&-; hook api/delivery "$FRESH_WIN" stop <"$TMP/send-slow.in" >"$TMP/send-slow.out") &
send_slow_pid=$!
sleep 0.5 # Stop captures its sequence before blocking on payload input.
(exec {send_slow_w}>&-; WS_SESSION=1 "$WS" send "$FRESH_ID" "newer turn")
assert_exit "$?" "0" "new message submits while old Stop awaits payload"
wait_until pane_received "$FRESH_ID" $'[ws #1]\nnewer turn'
assert_exit "$?" "0" "newer pasted turn confirms prompt submission"
exec {send_slow_w}>&-
wait "$send_slow_pid"
assert_eq "$(T display -p -t "$FRESH_WIN" '#{@ws_state}')" "working" "older Stop never idles a newer pasted turn"
assert_eq "$(cat "$TMP/send-slow.out")" "{}" "stale Stop still returns valid allow JSON"

echo "ws send: a split tab pastes only into the recorded agent pane"
hook api/delivery "$FRESH_WIN" idle </dev/null
SPLIT_PANE="$(T split-window -d -P -F '#{pane_id}' -t "$FRESH_WIN" "cat > '$TMP/split-shell.out'")"
T select-pane -t "$SPLIT_PANE"
WS_SESSION=1 "$WS" send "$FRESH_ID" "agent pane only"
assert_exit "$?" "0" "send to split tab succeeds"
wait_until pane_received "$FRESH_ID" $'[ws #1]\nagent pane only'
assert_eq "$(tail -1 "$WS_HOME/pane-$FRESH_ID.jsonl" | jq -r '.text')" $'[ws #1]\nagent pane only' "agent pane receives message"
assert_eq "$(cat "$TMP/split-shell.out" 2>/dev/null)" "" "selected shell pane never receives input"
T kill-pane -t "$SPLIT_PANE"

echo "ws send: a vanished agent pane keeps the message queued"
AGENT_PANE="$(T show -wqv -t "$FRESH_WIN" @ws_pane)"
T set -w -t "$FRESH_WIN" @ws_pane %99999
hook api/delivery "$FRESH_WIN" idle </dev/null
out="$(WS_SESSION=1 "$WS" send "$FRESH_ID" "no pane" 2>&1)"
assert_exit "$?" "75" "missing agent pane refuses paste (queued, exit 75)"
assert_contains "$out" "message queued" "missing pane explains pending delivery"
assert_eq "$(jq -r '.message' "$WS_HOME/sessions/$FRESH_ID/inbox")" $'[ws #1]\nno pane' "missing pane retains inbox"
T set -w -t "$FRESH_WIN" @ws_pane "$AGENT_PANE"
print '{}' | hook api/delivery "$FRESH_WIN" stop >/dev/null

# --- G2: send revalidates the target window after taking the hook lock ---
echo "ws send: a window closed while send waits for the hook lock resumes instead of stranding"
vanish_before="$(windows_of "$SEND_WT")"
"$WS" add api/delivery --agent claude --detach
VANISH_WIN="$(windows_of "$SEND_WT" | grep -vxF -f <(print -r -- "$vanish_before"))"
VANISH_ID="$(T display -p -t "$VANISH_WIN" '#{@ws_session}')"
assert_contains "$(wait_pane "$VANISH_WIN" PASTE-READY)" "PASTE-READY" "vanishing target TUI ready"
hook api/delivery "$VANISH_WIN" working </dev/null
VANISH_LOCK="${TMPDIR:-/tmp}/ws-hook-$WS_TMUX_SOCKET-$VANISH_WIN.lock"
(zmodload zsh/system; : >> "$VANISH_LOCK"; zsystem flock -f vanish_fd "$VANISH_LOCK"; : > "$TMP/vanish-held"; sleep 2) &
vanish_holder=$!
wait_until test -e "$TMP/vanish-held"
(WS_SESSION=1 "$WS" send "$VANISH_ID" "after vanish" >"$TMP/vanish.out" 2>&1; print $? > "$TMP/vanish.rc") &
vanish_sender=$!
sleep 1 # send resolved the live window and now blocks on the held hook lock
T kill-window -t "$VANISH_WIN"
wait "$vanish_holder"; wait "$vanish_sender"
assert_exit "$(cat "$TMP/vanish.rc")" "0" "send resumes a target whose window vanished before the hook lock"
wait_until pane_received "$VANISH_ID" $'[ws #1]\nafter vanish'
assert_eq "$(tail -1 "$WS_HOME/pane-$VANISH_ID.jsonl" 2>/dev/null | jq -r '.text')" $'[ws #1]\nafter vanish' "resumed target receives the message instead of stranding it"
# --- end G2 send revalidation ---

echo "ws send: done declared while send waits for the inbox lock is honored"
META="$WS_HOME/sessions/$FRESH_ID/meta"
cp "$META" "$TMP/fresh-meta"
: >> "$WS_HOME/sessions/$FRESH_ID/inbox.lock"
zsh -c 'zmodload zsh/system; zsystem flock "$1" && sleep 1.5' _ "$WS_HOME/sessions/$FRESH_ID/inbox.lock" &
race_lock_pid=$!
sleep 0.3
WS_SESSION=1 "$WS" send "$FRESH_ID" "after done" > "$TMP/race.out" 2>&1 &
race_send_pid=$!
sleep 0.3
jq '.status = "done"' "$META" > "$META.fixture" && mv "$META.fixture" "$META"
wait "$race_send_pid"
assert_exit "$?" "1" "send re-checks done under the inbox lock"
assert_contains "$(cat "$TMP/race.out")" "--force required" "racing send explains done guard"
assert_eq "$([[ -s "$WS_HOME/sessions/$FRESH_ID/inbox" ]] && print pending)" "" "unforced send never queues for a done session"
wait "$race_lock_pid"
cp "$TMP/fresh-meta" "$META"

echo "ws send: overlapping sends to a slow-starting tab never drop a message"
print 3.6 > "$WS_HOME/tui-delay"
"$WS" add api/delivery --agent claude --detach
SLOW_WIN="$(slot_window "$SEND_WT" 5)"
SLOW_ID="$(T display -p -t "$SLOW_WIN" '#{@ws_session}')"
WS_SESSION=1 "$WS" send "$SLOW_ID" "overlap-1" > "$TMP/overlap-1.log" 2>&1 &
overlap_1=$!
sleep 0.2
WS_SESSION=1 "$WS" send "$SLOW_ID" "overlap-2" > "$TMP/overlap-2.log" 2>&1 &
overlap_2=$!
wait "$overlap_1"; overlap_1_rc=$?
wait "$overlap_2"; overlap_2_rc=$?
for i in 1 2; do
  rc_var="overlap_${i}_rc"
  (( ${(P)rc_var} == 0 )) || assert_contains "$(cat "$TMP/overlap-$i.log")" "message queued" "overlap sender $i failure is reported"
done
overlap_seen() { [[ "$(cat "$WS_HOME/pane-$SLOW_ID.jsonl" "$WS_HOME/sessions/$SLOW_ID/inbox" 2>/dev/null)" == *overlap-1*overlap-2* ]]; }
wait_until overlap_seen
for i in 1 2; do
  assert_contains "$(cat "$WS_HOME/pane-$SLOW_ID.jsonl" "$WS_HOME/sessions/$SLOW_ID/inbox" 2>/dev/null)" "overlap-$i" "overlap message $i delivered or queued"
done
rm -f "$WS_HOME/tui-delay"

echo "ws send (F7): a message queued on a slow-starting idle tab is delivered without a re-send"
F7_BEFORE="$(windows_of "$SEND_WT")"
print 4 > "$WS_HOME/tui-delay"
"$WS" add api/delivery --agent claude --detach
F7_WIN="$(windows_of "$SEND_WT" | grep -vxF -f <(print -r -- "$F7_BEFORE"))"
F7_ID="$(T show -wqv -t "$F7_WIN" @ws_session)"
WS_READY_TIMEOUT=1 WS_DELIVER_RETRY_INTERVAL=1 WS_SESSION=1 "$WS" send "$F7_ID" "f7 late" >/dev/null 2>&1
assert_exit "$?" "75" "slow-start send reports queued"
wait_until pane_received "$F7_ID" "[ws #1]"$'\n'"f7 late"
assert_exit "$?" "0" "bounded retry delivers once the TUI is ready"
rm -f "$WS_HOME/tui-delay"

# --- delivery robustness: readiness and lock diagnostics ---
echo "ws send: readiness races release the inbox lock before waiting for input"
ROBUST_BEFORE="$(windows_of "$SEND_WT")"
print 8 > "$WS_HOME/tui-delay"
"$WS" add api/delivery --agent claude --detach
ROBUST_READY_WIN="$(windows_of "$SEND_WT" | grep -vxF -f <(print -r -- "$ROBUST_BEFORE"))"
ROBUST_READY_ID="$(T show -wqv -t "$ROBUST_READY_WIN" @ws_session)"
T set -w -t "$ROBUST_READY_WIN" @ws_main idle
ROBUST_LOCK="${TMPDIR:-/tmp}/ws-hook-$WS_TMUX_SOCKET-$ROBUST_READY_WIN.lock"
(zmodload zsh/system; : >> "$ROBUST_LOCK"; zsystem flock -f robust_fd "$ROBUST_LOCK"; : > "$TMP/robust-ready-held"; sleep 2.5) &
robust_holder=$!
wait_until test -e "$TMP/robust-ready-held"
(WS_SESSION=1 "$WS" send "$ROBUST_READY_ID" "after readiness race" > "$TMP/robust-ready.out" 2>&1; print $? > "$TMP/robust-ready.rc") &
robust_sender=$!
wait_until test -s "$WS_HOME/sessions/$ROBUST_READY_ID/inbox"
sleep 1 # sender has passed its unlocked main-state read and is waiting for the hook lock
T set -wu -t "$ROBUST_READY_WIN" @ws_main # races the sender's unlocked readiness check
wait "$robust_holder"
sleep 0.3 # sender now sees the missing main state under its hook/inbox locks
out="$(WS_SESSION="$ROBUST_READY_ID" "$WS" done 2>&1)"
assert_exit "$?" "1" "done refuses work while readiness waits"
assert_contains "$out" "pending messages" "readiness wait leaves the inbox lock available to done"
wait "$robust_sender"
assert_exit "$(cat "$TMP/robust-ready.rc")" "0" "send retries readiness after releasing its locks"
wait_until pane_received "$ROBUST_READY_ID" $'[ws #1]\nafter readiness race'
assert_exit "$?" "0" "unlocked readiness retry eventually delivers"
T kill-window -t "$ROBUST_READY_WIN"
rm -f "$WS_HOME/tui-delay"

echo "ws send: inbox lock timeout reports the lock instead of failing silently"
ROBUST_INBOX_LOCK="$WS_HOME/sessions/$SEND_ID/inbox.lock"
(zmodload zsh/system; : >> "$ROBUST_INBOX_LOCK"; zsystem flock -f robust_fd "$ROBUST_INBOX_LOCK"; : > "$TMP/robust-inbox-held"; sleep 4) &
robust_holder=$!
wait_until test -e "$TMP/robust-inbox-held"
out="$(WS_SESSION=1 "$WS" send "$SEND_ID" "lock timeout" 2>&1)"
assert_exit "$?" "1" "contended inbox lock times out"
assert_contains "$out" "timed out waiting for lock" "lock timeout emits a diagnostic"
wait "$robust_holder"
# --- end delivery robustness: readiness and lock diagnostics ---

echo "ws send: failed resume retains the message and reports it queued"
META="$WS_HOME/sessions/$FRESH_ID/meta"
jq --arg missing "$TMP/missing-workspace" '.path = $missing' "$META" > "$META.fixture"
mv "$META.fixture" "$META"
T kill-window -t "$FRESH_WIN"
out="$(WS_SESSION=1 "$WS" send "$FRESH_ID" "retain on failure" 2>&1)"
assert_exit "$?" "1" "missing workspace refuses resume"
assert_contains "$out" "message queued" "failure explains that delivery remains pending"
assert_eq "$(jq -r '.message' "$WS_HOME/sessions/$FRESH_ID/inbox")" $'[ws #1]\nretain on failure' "failed resume leaves a durable sender-prefixed message"

"$WS" rm api/delivery --force --delete-branch >/dev/null

echo "ws done: file output is durable; user-opened tab stays; show resolves names"
"$WS" new --repo "$REPO" --branch lifecycle --detach
DONE_WT="$WS_ROOT/api/lifecycle"
DONE_WIN="$(windows_of "$DONE_WT")"
assert_contains "$(wait_pane "$DONE_WIN" FAKE-claude)" "FAKE-claude" "lifecycle agent ready"
DONE_ID="$(T display -p -t "$DONE_WIN" '#{@ws_session}')"
WS_SESSION="$DONE_ID" "$WS" name finisher
printf '# Result\n\nquoted "value" $literal\n\n' > "$TMP/result file.md"
WS_SESSION="$DONE_ID" "$WS" done --output "$TMP/result file.md"
assert_exit "$?" "0" "done accepts output file"
cmp -s "$TMP/result file.md" "$WS_HOME/sessions/$DONE_ID/output.md"
assert_exit "$?" "0" "done preserves exact output bytes"
assert_eq "$("$WS" sessions "$DONE_ID" | cut -f6)" "done" "done records lifecycle status"
assert_eq "$(windows_of "$DONE_WT")" "$DONE_WIN" "user-opened tab stays after done"
assert_eq "$(jq -r '.event' "$WS_HOME/sessions/$DONE_ID/events")" "done" "done appends lifecycle event"
jq -e --arg id "$DONE_ID" --arg output "$WS_HOME/sessions/$DONE_ID/output.md" \
  '.id == $id and .output == $output and (.at | type == "string")' "$WS_HOME/sessions/$DONE_ID/events" >/dev/null
assert_exit "$?" "0" "done event identifies source, output and time"
out="$("$WS" show finisher)"
assert_exit "$?" "0" "show accepts session name"
assert_contains "$out" "status: done" "show reports lifecycle status"
assert_contains "$out" "output: $WS_HOME/sessions/$DONE_ID/output.md" "show reports durable output path"
echo "ws done: repeated declarations replace output, including stdin and empty default"
printf 'replacement\n\n' | WS_SESSION="$DONE_ID" "$WS" done -
assert_exit "$?" "0" "done accepts stdin shorthand"
assert_eq "$(cat "$WS_HOME/sessions/$DONE_ID/output.md"; print -n .)" $'replacement\n\n.' "repeat overwrites output preserving trailing newlines"
assert_eq "$(jq -sr 'map(.event) | join(",")' "$WS_HOME/sessions/$DONE_ID/events")" "done,done" "repeat appends another done event"
printf 'explicit stdin' | WS_SESSION="$DONE_ID" "$WS" done --output -
assert_exit "$?" "0" "done accepts explicit stdin output"
assert_eq "$(cat "$WS_HOME/sessions/$DONE_ID/output.md")" "explicit stdin" "explicit stdin replaces previous output"
WS_SESSION="$DONE_ID" "$WS" done
assert_exit "$?" "0" "done without output succeeds without reading stdin"
assert_eq "$(wc -c < "$WS_HOME/sessions/$DONE_ID/output.md" | tr -d ' ')" "0" "default done writes empty output"
assert_eq "$(jq -s length "$WS_HOME/sessions/$DONE_ID/events")" "4" "each declaration appends one event"

echo "ws done/show: invalid input leaves lifecycle intact"
for args in '--output' '--bad' '--output missing-file' 'extra argument'; do
  WS_SESSION="$DONE_ID" "$WS" done ${=args} 2>/dev/null
  assert_exit "$?" "1" "invalid done rejected: $args"
done
env -u WS_SESSION "$WS" done 2>/dev/null
assert_exit "$?" "1" "done outside a session is refused"
WS_SESSION=999999 "$WS" done 2>/dev/null
assert_exit "$?" "1" "done with unknown identity is refused"
assert_eq "$(jq -s length "$WS_HOME/sessions/$DONE_ID/events")" "4" "invalid declarations append no events"
assert_eq "$(wc -c < "$WS_HOME/sessions/$DONE_ID/output.md" | tr -d ' ')" "0" "invalid declarations preserve output"
"$WS" show no-such-session 2>/dev/null
assert_exit "$?" "1" "show rejects missing session"
"$WS" show 2>/dev/null
assert_exit "$?" "1" "show requires target"
out="$("$WS" send "$DONE_ID" should-refuse 2>&1)"
assert_exit "$?" "1" "send rejects session declared done through CLI"
assert_contains "$out" "--force required" "send explains done guard"

echo "ws lifecycle: agent exit emits its actual code and preserves done output"
lifecycle_exit_count() { [[ "$(jq -sr 'map(select(.event == "exit")) | length' "$WS_HOME/sessions/$1/events")" == "$2" ]]; }
lifecycle_done_stopped() { [[ "$("$WS" sessions "$1" | cut -f5,6)" == $'stopped\tdone' ]]; }
T send-keys -t "$DONE_WIN" Enter
wait_until lifecycle_exit_count "$DONE_ID" 1
assert_exit "$?" "0" "normal agent exit appends lifecycle event"
assert_eq "$(jq -sr 'map(select(.event == "exit")) | .[0].code' "$WS_HOME/sessions/$DONE_ID/events")" "0" "normal exit event records code zero"
# Exit publication precedes the launch wrapper's _end cleanup; wait for both transitions.
wait_until lifecycle_done_stopped "$DONE_ID"
assert_exit "$?" "0" "normal exit settles stopped with done lifecycle"
assert_eq "$("$WS" sessions "$DONE_ID" | cut -f5,6)" $'stopped\tdone' "agent exit preserves done lifecycle"
assert_contains "$("$WS" show "$DONE_ID")" "output: $WS_HOME/sessions/$DONE_ID/output.md" "output stays addressable after agent exit"
"$WS" open "$DONE_ID" --detach
DONE_WIN="$(windows_of "$DONE_WT")"
assert_contains "$(wait_pane "$DONE_WIN" FAKE-claude)" "FAKE-claude" "done identity can resume"
T send-keys -t "$DONE_WIN" 17 Enter
wait_until lifecycle_exit_count "$DONE_ID" 2
assert_exit "$?" "0" "failed agent exit appends lifecycle event"
assert_eq "$(jq -sr 'map(select(.event == "exit")) | .[1].code' "$WS_HOME/sessions/$DONE_ID/events")" "17" "failed exit retains actual nonzero code"
assert_contains "$(wait_pane "$DONE_WIN" 'agent exited (17)')" "agent exited (17)" "failure keeps existing close/resume behavior"
assert_eq "$(T display -p -t "$DONE_WIN" '#{@ws_session}')" "" "crashed tab no longer resolves as the live session"
T send-keys -t "$DONE_WIN" Enter

# A shell tab adopted with ws claude must report exits as well as ws new/add tabs.
MANUAL_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$DONE_WT" \
  "zsh -fc '\"$WS\" claude; echo EXIT=\$?; sleep 30'")"
assert_contains "$(wait_pane "$MANUAL_WIN" FAKE-claude)" "FAKE-claude" "manually opened agent ready"
MANUAL_ID="$(T display -p -t "$MANUAL_WIN" '#{@ws_session}')"
T send-keys -t "$MANUAL_WIN" Enter
assert_contains "$(wait_pane "$MANUAL_WIN" EXIT=0)" "EXIT=0" "adopted tab returns to shell"
assert_eq "$(jq -r '.event, .code' "$WS_HOME/sessions/$MANUAL_ID/events")" $'exit\n0' "adopted tab also emits actual process exit"
T kill-window -t "$MANUAL_WIN"

echo "ws lifecycle: closing a manually adopted tab reports SIGHUP too"
MANUAL_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$DONE_WT" \
  "zsh -fc '\"$WS\" claude; sleep 30'")"
assert_contains "$(wait_pane "$MANUAL_WIN" FAKE-claude)" "FAKE-claude" "adopted closure fixture ready"
MANUAL_ID="$(T display -p -t "$MANUAL_WIN" '#{@ws_session}')"
T kill-window -t "$MANUAL_WIN"
wait_until lifecycle_exit_count "$MANUAL_ID" 1
assert_exit "$?" "0" "closing adopted tab emits termination"
jq -se 'length == 1 and .[0].event == "exit" and (.[0].code == 0 or .[0].code == 129)' "$WS_HOME/sessions/$MANUAL_ID/events" >/dev/null
assert_exit "$?" "0" "adopted closure records actual EOF or SIGHUP exit once"

echo "ws done: spawned agent closes its own tab after publishing output"
"$WS" add api/lifecycle --detach
SPAWN_WIN="$(windows_of "$DONE_WT")"
assert_contains "$(wait_pane "$SPAWN_WIN" FAKE-claude)" "FAKE-claude" "spawn fixture ready"
SPAWN_ID="$(T display -p -t "$SPAWN_WIN" '#{@ws_session}')"
META="$WS_HOME/sessions/$SPAWN_ID/meta"
jq '.spawned = true | .resume_id = "spawn-resume"' "$META" > "$META.fixture"
mv "$META.fixture" "$META"
T send-keys -t "$SPAWN_WIN" done Enter
spawn_done_closed() { [[ "$("$WS" sessions "$1" | cut -f5,6)" == $'stopped\tdone' ]]; }
wait_until spawn_done_closed "$SPAWN_ID"
assert_exit "$?" "0" "spawned tab closes when its agent declares done"
assert_eq "$(awk -F '\t' -v id="$SPAWN_ID" '$4 == id' "$(git -C "$DONE_WT" rev-parse --absolute-git-dir)/ws-sessions")" "" "spawned done frees its workspace slot row"
assert_eq "$(cat "$WS_HOME/sessions/$SPAWN_ID/output.md")" "agent result" "spawned output is durable before tab closes"
assert_eq "$(jq -sr 'map(select(.event == "done")) | length' "$WS_HOME/sessions/$SPAWN_ID/events")" "1" "spawned completion appends one done event"
assert_eq "$("$WS" sessions "$SPAWN_ID" | cut -f6)" "done" "closing spawned tab preserves done status"
assert_contains "$("$WS" show "$SPAWN_ID")" "output: $WS_HOME/sessions/$SPAWN_ID/output.md" "closed spawned result is addressable"
# G2: done's self-kill is cleanup, not a crash: no exit event (would fire `ws on X exit`).
agent_gone() { ! pgrep -f "_agent $1 " >/dev/null; }
wait_until agent_gone "$SPAWN_ID"
assert_exit "$?" "0" "spawned agent wrapper finishes after done closure"
assert_eq "$(jq -sr 'map(.event) | join(",")' "$WS_HOME/sessions/$SPAWN_ID/events")" "done" "spawned done closure records one done and no exit"
"$WS" open "$SPAWN_ID" --detach
SPAWN_WIN="$(windows_of "$DONE_WT")"
assert_contains "$(wait_pane "$SPAWN_WIN" FAKE-claude)" "--resume spawn-resume" "closed spawned identity retains agent resume data"
T send-keys -t "$SPAWN_WIN" Enter
wait_until lifecycle_exit_count "$SPAWN_ID" 1
assert_exit "$?" "0" "resumed spawned session still reports its own exit"
assert_eq "$(jq -sr 'map(.event) | join(",")' "$WS_HOME/sessions/$SPAWN_ID/events")" "done,exit" "resumed exit follows the earlier done"

echo "ws lifecycle: a crashed spawned session reports one nonzero exit and no done"
"$WS" add api/lifecycle --detach
CRASH_WIN="$(windows_of "$DONE_WT")"
assert_contains "$(wait_pane "$CRASH_WIN" FAKE-claude)" "FAKE-claude" "spawned crash fixture ready"
CRASH_ID="$(T display -p -t "$CRASH_WIN" '#{@ws_session}')"
META="$WS_HOME/sessions/$CRASH_ID/meta"
jq '.spawned = true' "$META" > "$META.fixture"
mv "$META.fixture" "$META"
T send-keys -t "$CRASH_WIN" 17 Enter
wait_until lifecycle_exit_count "$CRASH_ID" 1
assert_exit "$?" "0" "spawned crash emits exit"
assert_contains "$(wait_pane "$CRASH_WIN" 'agent exited (17)')" "agent exited (17)" "spawned crash keeps close prompt"
assert_eq "$(jq -sr 'map("\(.event):\(.code // "")") | join(",")' "$WS_HOME/sessions/$CRASH_ID/events")" "exit:17" "spawned crash records single nonzero exit, no done"
T send-keys -t "$CRASH_WIN" Enter

echo "ws lifecycle: tab closure reports exit while a TUI still holds the tty"
"$WS" new --repo "$REPO" --branch delivery --detach
HUP_WT="$WS_ROOT/api/delivery"
HUP_WIN="$(windows_of "$HUP_WT")"
assert_contains "$(wait_pane "$HUP_WIN" PASTE-READY)" "PASTE-READY" "closure TUI ready"
HUP_ID="$(T display -p -t "$HUP_WIN" '#{@ws_session}')"
HUP_PID="$(T display -p -t "$HUP_WIN" '#{pane_pid}')"
# Retain only this private fixture's direct children for cleanup if the exit contract fails.
HUP_CHILDREN=( ${(f)"$(ps -Ao pid,ppid | awk -v p="$HUP_PID" '$2 == p {print $1}')"} )
T kill-window -t "$HUP_WIN"
wait_until lifecycle_exit_count "$HUP_ID" 1
assert_exit "$?" "0" "TUI closure emits an exit without waiting for terminal reads"
assert_eq "$(jq -r '.event, .code' "$WS_HOME/sessions/$HUP_ID/events")" $'exit\n129' "TUI closure records SIGHUP once"
kill -KILL "$HUP_PID" "${HUP_CHILDREN[@]}" 2>/dev/null
"$WS" rm api/delivery --force --delete-branch >/dev/null

echo "ws lifecycle: interactive launch shells keep the crash prompt and actual exit code"
for agent in claude codex grok; do
  WS_AGENT_SHELL="zsh -fic" "$WS" new --repo "$REPO" --branch "interactive-$agent" --agent "$agent" --detach
  INTERACTIVE_WIN="$(windows_of "$WS_ROOT/api/interactive-$agent")"
  assert_contains "$(wait_pane "$INTERACTIVE_WIN" "FAKE-$agent")" "FAKE-$agent" "$agent interactive shell starts TUI"
  INTERACTIVE_ID="$(T display -p -t "$INTERACTIVE_WIN" '#{@ws_session}')"
  T send-keys -t "$INTERACTIVE_WIN" 17 Enter
  assert_contains "$(wait_pane "$INTERACTIVE_WIN" 'agent exited (17)')" 'agent exited (17)' "$agent interactive shell keeps crash prompt"
  assert_eq "$(jq -r '.event, .code' "$WS_HOME/sessions/$INTERACTIVE_ID/events")" $'exit\n17' "$agent interactive shell records actual exit"
  "$WS" rm "api/interactive-$agent" --force --delete-branch >/dev/null
done

# --- exit path ---
echo "ws exit path: every exit stops delivery before slow exit actions run"
"$WS" new --repo "$REPO" --branch exit-path --detach
EXIT_WT="$WS_ROOT/api/exit-path"
"$WS" new --repo "$REPO" --branch delivery --detach
EXIT_TARGET_WIN="$(windows_of "$WS_ROOT/api/delivery")"
assert_contains "$(wait_pane "$EXIT_TARGET_WIN" PASTE-READY)" "PASTE-READY" "exit action target ready"
EXIT_TARGET_ID="$(T display -p -t "$EXIT_TARGET_WIN" '#{@ws_session}')"
for exit_code in 0 17; do
  [[ "$exit_code" == 0 ]] || "$WS" add api/exit-path --detach
  EXIT_WIN="$(windows_of "$EXIT_WT")"
  assert_contains "$(wait_pane "$EXIT_WIN" FAKE-claude)" "FAKE-claude" "exit $exit_code source ready"
  EXIT_ID="$(T display -p -t "$EXIT_WIN" '#{@ws_session}')"
  hook api/exit-path "$EXIT_WIN" stop </dev/null >/dev/null
  T kill-window -t "$EXIT_TARGET_WIN"
  wait_until lifecycle_exit_count "$EXIT_TARGET_ID" "$(( exit_code == 0 ? 1 : 2 ))"
  EXIT_TID="$("$WS" on "$EXIT_ID" exit --send-to "$EXIT_TARGET_ID" "exit action $exit_code")"
  # Keep the exit action waiting for its resumed TUI while a concurrent send runs.
  print 6 > "$WS_HOME/tui-delay"
  # Resume is deliberately unavailable: send must retain the message, never paste into
  # the dead source. This private fixture releases the lock immediately after send.
  EXIT_OPEN_LOCK="$WS_HOME/sessions/$EXIT_ID/open.lock"
  : >> "$EXIT_OPEN_LOCK"
  zmodload zsh/system
  zsystem flock -f exit_open_fd "$EXIT_OPEN_LOCK"
  T send-keys -t "$EXIT_WIN" "$exit_code" Enter
  wait_until lifecycle_exit_count "$EXIT_ID" 1
  assert_exit "$?" "0" "exit $exit_code publishes while its action is still waiting"
  assert_eq "$(T show -wqv -t "$EXIT_WIN" @ws_session)" "" "exit $exit_code source is no longer deliverable"
  assert_eq "$(T show -wqv -t "$EXIT_WIN" @ws_main)" "" "exit $exit_code clears its idle delivery tag"
  out="$("$WS" send "$EXIT_ID" "during exit $exit_code" 2>&1)"
  assert_exit "$?" "75" "exit $exit_code send reports unavailable resume"
  assert_contains "$(cat "$WS_HOME/sessions/$EXIT_ID/inbox")" "during exit $exit_code" "exit $exit_code send stays durable"
  exec {exit_open_fd}>&-
  wait_until pane_received "$EXIT_TARGET_ID" "[ws terminal]"$'\n'"exit action $exit_code"
  assert_exit "$?" "0" "exit $exit_code action finishes after delivery guard"
  EXIT_TARGET_WIN="$(jq -r '.window' "$WS_HOME/sessions/$EXIT_TARGET_ID/meta")"
  if [[ "$exit_code" == 17 ]]; then
    assert_contains "$(wait_pane "$EXIT_WIN" 'agent exited (17)')" 'agent exited (17)' "exit guard preserves crash prompt"
    T send-keys -t "$EXIT_WIN" Enter
  fi
done
echo "ws exit path: an old wrapper cannot untag a resumed or reassigned window"
"$WS" add api/exit-path --detach
EXIT_OLD_WIN="$(windows_of "$EXIT_WT")"
assert_contains "$(wait_pane "$EXIT_OLD_WIN" FAKE-claude)" "FAKE-claude" "old generation ready"
EXIT_ID="$(T display -p -t "$EXIT_OLD_WIN" '#{@ws_session}')"
T set -wu -t "$EXIT_OLD_WIN" @ws_session
"$WS" open "$EXIT_ID" --detach
EXIT_NEW_WIN="$(jq -r '.window' "$WS_HOME/sessions/$EXIT_ID/meta")"
assert_contains "$(wait_pane "$EXIT_NEW_WIN" FAKE-claude)" "FAKE-claude" "new generation ready"
hook api/exit-path "$EXIT_NEW_WIN" stop </dev/null >/dev/null
# Reassign the old tab's delivery tags too: this wrapper no longer owns either tag.
T set -w -t "$EXIT_OLD_WIN" @ws_session "$EXIT_TARGET_ID" \; set -w -t "$EXIT_OLD_WIN" @ws_main working
T send-keys -t "$EXIT_OLD_WIN" 17 Enter
wait_until lifecycle_exit_count "$EXIT_ID" 1
assert_exit "$?" "0" "old generation exit still publishes"
assert_eq "$(T show -wqv -t "$EXIT_NEW_WIN" @ws_session)" "$EXIT_ID" "old exit preserves resumed session tag"
assert_eq "$(T show -wqv -t "$EXIT_NEW_WIN" @ws_main)" idle "old exit preserves resumed delivery state"
assert_eq "$(T show -wqv -t "$EXIT_OLD_WIN" @ws_session)" "$EXIT_TARGET_ID" "old exit preserves reassigned session tag"
assert_eq "$(T show -wqv -t "$EXIT_OLD_WIN" @ws_main)" working "old exit preserves reassigned delivery state"
T kill-window -t "$EXIT_OLD_WIN"

echo "ws exit path: old wrapper cleanup spares a session resumed in a new window"
"$WS" new --repo "$REPO" --branch exit-gen --detach
EXIT_GEN_WT="$WS_ROOT/api/exit-gen"
EXIT_GEN_WIN="$(windows_of "$EXIT_GEN_WT")"
EXIT_GEN_ID="$(T display -p -t "$EXIT_GEN_WIN" '#{@ws_session}')"
EXIT_GEN_SLOT="$(T display -p -t "$EXIT_GEN_WIN" '#{@ws_slot}')"
EXIT_GEN_META="$WS_HOME/sessions/$EXIT_GEN_ID/meta"
EXIT_GEN_ROWS="$(git -C "$EXIT_GEN_WT" rev-parse --absolute-git-dir)/ws-sessions"
# Simulate `ws open` re-pointing the ID to a new window while the old wrapper waits.
jq '.window = "@resumed"' "$EXIT_GEN_META" > "$EXIT_GEN_META.tmp" && mv "$EXIT_GEN_META.tmp" "$EXIT_GEN_META"
"$WS" _end "$EXIT_GEN_WT" "$EXIT_GEN_SLOT" "$EXIT_GEN_ID" "$EXIT_GEN_WIN"
assert_eq "$(jq -r '.window, .status' "$EXIT_GEN_META")" $'@resumed\nactive' "stale cleanup keeps resumed meta"
assert_eq "$(awk -F '\t' -v s="$EXIT_GEN_SLOT" '$1 == s {print $4}' "$EXIT_GEN_ROWS")" "$EXIT_GEN_ID" "stale cleanup keeps resumed slot row"
"$WS" _end "$EXIT_GEN_WT" "$EXIT_GEN_SLOT" "$EXIT_GEN_ID" "@resumed"
assert_eq "$(jq -r '.window, .status' "$EXIT_GEN_META")" $'\nclosed' "matching-generation cleanup clears meta"
assert_eq "$(awk -F '\t' -v s="$EXIT_GEN_SLOT" '$1 == s' "$EXIT_GEN_ROWS")" "" "matching-generation cleanup removes slot row"
"$WS" rm api/exit-gen --force --delete-branch >/dev/null

# --- G1: generation/ownership gates for adopted tabs and late or nested hooks ---
echo "ws claude: adopted-tab clean exit spares a session resumed in a new window"
"$WS" new --repo "$REPO" --branch run-gen --detach
RUN_GEN_WT="$WS_ROOT/api/run-gen"
RUN_GEN_ROWS="$(git -C "$RUN_GEN_WT" rev-parse --absolute-git-dir)/ws-sessions"
RUN_GEN_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$RUN_GEN_WT" \
  "zsh -fc '\"$WS\" claude; echo EXIT=\$?; sleep 30'")"
assert_contains "$(wait_pane "$RUN_GEN_WIN" FAKE-claude)" "FAKE-claude" "adopted generation ready"
RUN_GEN_ID="$(T show -wqv -t "$RUN_GEN_WIN" @ws_session)"
RUN_GEN_SLOT="$(T show -wqv -t "$RUN_GEN_WIN" @ws_slot)"
RUN_GEN_META="$WS_HOME/sessions/$RUN_GEN_ID/meta"
jq '.window = "@resumed"' "$RUN_GEN_META" > "$RUN_GEN_META.tmp" && mv "$RUN_GEN_META.tmp" "$RUN_GEN_META"
T send-keys -t "$RUN_GEN_WIN" Enter
assert_contains "$(wait_pane "$RUN_GEN_WIN" EXIT=0)" "EXIT=0" "adopted old generation exits cleanly"
assert_eq "$(jq -r '.window, .status' "$RUN_GEN_META")" $'@resumed\nactive' "adopted exit keeps resumed meta"
assert_eq "$(awk -F '\t' -v s="$RUN_GEN_SLOT" '$1 == s {print $4}' "$RUN_GEN_ROWS")" "$RUN_GEN_ID" "adopted exit keeps resumed slot row"
T kill-window -t "$RUN_GEN_WIN"

echo "ws hook: a late Stop after exit cleanup never drains the (resumed) global inbox"
RUN_GEN_WIN="$(windows_of "$RUN_GEN_WT" | head -1)"
assert_contains "$(wait_pane "$RUN_GEN_WIN" FAKE-claude)" "FAKE-claude" "late-stop fixture ready"
LATE_ID="$(T show -wqv -t "$RUN_GEN_WIN" @ws_session)"
LATE_META="$WS_HOME/sessions/$LATE_ID/meta"
T set -wu -t "$RUN_GEN_WIN" @ws_session # exit cleanup untagged the old pane
print -r -- '{"message":"for the resumed pane"}' >> "$WS_HOME/sessions/$LATE_ID/inbox"
for late_win in "@resumed" ""; do
  jq --arg w "$late_win" '.window = $w' "$LATE_META" > "$LATE_META.tmp" && mv "$LATE_META.tmp" "$LATE_META"
  reply="$(print '{}' | WS_SESSION="$LATE_ID" hook api/run-gen "$RUN_GEN_WIN" stop)"
  assert_eq "$reply" "{}" "late Stop (meta.window='$late_win') answers {}"
  assert_contains "$(cat "$WS_HOME/sessions/$LATE_ID/inbox")" "for the resumed pane" "late Stop (meta.window='$late_win') leaves inbox"
done
jq --arg w "$RUN_GEN_WIN" '.window = $w' "$LATE_META" > "$LATE_META.tmp" && mv "$LATE_META.tmp" "$LATE_META"
T set -w -t "$RUN_GEN_WIN" @ws_session "$LATE_ID"
: > "$WS_HOME/sessions/$LATE_ID/inbox"

echo "ws hook: a nested agent with inherited tab env cannot drain the parent's inbox"
mkdir -p "$TMP/nest"
cat > "$TMP/nest/claude" <<EOF
#!/bin/sh
while read line; do
  case "\$line" in
  nested) sh -c '"$TMP/nest/grok"; :' ;; # tool shell runs a headless agent
  direct) printf '{}' | "\$WS_TEST_CLI" hook stop > "\$WS_HOME/nest-direct" ;;
  esac
done
EOF
cat > "$TMP/nest/grok" <<'EOF'
#!/bin/sh
printf '{}' | "$WS_TEST_CLI" hook stop > "$WS_HOME/nest-nested"
EOF
chmod +x "$TMP/nest/claude" "$TMP/nest/grok"
NEST_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$RUN_GEN_WT" \
  -e "WS_WORKSPACE=api/run-gen" -e "WS_SESSION=$LATE_ID" -e "WS_HOME=$WS_HOME" -e "WS_TEST_CLI=$WS" "$TMP/nest/claude")"
T set -w -t "$NEST_WIN" @ws_session "$LATE_ID" \; set -w -t "$NEST_WIN" @ws_path "$RUN_GEN_WT" \; set -w -t "$NEST_WIN" @ws_slot 1
jq --arg w "$NEST_WIN" '.window = $w' "$LATE_META" > "$LATE_META.tmp" && mv "$LATE_META.tmp" "$LATE_META"
print -r -- '{"message":"for the parent agent"}' >> "$WS_HOME/sessions/$LATE_ID/inbox"
T send-keys -t "$NEST_WIN" nested Enter
wait_until test -s "$WS_HOME/nest-nested"
assert_eq "$(cat "$WS_HOME/nest-nested")" "{}" "nested agent Stop answers {}"
assert_contains "$(cat "$WS_HOME/sessions/$LATE_ID/inbox")" "for the parent agent" "nested agent Stop leaves parent inbox"
T send-keys -t "$NEST_WIN" direct Enter
wait_until test -s "$WS_HOME/nest-direct"
assert_eq "$(jq -r '.reason' "$WS_HOME/nest-direct" 2>/dev/null)" "for the parent agent" "tab agent's own Stop still drains"
T kill-window -t "$NEST_WIN"
"$WS" rm api/run-gen --force --delete-branch >/dev/null

echo "ws exit path: HUP stops delivery even when the pane remains open"
"$WS" new --repo "$REPO" --branch exit-hup --detach
EXIT_HUP_WIN="$(windows_of "$WS_ROOT/api/exit-hup")"
assert_contains "$(wait_pane "$EXIT_HUP_WIN" FAKE-claude)" "FAKE-claude" "HUP source ready"
EXIT_HUP_ID="$(T display -p -t "$EXIT_HUP_WIN" '#{@ws_session}')"
hook api/exit-hup "$EXIT_HUP_WIN" stop </dev/null >/dev/null
EXIT_PANE_PID="$(T display -p -t "$EXIT_HUP_WIN" '#{pane_pid}')"
EXIT_WRAPPER_PID="$(ps -Ao pid,ppid,command | awk -v p="$EXIT_PANE_PID" -v id="$EXIT_HUP_ID" '$2 == p && index($0, "_agent " id " ") {print $1}')"
[[ "$EXIT_WRAPPER_PID" != <-> ]] || kill -HUP "$EXIT_WRAPPER_PID"
wait_until lifecycle_exit_count "$EXIT_HUP_ID" 1
assert_exit "$?" "0" "HUP publishes its lifecycle event"
assert_eq "$(jq -r '.code' "$WS_HOME/sessions/$EXIT_HUP_ID/events")" 129 "HUP records termination code"
assert_eq "$(T show -wqv -t "$EXIT_HUP_WIN" @ws_session)" "" "HUP clears session delivery tag"
assert_eq "$(T show -wqv -t "$EXIT_HUP_WIN" @ws_main)" "" "HUP clears idle delivery tag"
"$WS" rm api/exit-hup --force --delete-branch >/dev/null
"$WS" rm api/exit-path --force --delete-branch >/dev/null
"$WS" rm api/delivery --force --delete-branch >/dev/null

echo "ws exit path: agent children inherit default SIGINT and SIGQUIT"
# Replace only this private fixture's executable; running agents keep the old inode.
mv "$TMP/bin/claude" "$TMP/bin/claude-exit-normal"
cat > "$TMP/bin/claude" <<'SIGNAL_AGENT'
#!/usr/bin/env python3
import os, signal
print("SIGNAL-READY", flush=True)
os.kill(os.getpid(), getattr(signal, "SIG" + input().strip()))
print("SIGNAL-SURVIVED", flush=True)
SIGNAL_AGENT
chmod +x "$TMP/bin/claude"
for signal in INT QUIT; do
  signal_code=130
  [[ "$signal" != QUIT ]] || signal_code=131
  signal_branch="exit-signal-${(L)signal}"
  "$WS" new --repo "$REPO" --branch "$signal_branch" --detach
  SIGNAL_WIN="$(windows_of "$WS_ROOT/api/$signal_branch")"
  assert_contains "$(wait_pane "$SIGNAL_WIN" SIGNAL-READY)" SIGNAL-READY "$signal agent ready"
  SIGNAL_ID="$(T display -p -t "$SIGNAL_WIN" '#{@ws_session}')"
  T send-keys -t "$SIGNAL_WIN" "$signal" Enter
  wait_until lifecycle_exit_count "$SIGNAL_ID" 1
  assert_exit "$?" "0" "$signal agent termination publishes once"
  assert_eq "$(jq -r '.code' "$WS_HOME/sessions/$SIGNAL_ID/events")" "$signal_code" "$signal retains its default termination disposition"
  "$WS" rm "api/$signal_branch" --force --delete-branch >/dev/null
done
mv "$TMP/bin/claude-exit-normal" "$TMP/bin/claude"
# --- end exit path ---

# --- M1 done vs pending inbox ---
echo "ws done: a spawned session keeps accepted messages deliverable until Stop"
M1_TID="$(WS_SESSION="$DONE_ID" "$WS" on "$DONE_ID" done --spawn 'Finish after checking follow-ups')"
assert_exit "$?" "0" "M1 spawns a tracked child through a done trigger"
M1_ID="$(jq -r '.spawned_session' "$WS_HOME/triggers/archive/$M1_TID.json")"
M1_WIN="$(jq -r '.window' "$WS_HOME/sessions/$M1_ID/meta")"
assert_contains "$(wait_pane "$M1_WIN" FAKE-claude)" "FAKE-claude" "M1 spawned agent ready"
hook api/lifecycle "$M1_WIN" working </dev/null
WS_SESSION="$DONE_ID" "$WS" send "$M1_ID" 'Check this follow-up before finishing'
assert_exit "$?" "0" "M1 working child accepts follow-up"
out="$(printf 'premature result\n' | WS_SESSION="$M1_ID" "$WS" done - 2>&1)"
assert_exit "$?" "1" "M1 done refuses accepted but undelivered work"
assert_contains "$out" "pending messages" "M1 refusal explains why completion must wait"
assert_contains "$out" "Stop hook" "M1 refusal tells the agent how to receive pending work"
assert_contains "$("$WS" show "$M1_ID")" "status: active" "M1 refused done preserves active lifecycle"
assert_eq "$(T display -p -t "$M1_WIN" '#{@ws_session}' 2>/dev/null)" "$M1_ID" "M1 refused done keeps spawned tab alive"
assert_eq "$(jq -s length "$WS_HOME/sessions/$M1_ID/events")" "0" "M1 refused done publishes no lifecycle event"
assert_eq "$([[ -e "$WS_HOME/sessions/$M1_ID/output.md" ]] && print yes)" "" "M1 refused done publishes no premature output"
reply="$(hook api/lifecycle "$M1_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$DONE_ID]"$'\nCheck this follow-up before finishing' "M1 Stop delivers the accepted follow-up"
echo "ws done: an interrupted Stop reply still prevents spawned completion"
WS_SESSION="$DONE_ID" "$WS" send "$M1_ID" 'Recover this interrupted reply'
assert_exit "$?" "0" "M1 child accepts work before an interrupted Stop"
hook api/lifecycle "$M1_WIN" stop </dev/null 2>/dev/null | true # close stdout before the reply lands
out="$(WS_SESSION="$M1_ID" "$WS" done 2>&1)"
assert_exit "$?" "1" "M1 done refuses an unacked in-flight reply even with an empty inbox"
assert_contains "$out" "pending messages" "M1 unacked reply explains refusal"
assert_eq "$(T display -p -t "$M1_WIN" '#{@ws_session}' 2>/dev/null)" "$M1_ID" "M1 unacked reply keeps spawned tab alive"
reply="$(hook api/lifecycle "$M1_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$DONE_ID]"$'\nRecover this interrupted reply' "M1 Stop redelivers the unacked follow-up"
T send-keys -t "$M1_WIN" done Enter
wait_until spawn_done_closed "$M1_ID"
assert_exit "$?" "0" "M1 spawned child completes and closes after delivery"
assert_eq "$(cat "$WS_HOME/sessions/$M1_ID/output.md")" "agent result" "M1 successful completion publishes final output"
# --- end M1 done vs pending inbox ---

"$WS" rm api/lifecycle --force --delete-branch >/dev/null

echo "ws on: fan-in waits for all sources and expands outputs in source order"
"$WS" new --repo "$REPO" --branch triggers --detach
TRIGGER_WT="$WS_ROOT/api/triggers"
SOURCE_A_WIN="$(windows_of "$TRIGGER_WT")"
SOURCE_A="$(T display -p -t "$SOURCE_A_WIN" '#{@ws_session}')"
"$WS" add api/triggers --detach
SOURCE_B_WIN="$(slot_window "$TRIGGER_WT" 2)"
SOURCE_B="$(T display -p -t "$SOURCE_B_WIN" '#{@ws_session}')"
"$WS" add api/triggers --detach
TARGET_WIN="$(slot_window "$TRIGGER_WT" 3)"
TARGET_ID="$(T display -p -t "$TARGET_WIN" '#{@ws_session}')"
assert_contains "$(wait_pane "$TARGET_WIN" FAKE-claude)" "FAKE-claude" "trigger target ready"
hook api/triggers "$TARGET_WIN" working </dev/null
TID="$(WS_SESSION="$TARGET_ID" "$WS" on "$SOURCE_A,$SOURCE_B" done --send-to me 'results {id}: {output}')"
assert_exit "$?" "0" "fan-in trigger registers"
assert_contains "$("$WS" triggers)" "$TID"$'\t'"$SOURCE_A,$SOURCE_B"$'\tdone\tsend' "pending trigger is listed"
printf 'alpha "quoted" $literal' | WS_SESSION="$SOURCE_A" "$WS" done -
assert_exit "$?" "0" "first source completion publishes"
reply="$(hook api/triggers "$TARGET_WIN" stop </dev/null)"
assert_eq "$reply" "{}" "first source alone does not deliver"
hook api/triggers "$TARGET_WIN" working </dev/null
printf beta | WS_SESSION="$SOURCE_B" "$WS" done -
assert_exit "$?" "0" "last source completion publishes and fires"
reply="$(hook api/triggers "$TARGET_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$TARGET_ID]"$'\n'"results $SOURCE_A,$SOURCE_B: alpha \"quoted\" \$literal"$'\n'"beta" "fan-in expands literal output text and ordered ids"
assert_eq "$("$WS" triggers)" "" "fired trigger is no longer pending"
echo "ws on: already-satisfied registration fires once, even when events repeat concurrently"
hook api/triggers "$TARGET_WIN" working </dev/null
SATISFIED_TID="$(WS_SESSION="$TARGET_ID" "$WS" on "$SOURCE_A,$SOURCE_B" done --send-to me 'already {id}')"
assert_exit "$?" "0" "already-done fan-in registers and fires immediately"
repeat_pids=()
for source_id in "$SOURCE_A" "$SOURCE_B"; do
  WS_SESSION="$source_id" "$WS" done > "$TMP/repeat-$source_id.log" 2>&1 &
  repeat_pids+=($!)
done
for pid in $repeat_pids; do wait "$pid"; assert_exit "$?" "0" "concurrent repeat completion succeeds"; done
reply="$(hook api/triggers "$TARGET_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$TARGET_ID]"$'\n'"already $SOURCE_A,$SOURCE_B" "already-satisfied trigger delivers exactly once"
assert_eq "$(hook api/triggers "$TARGET_WIN" stop </dev/null)" "{}" "repeat completion never queues another action"

echo "ws on: send triggers reach done targets; registration is the explicit override"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
WS_SESSION="$TARGET_ID" "$WS" on "$SOURCE_B" done --send-to "$SOURCE_A" 'to done {id}' >/dev/null
assert_exit "$?" "0" "trigger to done target fires"
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$TARGET_ID]"$'\n'"to done $SOURCE_B" "done target receives trigger send"

echo "ws stop: in-flight inbox from an interrupted Stop is redelivered first"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
print -r -- '{"message":"stale"}' > "$WS_HOME/sessions/$SOURCE_A/inbox.inflight"
print -rn -- fresh | "$WS" send --force "$SOURCE_A" - 2>/dev/null
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'stale\n\n[ws terminal]\nfresh' "stale in-flight messages precede new ones"
assert_eq "$([[ -e "$WS_HOME/sessions/$SOURCE_A/inbox.inflight" ]] && print yes)" "" "in-flight file removed after reply"

# --- G1: Stop delivery ack + resume-id ownership ---
inflight_count() { ls "$WS_HOME/sessions/$1" | grep -c '^inbox\.inflight'; }
echo "ws stop: a reply that never reached the agent stays in flight and is redelivered"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
print -rn -- lost-reply | "$WS" send --force "$SOURCE_A" - 2>/dev/null
hook api/triggers "$SOURCE_A_WIN" stop </dev/null 2>/dev/null | head -c 0 # hook dies (SIGPIPE) writing its reply
assert_eq "$(inflight_count "$SOURCE_A")" "1" "unwritten reply keeps its in-flight file"
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'[ws terminal]\nlost-reply' "next Stop redelivers the unacked reply"
assert_eq "$(inflight_count "$SOURCE_A")" "0" "written reply acks its in-flight file"

echo "ws stop: a live Stop's in-flight messages are never reclaimed by a concurrent send"
sleep 30 &
owner_pid=$!
print -r -- '{"message":"owned"}' > "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$owner_pid"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
print -rn -- concurrent | "$WS" send --force "$SOURCE_A" - 2>/dev/null
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'[ws terminal]\nconcurrent' "live owner's messages are not redelivered"
assert_eq "$(cat "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$owner_pid")" '{"message":"owned"}' "live owner's in-flight file untouched"
kill "$owner_pid"; wait "$owner_pid" 2>/dev/null
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "owned" "dead owner's in-flight messages are redelivered"
assert_eq "$(inflight_count "$SOURCE_A")" "0" "reclaimed in-flight file removed"

echo "ws hook: a stale hook never rewrites a reused slot's resume row"
# resume_row <pane-session-id> <sid> — sessions-file row after record_resume_id for slot 1.
resume_row() {
  ( source "$WS"
    sessions_file() { print -r -- "$TMP/g1-ws-sessions"; }
    tmux_() {
      case "$*" in
        *@ws_path*) print -r -- "$TMP/g1-wt" ;;
        *@ws_slot*) print 1 ;;
        *@ws_agent*) print claude ;;
        *@ws_session*) print -r -- "$G1_PANE_ID" ;;
      esac
    }
    G1_PANE_ID="$1" TMUX_PANE=%0 record_resume_id "{\"session_id\":\"$2\"}"
    cat "$TMP/g1-ws-sessions" )
}
( source "$WS"
  f="$TMP/g1-ws-sessions"
  session_append "$f" claude "$TMP/g1-wt" >/dev/null # slot 1 → old session
  session_remove_locked "$f" 1
  session_append "$f" claude "$TMP/g1-wt" >/dev/null ) # slot 1 reused → new session
G1_OLD="$(( $(cat "$WS_HOME/sessions/counter") - 1 ))"
G1_NEW="$(cat "$WS_HOME/sessions/counter")"
assert_eq "$(resume_row "$G1_OLD" sid-old)" "1	claude		$G1_NEW" "stale hook leaves the new owner's row intact"
assert_eq "$(resume_row "$G1_NEW" sid-new)" "1	claude	sid-new	$G1_NEW" "current owner records its resume id"
assert_eq "$(resume_row "$G1_OLD" sid-old)" "1	claude	sid-new	$G1_NEW" "stale hook never replaces a recorded owner"
rm -f "$TMP/g1-ws-sessions" "$TMP/g1-ws-sessions.lock"
# --- end G1 ---

# --- G3b: stale-launch hooks, pid-reuse inflight reclaim, Stop lock budget ---
g3b_resume() { jq -r '.resume_id // empty' "$WS_HOME/sessions/$1/meta"; }
echo "ws hook: a delayed hook from an older launch on a reused pane touches nothing"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
print -rn -- for-new-launch | "$WS" send --force "$SOURCE_A" - 2>/dev/null
G3B_INBOX="$(cat "$WS_HOME/sessions/$SOURCE_A/inbox")"
G3B_RESUME="$(g3b_resume "$SOURCE_A")"
reply="$(print -r -- '{"session_id":"sid-stale-launch"}' | WS_SESSION="$SOURCE_B" hook api/triggers "$SOURCE_A_WIN" stop)"
assert_eq "$reply" "{}" "stale Stop replies {}"
assert_eq "$(cat "$WS_HOME/sessions/$SOURCE_A/inbox")" "$G3B_INBOX" "stale Stop leaves the pane's inbox untouched"
assert_eq "$(inflight_count "$SOURCE_A")" "0" "stale Stop drains nothing in flight"
assert_eq "$(g3b_resume "$SOURCE_A")" "$G3B_RESUME" "stale Stop keeps the pane's resume id"
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'[ws terminal]\nfor-new-launch' "current launch still receives its inbox"

echo "ws stop: an old in-flight file whose pid was reused is reclaimed"
sleep 30 &
g3b_pid=$!
print -r -- '{"message":"pid-reused"}' > "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$g3b_pid"
touch -t 202001010000 "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$g3b_pid"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "pid-reused" "stale in-flight file is redelivered despite a live pid"
assert_eq "$(inflight_count "$SOURCE_A")" "0" "stale in-flight file reclaimed"
echo "ws done: reclaims stale in-flight files before refusing"
print -r -- '{"message":"pid-reused-done"}' > "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$g3b_pid"
touch -t 202001010000 "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$g3b_pid"
WS_SESSION="$SOURCE_A" "$WS" done 2>/dev/null
assert_exit "$?" "1" "done still refuses while messages are pending"
assert_eq "$(inflight_count "$SOURCE_A")" "0" "done requeued the stale in-flight file"
assert_eq "$(jq -r '.message' "$WS_HOME/sessions/$SOURCE_A/inbox")" "pid-reused-done" "requeued message waits in the inbox"
kill "$g3b_pid"; wait "$g3b_pid" 2>/dev/null
hook api/triggers "$SOURCE_A_WIN" working </dev/null
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "pid-reused-done" "next Stop delivers the requeued message"

echo "ws hook: Stop stays inside the hook timeout when resume-id locks are held"
G3B_META="$WS_HOME/sessions/$SOURCE_A/meta"
G3B_SESS="$(git -C "$(T display -p -t "$SOURCE_A_WIN" '#{@ws_path}')" rev-parse --absolute-git-dir)/ws-sessions"
: >> "$G3B_META.lock"; : >> "$G3B_SESS.lock"
zsh -c 'zmodload zsh/system; zsystem flock -f a "$1.lock" && zsystem flock -f b "$2.lock" && : > "$3" && sleep 8' g3b "$G3B_META" "$G3B_SESS" "$TMP/g3b-held" &
g3b_holder=$!
wait_until test -e "$TMP/g3b-held"
G3B_RESUME="$(g3b_resume "$SOURCE_A")"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
zmodload zsh/datetime
g3b_t0=$EPOCHREALTIME
reply="$(print -r -- '{"session_id":"sid-under-contention"}' | hook api/triggers "$SOURCE_A_WIN" stop 2>/dev/null)"
g3b_elapsed=$(( EPOCHREALTIME - g3b_t0 ))
assert_eq "$reply" "{}" "contended Stop still replies"
assert_eq "$(( g3b_elapsed < 2 ))" "1" "contended Stop finishes well under 5s (took ${g3b_elapsed}s)"
assert_eq "$(g3b_resume "$SOURCE_A")" "$G3B_RESUME" "contended Stop skips the resume-id update"
kill "$g3b_holder"; wait "$g3b_holder" 2>/dev/null
rm -f "$TMP/g3b-held"
# --- end G3b ---

echo "ws on: rejects direct and transitive cycles; removal cancels a pending action"
CYCLE_TID="$("$WS" on "$SOURCE_A" exit --send-to "$TARGET_ID" cycle-forward)"
assert_exit "$?" "0" "forward dependency registers"
out="$("$WS" on "$TARGET_ID" exit --send-to "$SOURCE_A" cycle-backward 2>&1)"
assert_exit "$?" "1" "reverse edge forming a cycle is rejected"
assert_contains "$out" "cycle" "cycle error explains rejection"
"$WS" on "$TARGET_ID" done --send-to "$TARGET_ID" self-cycle 2>/dev/null
assert_exit "$?" "1" "direct self-dependency is rejected"
"$WS" on "$TARGET_ID,$SOURCE_B" done --send-to "$TARGET_ID" fan-in-cycle 2>/dev/null
assert_exit "$?" "1" "fan-in cannot include its own target"
CHAIN_TID="$("$WS" on "$TARGET_ID" exit --send-to "$SOURCE_B" cycle-chain)"
out="$("$WS" on "$SOURCE_B" exit --send-to "$SOURCE_A" cycle-end 2>&1)"
assert_exit "$?" "1" "three-session cycle is rejected"
assert_contains "$out" "cycle" "transitive cycle is explained"
"$WS" on --rm "$CHAIN_TID"
assert_exit "$?" "0" "removes pending chain trigger"
"$WS" on --rm "$CYCLE_TID"
assert_exit "$?" "0" "removes pending trigger"
assert_eq "$("$WS" triggers)" "" "removed triggers are absent"
"$WS" on --rm "$CYCLE_TID" 2>/dev/null
assert_exit "$?" "1" "removing an absent trigger is rejected"
hook api/triggers "$TARGET_WIN" working </dev/null
T send-keys -t "$SOURCE_A_WIN" Enter
wait_until lifecycle_exit_count "$SOURCE_A" 1
assert_eq "$(hook api/triggers "$TARGET_WIN" stop </dev/null)" "{}" "cancelled trigger never delivers"

echo "ws on: spawn expands a literal prompt and starts a tracked child in the same worktree"
printf 'spawn result {id} {output}' | WS_SESSION="$SOURCE_B" "$WS" done -
SPAWN_PROMPT=$'Follow "quoted" $literal\nsource {id}: {output}; $(touch never-execute-ws-prompt)'
for spawn_agent in claude codex grok; do
  before_ids="$("$WS" sessions | cut -f1 | paste -sd, -)"
  SPAWN_TID="$(WS_SESSION="$TARGET_ID" "$WS" on "$SOURCE_B" done --spawn "$SPAWN_PROMPT" --agent "$spawn_agent")"
  assert_exit "$?" "0" "$spawn_agent spawn trigger registers and fires"
  CHILD_ID="$("$WS" sessions | awk -F '\t' -v a="$spawn_agent" -v ids="$before_ids" '$3 == "api/triggers" && $4 == a { if (index("," ids ",", "," $1 ",") == 0) print $1 }')"
  assert_eq "${CHILD_ID:+found}" "found" "$spawn_agent spawn creates one new session"
  if [[ -n "$CHILD_ID" ]]; then
    CHILD_WIN="$(jq -r '.window' "$WS_HOME/sessions/$CHILD_ID/meta")"
    assert_contains "$(wait_pane "$CHILD_WIN" "FAKE-$spawn_agent")" "WHOAMI=$CHILD_ID" "$spawn_agent child has WS_SESSION and can query it"
    wait_until test -s "$WS_HOME/argv-$CHILD_ID.json"
    assert_agent_preamble "$spawn_agent" "$CHILD_ID" spawned
    assert_eq "$(jq -r '.[-1]' "$WS_HOME/argv-$CHILD_ID.json")" "Follow \"quoted\" \$literal"$'\n'"source $SOURCE_B: spawn result {id} {output}; \$(touch never-execute-ws-prompt)" "$spawn_agent initial prompt is one literal final argument"
    assert_eq "$(jq -r '.spawned' "$WS_HOME/sessions/$CHILD_ID/meta")" "true" "$spawn_agent child is marked spawned"
    assert_eq "$(jq -r '.path' "$WS_HOME/sessions/$CHILD_ID/meta")" "$TRIGGER_WT" "$spawn_agent child uses registering session worktree"
    T send-keys -t "$CHILD_WIN" done Enter
    wait_until spawn_done_closed "$CHILD_ID"
    assert_exit "$?" "0" "$spawn_agent spawned child closes on done"
    assert_eq "$(cat "$WS_HOME/sessions/$CHILD_ID/output.md")" "agent result" "$spawn_agent child publishes output before closure"
  fi
done
assert_eq "$([[ -e "$TRIGGER_WT/never-execute-ws-prompt" ]] && print yes)" "" "spawn prompt never executes shell substitution"

echo "ws on: concurrent last-source exits spawn once and default agent is claude"
DEFAULT_TID="$(WS_SESSION="$TARGET_ID" "$WS" on "$SOURCE_B" done --spawn default-child)"
assert_exit "$?" "0" "spawn accepts omitted agent"
DEFAULT_ID="$("$WS" sessions | awk -F '\t' '$3 == "api/triggers" && $5 != "stopped" && $1 != "'"$TARGET_ID"'" && $1 != "'"$SOURCE_B"'" {print $1}')"
assert_eq "$("$WS" sessions "$DEFAULT_ID" | cut -f4)" "claude" "spawn defaults to claude"
DEFAULT_WIN="$(jq -r '.window' "$WS_HOME/sessions/$DEFAULT_ID/meta")"
assert_contains "$(wait_pane "$DEFAULT_WIN" FAKE-claude)" "default-child" "default child receives initial prompt"
exit_before="$("$WS" sessions | wc -l | tr -d ' ')"
EXIT_TID="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B,$TARGET_ID" exit --spawn 'after exit {id}' --agent codex)"
assert_exit "$?" "0" "exit fan-in registers while both sources are live"
assert_eq "$("$WS" sessions | wc -l | tr -d ' ')" "$exit_before" "exit trigger does not confuse done with exit"
assert_contains "$("$WS" triggers)" "$EXIT_TID"$'\t'"$SOURCE_B,$TARGET_ID"$'\texit\tspawn' "pending spawn trigger is listed"
T send-keys -t "$SOURCE_B_WIN" Enter &
exit_a_pid=$!
T send-keys -t "$TARGET_WIN" Enter &
exit_b_pid=$!
wait "$exit_a_pid"; wait "$exit_b_pid"
wait_until lifecycle_exit_count "$SOURCE_B" 1
wait_until lifecycle_exit_count "$TARGET_ID" 1
exit_child_ready() { [[ "$("$WS" sessions | awk -F '\t' '$3 == "api/triggers" && $4 == "codex" && $5 != "stopped" {print $1}')" == <-> ]]; }
wait_until exit_child_ready
assert_exit "$?" "0" "concurrent exits start one codex child"
EXIT_CHILD="$("$WS" sessions | awk -F '\t' '$3 == "api/triggers" && $4 == "codex" && $5 != "stopped" {print $1}')"
assert_eq "$("$WS" sessions | wc -l | tr -d ' ')" "$((exit_before + 1))" "two racing evaluations allocate exactly one child"
assert_contains "$(wait_pane "$(jq -r '.window' "$WS_HOME/sessions/$EXIT_CHILD/meta")" FAKE-codex)" "after exit $SOURCE_B,$TARGET_ID" "exit child receives expanded source ids"
assert_eq "$("$WS" triggers)" "" "exit action is no longer pending"

echo "ws on: sends a literal dash instead of interpreting the trigger message as stdin"
hook api/triggers "$DEFAULT_WIN" working </dev/null
DASH_TID="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B" done --send-to me - </dev/null)"
assert_exit "$?" "0" "literal dash trigger registers"
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$DEFAULT_ID]"$'\n-' "trigger dash is delivered literally"

# --- G2: empty templates reject registration; failed actions keep their stderr ---
echo "ws on: output-only send and spawn templates reject registration"
empty_before="$(windows_of "$TRIGGER_WT")"
"$WS" add api/triggers --detach
EMPTY_WIN="$(windows_of "$TRIGGER_WT" | grep -vxF -f <(print -r -- "$empty_before"))"
EMPTY_SRC="$(T display -p -t "$EMPTY_WIN" '#{@ws_session}')"
WS_SESSION="$EMPTY_SRC" "$WS" done </dev/null
assert_exit "$?" "0" "empty-output source declares done"
empty_sessions="$("$WS" sessions | wc -l | tr -d ' ')"
for empty_action in --send-to --spawn; do
  empty_counter="$(cat "$WS_HOME/triggers/counter")"
  if [[ "$empty_action" == --send-to ]]; then
    out="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$EMPTY_SRC" done --send-to me '{output}' 2>&1)"
  else
    out="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$EMPTY_SRC" done --spawn ' {output}' 2>&1)"
  fi
  assert_exit "$?" "1" "$empty_action empty expansion reports failure"
  assert_contains "$out" "empty" "$empty_action empty expansion explains itself"
  assert_eq "$(cat "$WS_HOME/triggers/counter")" "$empty_counter" "$empty_action empty template allocates no trigger"
done
assert_eq "$("$WS" sessions | wc -l | tr -d ' ')" "$empty_sessions" "empty spawn prompt never starts a blank child"
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
assert_eq "$reply" "{}" "empty send expansion queues nothing"

echo "ws on: a failed trigger action records its error"
BROKEN_ID=999999
mkdir -p "$WS_HOME/sessions/$BROKEN_ID"
jq -n --arg id "$BROKEN_ID" '{id:$id,name:"",workspace:"api/gone",agent:"claude",status:"active",path:"/nonexistent/ws-gone",slot:1,window:""}' > "$WS_HOME/sessions/$BROKEN_ID/meta"
out="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$EMPTY_SRC" done --send-to "$BROKEN_ID" unreachable 2>&1)"
assert_exit "$?" "1" "unreachable trigger target fails"
BROKEN_TID="${${(f)out}[1]}"
assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/$BROKEN_TID.json")" "failed" "failed action is marked failed"
assert_contains "$(jq -r '.error // ""' "$WS_HOME/triggers/archive/$BROKEN_TID.json")" "no longer exists" "failed action record keeps stderr"
rm -rf "$WS_HOME/sessions/$BROKEN_ID"
T send-keys -t "$EMPTY_WIN" Enter
# --- end G2 trigger failures ---

# --- G4: queued-not-delivered sends are not failures ---
echo "ws send/on: a durably queued but undelivered send exits 75 and triggers record queued"
QUEUED_LOCK="${TMPDIR:-/tmp}/ws-hook-$WS_TMUX_SOCKET-$DEFAULT_WIN.lock"
rm -f "$TMP/queued-held" "$TMP/queued-release"
(zmodload zsh/system; : >> "$QUEUED_LOCK"; zsystem flock -f queued_fd "$QUEUED_LOCK"; : > "$TMP/queued-held"
  while [[ ! -e "$TMP/queued-release" ]]; do sleep 0.05; done) &
queued_holder=$!
wait_until test -e "$TMP/queued-held"
out="$(WS_SESSION=1 "$WS" send "$DEFAULT_ID" "queued direct" 2>&1)"
assert_exit "$?" "75" "busy target: direct send exits 75 (queued, not delivered)"
assert_contains "$out" "message queued" "queued send keeps its warning"
out="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$EMPTY_SRC" done --send-to me 'queued trigger' 2>&1)"
assert_exit "$?" "0" "ws on: queued inline action is not a failure"
QUEUED_TID="${${(f)out}[1]}"
assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/$QUEUED_TID.json" 2>/dev/null)" "queued" "queued action archived as queued"
assert_contains "$(jq -r '.warning // ""' "$WS_HOME/triggers/archive/$QUEUED_TID.json" 2>/dev/null)" "message queued" "queued record keeps the warning"
: > "$TMP/queued-release"
wait "$queued_holder"
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null | jq -r '.reason // empty')"
assert_eq "$(print -r -- "$reply" | grep -c '^queued direct$')" "1" "queued direct send delivered exactly once"
assert_eq "$(print -r -- "$reply" | grep -c '^queued trigger$')" "1" "queued trigger send delivered exactly once"
# --- end G4 queued sends ---

echo "ws on: validates source lists, events and action options without creating triggers"
for bad_sources in '' ',' "$SOURCE_B," ",$SOURCE_B" "$SOURCE_B,,${TARGET_ID}" no-such-session; do
  "$WS" on "$bad_sources" done --send-to "$DEFAULT_ID" invalid 2>/dev/null
  assert_exit "$?" "1" "invalid source list rejected: $bad_sources"
done
"$WS" on "$SOURCE_B" idle --send-to "$DEFAULT_ID" invalid 2>/dev/null
assert_exit "$?" "1" "unsupported lifecycle event rejected"
"$WS" on "$SOURCE_B" done --send-to missing-target invalid 2>/dev/null
assert_exit "$?" "1" "unknown target rejected"
"$WS" on "$SOURCE_B" done --send-to "$DEFAULT_ID" '' 2>/dev/null
assert_exit "$?" "1" "empty message rejected"
WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B" done --spawn '' 2>/dev/null
assert_exit "$?" "1" "empty spawn prompt rejected"
WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B" done --spawn invalid --agent invalid 2>/dev/null
assert_exit "$?" "1" "unknown spawn agent rejected"
WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B" done --spawn invalid --send-to "$DEFAULT_ID" 2>/dev/null
assert_exit "$?" "1" "mixed send/spawn options rejected"
(cd "$REPO" && env -u WS_SESSION -u TMUX -u WS_WORKSPACE "$WS" on "$SOURCE_B" done --spawn invalid 2>/dev/null)
assert_exit "$?" "1" "terminal spawn outside ws worktree rejected"
"$WS" triggers unexpected 2>/dev/null
assert_exit "$?" "1" "triggers rejects unexpected arguments"
"$WS" on --rm ../counter 2>/dev/null
assert_exit "$?" "1" "remove rejects unsafe trigger ids"
assert_eq "$("$WS" triggers)" "" "invalid registrations create no pending triggers"

# --- G3: spawn prompt via file, open failure cleanup, ready timeout, non-interactive ws ---
echo "ws on: spawn prompts beyond tmux's command limit reach the agent argv intact"
new_session_id() { comm -13 <(print -r -- "$1" | sort) <(print -rl -- "$WS_HOME"/sessions/<->(N:t) | sort); }
BIG_PAD=""
BIG_PROMPT="big-start ${(l:24000::x:)BIG_PAD} \$(touch never-execute-big) big-end"
before_ids="$(print -rl -- "$WS_HOME"/sessions/<->(N:t))"
WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B" done --spawn "$BIG_PROMPT" --agent claude >/dev/null
assert_exit "$?" "0" "large spawn trigger fires"
BIG_ID="$(new_session_id "$before_ids")"
# n5: session discovery must fail closed even when ls color is forced.
assert_eq "$([[ "$BIG_ID" == <-> ]] && print yes)" "yes" "large spawn discovers one numeric session id"
wait_until test -s "$WS_HOME/argv-$BIG_ID.json"
assert_eq "$(jq -r '.[-1]' "$WS_HOME/argv-$BIG_ID.json" 2>/dev/null)" "$BIG_PROMPT" "24KB spawn prompt is the literal final argument"
assert_eq "$(jq -r '.[-2]' "$WS_HOME/argv-$BIG_ID.json" 2>/dev/null)" "--" "large prompt still follows --"
assert_eq "$([[ -e "$WS_HOME/sessions/$BIG_ID/prompt" ]] && print yes)" "" "prompt file is consumed at launch"
assert_eq "$([[ -e "$TRIGGER_WT/never-execute-big" ]] && print yes)" "" "large prompt never executes substitution"
if [[ "$BIG_ID" == <-> ]]; then
  T send-keys -t "$(jq -r '.window' "$WS_HOME/sessions/$BIG_ID/meta")" done Enter
  wait_until spawn_done_closed "$BIG_ID"
fi

echo "ws on: a spawn whose tab fails to open leaves no slot row and a closed identity"
FAIL_RECORD="$TMP/fail-trigger.json"
jq -n --arg p "$TRIGGER_WT" --arg s "$DEFAULT_ID" '{id:"t-fail",path:$p,agent:"codex",sender:$s}' > "$FAIL_RECORD"
before_ids="$(print -rl -- "$WS_HOME"/sessions/<->(N:t))"
( source "$WS"; open_session() { return 1; }; ws_bus_spawn_trigger "$FAIL_RECORD" never-opened ) >/dev/null 2>&1
assert_exit "$?" "1" "failed spawn open reports failure"
FAIL_ID="$(new_session_id "$before_ids")"
assert_eq "$([[ "$FAIL_ID" == <-> ]] && print yes)" "yes" "failed spawn allocated one numeric identity"
assert_eq "$(awk -F '\t' -v id="$FAIL_ID" '$4 == id' "$(git -C "$TRIGGER_WT" rev-parse --absolute-git-dir)/ws-sessions")" "" "failed spawn frees its slot row"
assert_eq "$(jq -r '.status' "$WS_HOME/sessions/$FAIL_ID/meta" 2>/dev/null)" "closed" "failed spawn identity is closed"

echo "ws send: input readiness waits up to WS_READY_TIMEOUT seconds"
# ready_probe <polls-until-ready> — runs ws_bus_wait_ready against a stub pane.
ready_probe() {
  ( local after="$1"; source "$WS"
    WS_SESSIONS="$TMP/ready"; mkdir -p "$WS_SESSIONS/1"; print queued > "$WS_SESSIONS/1/inbox"; print 0 > "$TMP/ready-polls"
    ws_bus_pane() { print %0; }
    tmux_() { local n=$(( $(<"$TMP/ready-polls") + 1 )); print $n > "$TMP/ready-polls"; (( n > after )) && print 1 || print 0; }
    ws_bus_wait_ready 1 @0 )
}
ready_probe 55
assert_exit "$?" "0" "TUI ready after >5s of polling succeeds within the default budget"
WS_READY_TIMEOUT=0.3 ready_probe 1000
assert_exit "$?" "1" "WS_READY_TIMEOUT bounds the wait"
assert_eq "$(( $(<"$TMP/ready-polls") < 10 ))" "1" "short WS_READY_TIMEOUT gives up early"

echo "ws add: agent shells resolve a non-interactive ws command"
cat > "$TMP/env-shell" <<'EOF'
#!/bin/zsh -f
print -r -- "$(command -v ws)|${WS_BIN:-}" >> "$WS_HOME/agent-env"
exec zsh -fc "$@"
EOF
chmod +x "$TMP/env-shell"
WS_AGENT_SHELL="$TMP/env-shell" "$WS" add api/triggers --agent claude --detach >/dev/null
# env-shell logs twice: the tmux tab shell (before run's PATH entry), then the agent shell.
agent_env_lines() { (( $(wc -l < "$WS_HOME/agent-env" 2>/dev/null || print 0) >= 2 )); }
WS_TEST_WAIT=5 wait_until agent_env_lines
ENV_LINE="$(sed -n 2p "$WS_HOME/agent-env" 2>/dev/null)"
assert_eq "$([[ "${ENV_LINE%%|*}" -ef "$WS" ]] && print yes)" "yes" "ws on agent PATH resolves to ws.sh"
assert_eq "${ENV_LINE#*|}" "$WS" "agent env exports WS_BIN"

# --- M2 stale exit: resumed sources wait for a new exit ---
echo "ws on: old exits do not satisfy a new watcher on a resumed session"
"$WS" open "$SOURCE_A" --detach >/dev/null
M2_SOURCE_WIN="$(jq -r '.window' "$WS_HOME/sessions/$SOURCE_A/meta")"
assert_contains "$(wait_pane "$M2_SOURCE_WIN" FAKE-claude)" "WHOAMI=$SOURCE_A" "stale-exit source resumes with the same id"
hook api/triggers "$DEFAULT_WIN" working </dev/null
M2_TID="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_A" exit --send-to me 'new exit {id}')"
assert_exit "$?" "0" "resumed source exit watcher registers"
assert_contains "$("$WS" triggers)" "$M2_TID"$'\t'"$SOURCE_A"$'\texit\tsend' "old exit leaves new watcher pending"
assert_eq "$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)" "{}" "registration delivers nothing for an old exit"
hook api/triggers "$DEFAULT_WIN" working </dev/null
T send-keys -t "$M2_SOURCE_WIN" Enter
wait_until lifecycle_exit_count "$SOURCE_A" 2
m2_trigger_finished() {
  [[ -z "$("$WS" triggers | awk -F '\t' -v id="$M2_TID" '$1 == id')" &&
     "$("$WS" sessions "$SOURCE_A" | cut -f5)" == stopped ]]
}
wait_until m2_trigger_finished
assert_exit "$?" "0" "next exit fires the pending watcher"
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$DEFAULT_ID]"$'\n'"new exit $SOURCE_A" "next exit delivers the continuation"
assert_eq "$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)" "{}" "exit watcher delivers once"
# --- end M2 stale exit ---

# --- trigger edges ---
echo "ws on: trigger edges warn and skip a corrupt record during cycle validation"
for corrupt_record in '{' '{"status":"pending","action":"send","sources":7}'; do
  print -r -- "$corrupt_record" > "$WS_HOME/triggers/999999.json"
  out="$("$WS" on "$SOURCE_A" exit --send-to "$DEFAULT_ID" 'after exit' 2>&1)"
  assert_exit "$?" "0" "corrupt cycle-graph record does not reject registration"
  assert_contains "$out" "skipping unreadable record" "corrupt cycle-graph record is warned about"
  rm -f "$WS_HOME/triggers/999999.json"
  "$WS" on --rm "$(cat "$WS_HOME/triggers/counter")"
done
# Registration must report this to the waiter even before a future event has output.
echo "ws on: trigger edges reject templates that can expand to empty"
for edge_template in '{output}' $' \t{output} {output}\n'; do
  edge_counter="$(cat "$WS_HOME/triggers/counter")"
  out="$("$WS" on "$SOURCE_A" exit --send-to "$DEFAULT_ID" "$edge_template" 2>&1)"
  assert_exit "$?" "1" "output-only future trigger rejects registration"
  assert_contains "$out" "empty" "waiter sees the empty-template error at registration"
  assert_eq "$(cat "$WS_HOME/triggers/counter")" "$edge_counter" "empty template leaves no pending trigger"
  [[ "$(cat "$WS_HOME/triggers/counter")" == "$edge_counter" ]] || "$WS" on --rm "$(cat "$WS_HOME/triggers/counter")"
done
echo "ws on: trigger edges archive terminal actions outside the active registry"
assert_eq "$([[ -f "$WS_HOME/triggers/$TID.json" ]] && print active)" "" "fired trigger is pruned from the active registry"
assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/$TID.json" 2>/dev/null)" "fired" "archive preserves the fired trigger for diagnostics"
assert_eq "$([[ -f "$WS_HOME/triggers/$BROKEN_TID.json" ]] && print active)" "" "failed trigger is pruned from the active registry"
assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/$BROKEN_TID.json" 2>/dev/null)" "failed" "archive preserves the failed action"
assert_contains "$(jq -r '.error // ""' "$WS_HOME/triggers/archive/$BROKEN_TID.json" 2>/dev/null)" "no longer exists" "archive keeps the failure reason"
# --- trigger edge batching ---
echo "ws on: trigger edges batch unrelated records and retire legacy terminal records"
edge_jq="$(command -v jq)"
mkdir -p "$TMP/edge-bin"
cat > "$TMP/edge-bin/jq" <<'EDGE_JQ'
#!/bin/sh
printf '.\n' >> "$WS_EDGE_JQ_CALLS"
exec "$WS_EDGE_JQ" "$@"
EDGE_JQ
chmod +x "$TMP/edge-bin/jq"
WS_SESSION="$DEFAULT_ID" "$WS" name trigger-edge-waiter
for edge_fixture in {700001..700030}; do
  mkdir -p "$WS_HOME/sessions/$edge_fixture"
  jq -n --arg id "$edge_fixture" '{id:$id,name:"",status:"active"}' > "$WS_HOME/sessions/$edge_fixture/meta"
  jq -n --arg id "$edge_fixture" --arg source "$SOURCE_B" --arg target "$DEFAULT_ID" \
    '{id:$id,sources:[$source],event:"exit",action:"send",target:$target,template:"after exit",status:"pending"}' \
    > "$WS_HOME/triggers/$edge_fixture.json"
done
: > "$TMP/edge-jq-calls"
out="$(PATH="$TMP/edge-bin:$PATH" WS_EDGE_JQ="$edge_jq" WS_EDGE_JQ_CALLS="$TMP/edge-jq-calls" "$WS" show trigger-edge-waiter)"
assert_contains "$out" "id: $DEFAULT_ID" "batched name lookup resolves the waiter"
edge_calls="$(wc -l < "$TMP/edge-jq-calls")"
assert_eq "$(( edge_calls <= 2 ))" "1" "name lookup uses bounded jq processes across many sessions"
# A record retained by older versions must be archived without executing it again.
cp "$WS_HOME/triggers/archive/$TID.json" "$WS_HOME/triggers/$TID.json"
: > "$TMP/edge-jq-calls"
PATH="$TMP/edge-bin:$PATH" WS_EDGE_JQ="$edge_jq" WS_EDGE_JQ_CALLS="$TMP/edge-jq-calls" WS_SESSION="$SOURCE_A" "$WS" done
assert_exit "$?" "0" "done succeeds with many unrelated pending triggers"
edge_retired() { [[ ! -f "$WS_HOME/triggers/$TID.json" ]]; }
wait_until edge_retired # evaluation runs in a detached worker
edge_calls="$(wc -l < "$TMP/edge-jq-calls")"
assert_eq "$(( edge_calls < 20 ))" "1" "event evaluation uses bounded jq processes across unrelated triggers"
assert_eq "$([[ -f "$WS_HOME/triggers/$TID.json" ]] && print active)" "" "legacy fired record is retired on the next event"
assert_eq "$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)" "{}" "unrelated triggers do not notify the waiter"
for edge_fixture in {700001..700030}; do
  rm -rf "$WS_HOME/sessions/$edge_fixture"
  rm -f "$WS_HOME/triggers/$edge_fixture.json"
done
# --- end trigger edge batching ---
# --- trigger edge record tolerance ---
echo "ws on: trigger edges tolerate corrupt records and never block on live actions"
for corrupt_record in '{' '{"status":"pending","event":"done","action":"send","sources":7}'; do
  print -r -- "$corrupt_record" > "$WS_HOME/triggers/999998.json"
  jq -n --arg source "$SOURCE_A" --arg target "$DEFAULT_ID" \
    '{id:"999997",sources:[$source],event:"done",action:"send",target:$target,template:"healthy edge",sender:"",status:"pending"}' \
    > "$WS_HOME/triggers/999997.json"
  jq -n --arg source "$SOURCE_B" --arg target "$DEFAULT_ID" \
    '{id:"999996",sources:[$source],event:"exit",action:"send",target:$target,template:"later edge",sender:"",status:"pending"}' \
    > "$WS_HOME/triggers/999996.json"
  out="$("$WS" triggers 2>&1)"
  assert_exit "$?" "0" "triggers listing survives a corrupt record"
  assert_contains "$out" "later edge" "triggers listing keeps healthy records"
  hook api/triggers "$DEFAULT_WIN" working </dev/null # busy target: delivery queues for Stop
  out="$(WS_SESSION="$SOURCE_A" "$WS" done 2>&1)"
  assert_exit "$?" "0" "done succeeds despite a corrupt sibling trigger"
  assert_contains "$out" "999998.json" "corrupt trigger record is named in a warning"
  wait_until test -f "$WS_HOME/triggers/archive/999997.json"
  assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/999997.json" 2>/dev/null)" "fired" "healthy trigger fires beside a corrupt one"
  reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
  assert_contains "$(print -r -- "$reply" | jq -r '.reason // empty')" "healthy edge" "healthy trigger delivers beside a corrupt one"
  rm -f "$WS_HOME/triggers/999998.json" "$WS_HOME/triggers/999996.json" "$WS_HOME/triggers/999997.json"
done
mkdir -p "$WS_HOME/sessions/799999"
print -r -- '{' > "$WS_HOME/sessions/799999/meta"
out="$("$WS" show trigger-edge-waiter 2>&1)"
assert_contains "$out" "id: $DEFAULT_ID" "name lookup survives one corrupt session meta"
rm -rf "$WS_HOME/sessions/799999"
# Legacy record whose expansion is empty: fail and archive instead of re-erroring forever.
jq -n --arg source "$SOURCE_A" --arg target "$DEFAULT_ID" \
  '{id:"999995",sources:[$source],event:"done",action:"send",target:$target,template:"{output}",sender:"",status:"pending"}' \
  > "$WS_HOME/triggers/999995.json"
WS_SESSION="$SOURCE_A" "$WS" done 2>/dev/null
assert_exit "$?" "0" "done succeeds when a legacy trigger expands to empty"
wait_until test -f "$WS_HOME/triggers/archive/999995.json"
assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/999995.json" 2>/dev/null)" "failed" "empty expansion archives the trigger as failed"
assert_contains "$(jq -r '.error // ""' "$WS_HOME/triggers/archive/999995.json" 2>/dev/null)" "empty" "empty expansion records its error"
# A fired record whose action is still running holds its lock: unrelated events skip it.
jq -n '{id:"999994",sources:["0"],event:"done",action:"send",target:"0",template:"x",sender:"",status:"fired"}' \
  > "$WS_HOME/triggers/999994.json"
: > "$WS_HOME/triggers/999994.lock"
rm -f "$TMP/edge-held"
zsh -c 'zmodload zsh/system; zsystem flock "$1" && : > "$2" && sleep 8' _ "$WS_HOME/triggers/999994.lock" "$TMP/edge-held" &
edge_holder=$!
for _ in {1..100}; do [[ -f "$TMP/edge-held" ]] && break; sleep 0.05; done
zmodload zsh/datetime
edge_start=$EPOCHREALTIME
out="$(WS_SESSION="$SOURCE_A" "$WS" done 2>&1)"
assert_exit "$?" "0" "done succeeds while an unrelated action is in flight"
assert_eq "$(( EPOCHREALTIME - edge_start < 2.5 ))" "1" "in-flight action lock does not delay unrelated events"
assert_eq "$([[ "$out" == *"timed out"* ]] && print timeout)" "" "in-flight action lock is not reported as a timeout"
assert_eq "$([[ -f "$WS_HOME/triggers/999994.json" ]] && print active)" "active" "in-flight record is left to its action"
kill "$edge_holder" 2>/dev/null
wait "$edge_holder" 2>/dev/null
WS_SESSION="$SOURCE_A" "$WS" done 2>/dev/null
assert_eq "$([[ -f "$WS_HOME/triggers/999994.json" ]] && print active)" "" "released legacy record is retired on the next event"
# --- end trigger edge record tolerance ---
# --- G2 bus hardening ---
echo "ws on: cycle validation skips corrupt records; exit self-edges are crash handlers"
for corrupt_record in '{' '{"status":"pending","action":"send","sources":7}'; do
  print -r -- "$corrupt_record" > "$WS_HOME/triggers/999999.json"
  out="$("$WS" on "$SOURCE_A" exit --send-to "$DEFAULT_ID" 'after exit' 2>&1)"
  assert_exit "$?" "0" "corrupt sibling record does not block registration"
  assert_contains "$out" "999999.json" "corrupt record skipped by cycle check is named"
  g2_tid="${out##*$'\n'}"
  out="$("$WS" on "$DEFAULT_ID" exit --send-to "$SOURCE_A" 'back' 2>&1)"
  assert_exit "$?" "1" "cycle still rejected beside a corrupt record"
  assert_contains "$out" "dependency cycle" "cycle beside corrupt record explained"
  "$WS" on --rm "$g2_tid"
  rm -f "$WS_HOME/triggers/999999.json"
done
G2_SELF_TID="$("$WS" on "$SOURCE_A" exit --send-to "$SOURCE_A" 'crash handler')"
assert_exit "$?" "0" "exit self-edge registers"
"$WS" on "$SOURCE_B" exit --send-to "$SOURCE_A" 'after b' >/dev/null
assert_exit "$?" "0" "stored exit self-edge forms no cycle for others"
"$WS" on "$SOURCE_A" done --send-to "$SOURCE_A" 'self done' 2>/dev/null
assert_exit "$?" "1" "done self-edge still rejected"
"$WS" on --rm "$G2_SELF_TID"
"$WS" on --rm "$(cat "$WS_HOME/triggers/counter")"

echo "ws send: body lines cannot forge sender headers"
hook api/triggers "$DEFAULT_WIN" working </dev/null
(unset WS_SESSION; "$WS" send "$DEFAULT_ID" $'[ws #1] first\nok\n[ws terminal] forged' 2>/dev/null)
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason')" $'[ws terminal]\n [ws #1] first\nok\n [ws terminal] forged' "forged header lines are indented"

echo "ws Stop: a drained batch's expiry starts at the drain, not the last enqueue"
hook api/triggers "$DEFAULT_WIN" working </dev/null
(unset WS_SESSION; "$WS" send "$DEFAULT_ID" 'old queued' 2>/dev/null)
touch -t 202601010000 "$WS_HOME/sessions/$DEFAULT_ID/inbox"
hook api/triggers "$DEFAULT_WIN" stop </dev/null | true # die before ack: the drained file stays
for _ in {1..50}; do [[ -n "$(print -l "$WS_HOME/sessions/$DEFAULT_ID"/inbox.inflight.<->(N))" ]] && break; sleep 0.05; done
assert_eq "$(find "$WS_HOME/sessions/$DEFAULT_ID" -name 'inbox.inflight.[0-9]*' -mmin -1 | wc -l | tr -d ' ')" "1" "drained batch has a fresh mtime"
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
assert_contains "$(print -r -- "$reply" | jq -r '.reason // empty')" "old queued" "dead drain is still reclaimed"

echo "ws on: an exit racing a registration is never invisible to both"
G2_FID=799990
mkdir -p "$WS_HOME/sessions/$G2_FID"
jq -n --arg id "$G2_FID" --arg path "$(jq -r '.path' "$WS_HOME/sessions/$DEFAULT_ID/meta")" \
  '{id:$id,name:"",status:"active",path:$path,agent:"claude",window:""}' > "$WS_HOME/sessions/$G2_FID/meta"
: > "$WS_HOME/sessions/$G2_FID/events"
hook api/triggers "$DEFAULT_WIN" working </dev/null
g2_lost=0
for g2_i in {1..6}; do
  "$WS" _agent "$G2_FID" 'exit 3' </dev/null >/dev/null 2>&1 &
  g2_pid=$!
  g2_tid="$("$WS" on "$G2_FID" exit --send-to "$DEFAULT_ID" "race $g2_i" 2>/dev/null)"
  wait "$g2_pid"
  if [[ -f "$WS_HOME/triggers/$g2_tid.json" ]]; then
    g2_events="$(jq -s 'length' "$WS_HOME/sessions/$G2_FID/events")"
    (( $(jq -r --arg id "$G2_FID" '.event_offsets[$id]' "$WS_HOME/triggers/$g2_tid.json") >= g2_events )) || (( g2_lost += 1 ))
    "$WS" on --rm "$g2_tid"
  fi
done
assert_eq "$g2_lost" "0" "pending racing trigger always has an offset past the exit"
hook api/triggers "$DEFAULT_WIN" stop </dev/null >/dev/null
rm -rf "$WS_HOME/sessions/$G2_FID"

echo "ws send: an agent exiting mid-delivery never types into its tab's shell"
G2_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$(jq -r '.path' "$WS_HOME/sessions/$DEFAULT_ID/meta")" \
  "zsh -fc '\"$WS\" claude; while read -r line; do eval \"\$line\"; done'")"
assert_contains "$(wait_pane "$G2_WIN" FAKE-claude)" "FAKE-claude" "mid-delivery fixture ready"
G2_ID="$(T display -p -t "$G2_WIN" '#{@ws_session}')"
T set -w -t "$G2_WIN" @ws_state idle \; set -w -t "$G2_WIN" @ws_main 1
mkdir -p "$TMP/g2-bin"
cat > "$TMP/g2-bin/tmux" <<'G2_TMUX'
#!/bin/zsh
# Agent exits after validation, right before the paste: wait for its wrapper to untag.
if [[ " $* " == *" load-buffer "* ]]; then
  "$G2_TMUX" -L "$WS_TMUX_SOCKET" send-keys -t "$G2_WIN" Enter
  for _ in {1..100}; do
    [[ -z "$("$G2_TMUX" -L "$WS_TMUX_SOCKET" display -p -t "$G2_WIN" '#{@ws_session}')" ]] && break
    sleep 0.05
  done
fi
exec "$G2_TMUX" "$@"
G2_TMUX
chmod +x "$TMP/g2-bin/tmux"
(unset WS_SESSION; G2_TMUX="$(command -v tmux)" G2_WIN="$G2_WIN" PATH="$TMP/g2-bin:$PATH" "$WS" send "$G2_ID" "touch $TMP/g2-pwned" 2>/dev/null)
assert_exit "$?" "75" "lost ownership reports the message queued"
sleep 1
assert_eq "$([[ -e "$TMP/g2-pwned" ]] && print typed)" "" "message is not executed by the tab's shell"
assert_contains "$(cat "$WS_HOME/sessions/$G2_ID/inbox" 2>/dev/null)" "g2-pwned" "message stays queued"
T kill-window -t "$G2_WIN"

echo "ws send: a paste admitted just before the agent exits is discarded, not run by the shell"
G2_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$(jq -r '.path' "$WS_HOME/sessions/$DEFAULT_ID/meta")" \
  "zsh -fc '\"$WS\" claude; while read -r line; do eval \"\$line\"; done'")"
assert_contains "$(wait_pane "$G2_WIN" FAKE-claude)" "FAKE-claude" "late-paste fixture ready"
G2_ID="$(T display -p -t "$G2_WIN" '#{@ws_session}')"
T set -w -t "$G2_WIN" @ws_state idle \; set -w -t "$G2_WIN" @ws_main 1
cat > "$TMP/g2-bin/tmux" <<'G2_TMUX'
#!/bin/zsh
# One tmux command: the agent's exit line and the owned paste land together, before any untag.
[[ " $* " == *" paste-buffer "* ]] && exec "$G2_TMUX" -L "$WS_TMUX_SOCKET" send-keys -t "$G2_WIN" Enter \; "${@[3,-1]}"
exec "$G2_TMUX" "$@"
G2_TMUX
(unset WS_SESSION; G2_TMUX="$(command -v tmux)" G2_WIN="$G2_WIN" PATH="$TMP/g2-bin:$PATH" \
  "$WS" send "$G2_ID" $'touch '"$TMP"$'/g2-late\ntrue' 2>/dev/null)
assert_exit "$?" "75" "late paste reports the message queued"
sleep 1
T send-keys -t "$G2_WIN" "touch $TMP/g2-probe" Enter
for _ in {1..40}; do [[ -e "$TMP/g2-probe" ]] && break; sleep 0.05; done
assert_eq "$([[ -e "$TMP/g2-late" ]] && print typed)" "" "late paste is not executed by the tab's shell"
assert_eq "$([[ -e "$TMP/g2-probe" ]] && print clean)" "clean" "tab shell input holds no leftover paste"
T kill-window -t "$G2_WIN"
# --- end G2 bus hardening ---
# --- end trigger edges ---

# --- review fixes: pane ownership, async done, lock bootstrap, delayed delivery, stop lock ---
echo "ws hook (F1): a hook from a split pane never acts for the tab's agent"
hook api/triggers "$DEFAULT_WIN" working </dev/null
(unset WS_SESSION; "$WS" send "$DEFAULT_ID" 'split-pane-msg' 2>/dev/null)
F1_PANE="$(T split-window -d -P -F '#{pane_id}' -t "$DEFAULT_WIN")"
reply="$(WS_SESSION="$DEFAULT_ID" WS_WORKSPACE=api/triggers TMUX_PANE="$F1_PANE" "$WS" hook stop </dev/null)"
assert_eq "$reply" "{}" "split-pane Stop gets an empty reply"
assert_contains "$(cat "$WS_HOME/sessions/$DEFAULT_ID/inbox" 2>/dev/null)" "split-pane-msg" "split-pane Stop leaves the owner's inbox"
T kill-pane -t "$F1_PANE" 2>/dev/null
reply="$(WS_SESSION="$DEFAULT_ID" WS_WORKSPACE=api/triggers TMUX_PANE="$(T show -wqv -t "$DEFAULT_WIN" @ws_pane)" "$WS" hook stop </dev/null)"
assert_contains "$reply" "split-pane-msg" "owning pane's Stop still drains the inbox"

echo "ws done (F2/F6): fresh installs are quiet and triggers evaluate off the agent's call"
F2_BEFORE="$(windows_of "$TRIGGER_WT")"
"$WS" add api/triggers --detach
F2_WIN="$(windows_of "$TRIGGER_WT" | grep -vxF -f <(print -r -- "$F2_BEFORE"))"
F2_SRC="$(T display -p -t "$F2_WIN" '#{@ws_session}')"
mv "$WS_HOME/triggers" "$TMP/triggers-aside"
out="$(WS_SESSION="$F2_SRC" "$WS" done </dev/null 2>&1)"
assert_exit "$?" "0" "done succeeds before any trigger directory exists"
assert_eq "$out" "" "fresh install prints no lock errors"
rm -rf "$WS_HOME/triggers"
mv "$TMP/triggers-aside" "$WS_HOME/triggers"
hook api/triggers "$DEFAULT_WIN" idle </dev/null
F2_TID="$("$WS" on "$SOURCE_A" done --send-to "$DEFAULT_ID" 'f2 msg')"
zmodload zsh/datetime
f2_start=$EPOCHREALTIME
WS_READY_TIMEOUT=4 WS_SESSION="$SOURCE_A" "$WS" done </dev/null >/dev/null 2>&1
assert_eq "$(( EPOCHREALTIME - f2_start < 2 ))" "1" "done returns before its trigger action finishes"
WS_TEST_WAIT=15 wait_until test -f "$WS_HOME/triggers/archive/$F2_TID.json"
assert_exit "$?" "0" "detached worker still fires the trigger"

echo "ws hook (F8): a Stop that waits on the hook lock still settles the tab and drains the inbox"
F8_LOCK="${TMPDIR:-/tmp}/ws-hook-$WS_TMUX_SOCKET-$DEFAULT_WIN.lock"
hook api/triggers "$DEFAULT_WIN" working </dev/null
(unset WS_SESSION; "$WS" send "$DEFAULT_ID" 'f8 msg' 2>/dev/null)
rm -f "$TMP/f8-held" "$TMP/f8-release"
(zmodload zsh/system; : >> "$F8_LOCK"; zsystem flock -f f8_fd "$F8_LOCK"; : > "$TMP/f8-held"
  while [[ ! -e "$TMP/f8-release" ]]; do sleep 0.05; done) &
f8_holder=$!
wait_until test -e "$TMP/f8-held"
(sleep 6; : > "$TMP/f8-release") &
hook api/triggers "$DEFAULT_WIN" stop </dev/null >/dev/null
f8_settled() { [[ "$(T show -wqv -t "$DEFAULT_WIN" @ws_state)" != working ]]; }
WS_TEST_WAIT=12 wait_until f8_settled
assert_exit "$?" "0" "lock-starved Stop does not leave the tab working"
wait "$f8_holder"
# --- end review fixes ---

"$WS" rm api/triggers --force --delete-branch >/dev/null

echo "ws rm: refuses dirty, removes clean worktree + all its windows"
"$WS" add api/feat-login --detach
print dirty >> "$WT/file.txt"
"$WS" rm api/feat-login 2>/dev/null
assert_exit "$?" "1" "dirty workspace refused"
assert_eq "$([[ -d "$WT" ]] && print yes)" "yes" "dirty worktree kept"
git -C "$WT" checkout -q -- file.txt
"$WS" rm api/feat-login --delete-branch
assert_exit "$?" "0" "clean rm succeeds"
assert_eq "$([[ -d "$WT" ]] && print yes)" "" "worktree dir removed"
assert_eq "$(windows_of "$WT")" "" "all session windows killed"
assert_eq "$(T has-session -t '=api/feat-login' 2>/dev/null && print alive)" "" "worktree tmux session killed"
assert_eq "$(git -C "$REPO" branch --list feat/login)" "" "branch deleted"
"$WS" rm api/orphan --force
assert_exit "$?" "0" "rm works for window-less workspace"

test_summary
