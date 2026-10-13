#!/bin/zsh
# ws.sh — terminal workspace manager (worktree + one tmux window per agent session). Runs
# against a private tmux server (-L) with fake agent bins, so it never touches the user's.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
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
trap '[[ -n "$socket_listener_pid" ]] && kill "$socket_listener_pid" 2>/dev/null; tmux -L "$WS_TMUX_SOCKET" kill-server 2>/dev/null; rm -rf "$TMP" "$NOTCH_HOME"' EXIT
WS="$ROOT/ws.sh"
export WS_TEST_CLI="$WS"
T() { tmux -L "$WS_TMUX_SOCKET" "$@"; }

# Fake agents print their argv, then stay alive until a line arrives (→ clean exit 0).
mkdir -p "$TMP/bin"
for agent in claude codex grok; do
  cat > "$TMP/bin/$agent" <<EOF
#!/bin/sh
own_id=\$("\$WS_TEST_CLI" whoami)
echo "FAKE-$agent \$* WS=\$WS_WORKSPACE ID=\$WS_SESSION WHOAMI=\$own_id"
read line
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

REPO="$(make_repo api)"
print 'SECRET=1' > "$REPO/.env"

echo "ws new: creates worktree, copies env, opens a tmux session w/ one agent tab"
"$WS" new --repo "$REPO" --branch feat/login --detach
assert_exit "$?" "0" "new succeeds"
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
  "$WS" new --repo "$REPO" --branch "global-$i" --detach > "$TMP/global-$i.log" 2>&1 &
  global_pids+=($!)
done
for pid in $global_pids; do
  wait "$pid"
  assert_exit "$?" "0" "concurrent workspace launches successfully"
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
"$WS" open api/global-1 --detach
out="$(wait_pane "$(windows_of "$WS_ROOT/api/global-1")" FAKE-claude)"
assert_contains "$out" "ID=$GLOBAL_SESSION_ID WHOAMI=$GLOBAL_SESSION_ID" "workspace resume preserves global id without an agent resume id"
assert_eq "${out/*--resume*/resumed}" "$out" "global id is never passed as an agent resume id"
for i in {1..8}; do "$WS" rm "api/global-$i" --force --delete-branch >/dev/null; done

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
assert_eq "$(windows_of "$WT")" "" "both windows gone (simulated reboot)"
assert_contains "$("$WS" list)" $'stopped\tapi/feat-login\t' "workspace shows stopped"
"$WS" open reviewer --detach
assert_exit "$?" "0" "named stopped session resumes independently"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" FAKE-claude)" "--resume sid-claude-1" "named resume uses its saved conversation id"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" FAKE-claude)" "ID=1 WHOAMI=1" "named resume preserves global identity"
assert_eq "$(slot_window "$WT" 2)" "" "resuming one id leaves other sessions stopped"
T kill-window -t "$(slot_window "$WT" 1)"
"$WS" open api/feat-login --detach
wait_until test -n "$(slot_window "$WT" 2)"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" sid-claude-1)" "--resume sid-claude-1" "claude session resumed by id"
assert_contains "$(wait_pane "$(slot_window "$WT" 2)" sid-codex-2)" "resume sid-codex-2" "codex session resumed by id"
assert_eq "$(session_of "$(slot_window "$WT" 2)")" "api/feat-login" "restored into one worktree session"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" FAKE-claude)" "ID=1 WHOAMI=1" "workspace resume keeps the same named global identity"

echo "ws grok: launched w/ --always-approve + global ws hook file; resumed by GROK_SESSION_ID"
"$WS" new --repo "$REPO" --branch grokky --agent grok --detach
GWT="$WS_ROOT/api/grokky"
GW="$(slot_window "$GWT" 1)"
assert_eq "$(T display -p -t "$GW" '#{window_name}')" "grok" "grok tab named after agent"
assert_contains "$(wait_pane "$GW" FAKE-grok)" "--always-approve" "grok skips approvals"
GH="$GROK_HOME/hooks/ws.json"
assert_eq "$(jq -r '.hooks.StopCancelled[0].hooks[0].command' "$GH" 2>/dev/null)" "$WS hook idle" "grok hook file: StopCancelled → idle"
assert_eq "$(jq -r '.hooks.Notification[0].matcher' "$GH" 2>/dev/null)" "permission_prompt" "grok hook file: permission prompt → waiting"
"$WS" add api/grokky --agent grok --detach
GW2="$(slot_window "$GWT" 2)"
assert_eq "$(T display -p -t "$GW2" '#{window_name}')" "grok#2" "second grok tab named grok#2"
GROK_SESSION_ID=sid-grok-2 hook api/grokky "$GW2" idle </dev/null
T kill-window -t "$GW"; T kill-window -t "$GW2"
"$WS" open api/grokky --detach
wait_until test -n "$(slot_window "$GWT" 2)"
assert_contains "$(wait_pane "$(slot_window "$GWT" 2)" sid-grok-2)" "--resume sid-grok-2" "grok resumed by GROK_SESSION_ID"
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
