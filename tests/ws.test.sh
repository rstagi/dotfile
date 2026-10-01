#!/bin/zsh
# ws.sh — terminal workspace manager (worktree + one tmux window per agent session). Runs
# against a private tmux server (-L) with fake agent bins, so it never touches the user's.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
export WS_TMUX_SOCKET="ws-test-$$"
export WS_ROOT="$TMP/worktrees"
export WS_HOME="$TMP/wshome"
export WS_NOTIFY=0
export WS_AGENT_SHELL="zsh -fc"
export LOOP_REPO_REGISTRY="$TMP/repos.json"
trap 'tmux -L "$WS_TMUX_SOCKET" kill-server 2>/dev/null; rm -rf "$TMP"' EXIT
WS="$ROOT/ws.sh"
T() { tmux -L "$WS_TMUX_SOCKET" "$@"; }

# Fake agents print their argv, then stay alive until a line arrives (→ clean exit 0).
mkdir -p "$TMP/bin"
for agent in claude codex; do
  cat > "$TMP/bin/$agent" <<EOF
#!/bin/sh
echo "FAKE-$agent \$* WS=\$WS_WORKSPACE"
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

# wait_until <cmd...> — poll until the command succeeds (max 3s)
wait_until() {
  local i
  for i in {1..30}; do "$@" && return 0; sleep 0.1; done
  return 1
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
T kill-window -t "$WIN"; T kill-window -t "$W2"
assert_eq "$(windows_of "$WT")" "" "both windows gone (simulated reboot)"
assert_contains "$("$WS" list)" $'stopped\tapi/feat-login\t' "workspace shows stopped"
"$WS" open api/feat-login --detach
wait_until test -n "$(slot_window "$WT" 2)"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" sid-claude-1)" "--resume sid-claude-1" "claude session resumed by id"
assert_contains "$(wait_pane "$(slot_window "$WT" 2)" sid-codex-2)" "resume sid-codex-2" "codex session resumed by id"
assert_eq "$(session_of "$(slot_window "$WT" 2)")" "api/feat-login" "restored into one worktree session"

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
cat > "$TMP/bin/terminal-notifier" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_NOTIFIER_OUT"
EOF
chmod +x "$TMP/bin/terminal-notifier"
export FAKE_NOTIFIER_OUT="$TMP/notifier.out"
NWIN="$(slot_window "$WT" 1)"
WS_NOTIFY=1 WS_TERMINAL_BUNDLE=com.example.term hook api/feat-login "$NWIN" working </dev/null
WS_NOTIFY=1 WS_TERMINAL_BUNDLE=com.example.term hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)"
assert_contains "$nout" "api/feat-login needs input" "message names workspace + state"
assert_contains "$nout" $'-sound\n' "plays a sound"
assert_contains "$nout" $'-activate\ncom.example.term' "click activates the terminal app"
assert_contains "$nout" "switch-client" "click switches tmux to the tab"
assert_contains "$nout" "$NWIN" "click targets the notifying tab"
assert_contains "$nout" $'-group\nws-'"$NWIN" "one notification per tab (replaced, not stacked)"
rm -f "$FAKE_NOTIFIER_OUT"
: > "$TMP/icon.png"
WS_NOTIFY=1 WS_NOTIFY_ICON_DONE="$TMP/icon.png" hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)"
assert_contains "$nout" $'-sound\nGlass' "done plays Glass by default"
assert_contains "$nout" $'-contentImage\n'"$TMP/icon.png" "done icon attached (configurable)"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT")"
assert_contains "$nout" $'-sound\nPing' "needs input plays Ping by default"
assert_contains "$nout" $'-contentImage\n'"$ROOT/assets/ws/waiting.png" "needs input uses the bundled waiting icon"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 WS_NOTIFY_SOUND_DONE=Pop hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT")"
assert_contains "$nout" $'-sound\nPop' "done sound overridable"
assert_contains "$nout" $'-contentImage\n'"$ROOT/assets/ws/done.png" "done uses the bundled done icon"
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 hook api/feat-login "$NWIN" working </dev/null
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "no notification for working"
hook api/feat-login "$NWIN" idle </dev/null

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
EOF
chmod +x "$TMP/bin/gh"
export FAKE_GH_LOG="$TMP/gh.log"
(cd "$MWT" && print n | "$WS" merge) >/dev/null 2>&1
assert_contains "$(cat "$FAKE_GH_LOG")" "pr merge feat/ship --squash" "squash-merges the branch's PR"
assert_eq "$(git -C "$MREPO" log -1 --format=%s main)" "squashed: ship it" "main checkout fast-forwarded"
assert_eq "$([[ -d "$MWT" ]] && print yes)" "yes" "declined: worktree kept"
assert_eq "$(git -C "$MREPO" branch --list feat/ship | tr -d ' *+')" "feat/ship" "declined: branch kept"
(cd "$MWT" && print y | "$WS" merge) >/dev/null 2>&1
assert_eq "$([[ -d "$MWT" ]] && print yes)" "" "confirmed: worktree removed"
assert_eq "$(git -C "$MREPO" branch --list feat/ship)" "" "confirmed: unmerged-by-ancestry (squashed) branch force-deleted"
assert_eq "$(git -C "$MREPO" ls-remote --heads origin feat/ship)" "" "confirmed: remote branch deleted"
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
