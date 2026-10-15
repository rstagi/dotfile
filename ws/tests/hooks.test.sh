#!/bin/zsh
# ws tests: Basic `ws hook` state transitions, background subagents and early (pre-tag) hooks.
# Not standalone: ws/tests/run.sh sources helpers.sh then its group's area files in order, in one
# process, because workspaces, tabs and global session ids carry over within a group.
(( $+functions[ws_tests_require_sandbox] )) && ws_tests_require_sandbox || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

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

echo "ws locks: WS_LOCK_SCALE scales every lock wait; unset keeps the real budgets"
lock_waits() { ( source "$WS"; print -r -- "$(lock_wait 3) $(lock_wait 2) $(lock_wait 1)" ); }
assert_eq "$(unset WS_LOCK_SCALE; lock_waits)" "3 2 1" "default lock waits stay 3s/2s/1s (under the 5s hook timeout)"
assert_eq "$(WS_LOCK_SCALE=0.1 lock_waits)" "0.3 0.2 0.1" "WS_LOCK_SCALE multiplies each wait"
assert_eq "$(WS_LOCK_SCALE=bogus lock_waits)" "3 2 1" "an invalid scale falls back to the real budgets"
zmodload zsh/datetime zsh/system
: >> "$TMP/scaled.lock"
(zsystem flock -f held_fd "$TMP/scaled.lock"; : > "$TMP/scaled-held"; sleep 3) &
scaled_holder=$!
wait_until test -e "$TMP/scaled-held"
scaled_start=$EPOCHREALTIME
out="$(WS_LOCK_SCALE=0.1 zsh -c 'source "$1"; with_lock "$2" true' _ "$WS" "$TMP/scaled" 2>&1)"
assert_exit "$?" "1" "scaled with_lock still times out on a held lock"
assert_eq "$(( EPOCHREALTIME - scaled_start < 1.5 ))" "1" "scaled with_lock gives up after its scaled wait"
assert_contains "$out" "timed out waiting for lock" "scaled timeout keeps its diagnostic"
kill "$scaled_holder" 2>/dev/null; wait "$scaled_holder" 2>/dev/null
