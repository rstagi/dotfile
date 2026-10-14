#!/bin/zsh
# ws tests: Basic `ws hook` state transitions, background subagents and early (pre-tag) hooks.
# Not standalone: ws/tests/run.sh sources helpers.sh then the area files in order, in one process,
# because workspaces, tabs and global session ids carry over between areas.
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
