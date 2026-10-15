#!/bin/zsh
# ws tests: Review fixes: pane ownership, async done, lock bootstrap, delayed delivery, stop lock.
# Not standalone: ws/tests/run.sh sources helpers.sh then the area files in order, in one process,
# because workspaces, tabs and global session ids carry over between areas.
(( $+functions[ws_tests_require_sandbox] )) && ws_tests_require_sandbox || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

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

echo "ws done (F2): a slow trigger target neither stalls done nor delays done_window"
F2S_BEFORE="$(windows_of "$TRIGGER_WT")"
"$WS" add api/triggers --detach
F2S_WIN="$(windows_of "$TRIGGER_WT" | grep -vxF -f <(print -r -- "$F2S_BEFORE"))"
F2S_SRC="$(T display -p -t "$F2S_WIN" '#{@ws_session}')"
F2S_META="$WS_HOME/sessions/$F2S_SRC/meta"
jq '.spawned = true' "$F2S_META" > "$F2S_META.fixture" && mv "$F2S_META.fixture" "$F2S_META"
F2S_BEFORE="$(windows_of "$TRIGGER_WT")"
print 3 > "$WS_HOME/tui-delay"
"$WS" add api/triggers --detach
F2S_TWIN="$(windows_of "$TRIGGER_WT" | grep -vxF -f <(print -r -- "$F2S_BEFORE"))"
F2S_TGT="$(T display -p -t "$F2S_TWIN" '#{@ws_session}')"
F2S_TID="$("$WS" on "$F2S_SRC" done --send-to "$F2S_TGT" 'f2 slow msg')"
f2s_start=$EPOCHREALTIME
WS_READY_TIMEOUT=10 WS_SESSION="$F2S_SRC" "$WS" done </dev/null >/dev/null 2>&1
assert_eq "$(( EPOCHREALTIME - f2s_start < 2 ))" "1" "done returns while the target is still starting"
assert_eq "$(jq -r '.done_window // empty' "$F2S_META")" "$F2S_WIN" "done_window is recorded before any trigger runs"
assert_eq "$([[ -f "$WS_HOME/triggers/$F2S_TID.json" ]] && print pending)" "pending" "slow trigger is still in flight after done returns"
WS_TEST_WAIT=25 wait_until test -f "$WS_HOME/triggers/archive/$F2S_TID.json"
assert_exit "$?" "0" "detached worker delivers once the slow target is ready"
rm -f "$WS_HOME/tui-delay"

echo "ws hook (F8): a Stop that waits on the hook lock still settles the tab and drains the inbox"
# Own live target: F2's trigger delivered to DEFAULT_ID, whose fake agent read it and exited.
F8_BEFORE="$(windows_of "$TRIGGER_WT")"
"$WS" add api/triggers --detach
F8_WIN="$(windows_of "$TRIGGER_WT" | grep -vxF -f <(print -r -- "$F8_BEFORE"))"
F8_ID="$(T display -p -t "$F8_WIN" '#{@ws_session}')"
F8_LOCK="${TMPDIR:-/tmp}/ws-hook-$WS_TMUX_SOCKET-$F8_WIN.lock"
hook api/triggers "$F8_WIN" working </dev/null
(unset WS_SESSION; "$WS" send "$F8_ID" 'f8 msg' 2>/dev/null)
assert_exit "$?" "0" "send to the working tab queues without waiting"
rm -f "$TMP/f8-held" "$TMP/f8-release"
(zmodload zsh/system; : >> "$F8_LOCK"; zsystem flock -f f8_fd "$F8_LOCK"; : > "$TMP/f8-held"
  while [[ ! -e "$TMP/f8-release" ]]; do sleep 0.05; done) &
f8_holder=$!
wait_until test -e "$TMP/f8-held"
# WS_LOCK_SCALE: Stop starves after 0.3s (not 3s) and hands off to the detached retry.
(sleep 1.5; : > "$TMP/f8-release") &
WS_LOCK_SCALE=0.1 hook api/triggers "$F8_WIN" stop </dev/null >/dev/null
f8_settled() { [[ "$(T show -wqv -t "$F8_WIN" @ws_state)" != working ]]; }
WS_TEST_WAIT=12 wait_until f8_settled
assert_exit "$?" "0" "lock-starved Stop does not leave the tab working"
wait "$f8_holder"
# --- end review fixes ---

"$WS" rm api/triggers --force --delete-branch >/dev/null
