#!/bin/zsh
# ws tests: Pins, picker rows/summary/cursor, unread marks, ws claude adoption, stopped-workspace restore.
# Not standalone: ws/tests/run.sh sources helpers.sh then its group's area files in order, in one
# process, because workspaces, tabs and global session ids carry over within a group.
(( $+functions[ws_tests_require_sandbox] )) && ws_tests_require_sandbox || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

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
