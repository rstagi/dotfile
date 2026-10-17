#!/bin/zsh
# Capture hook states from the existing zsh hook scenarios in a private sandbox.
set -e
DEST="${0:A:h}"
source "${0:A:h}/../../../../../ws/tests/helpers.sh"
repo="$(make_repo api)"
"$WS" new --repo "$repo" --branch hooks --detach >/dev/null
win="$(slot_window "$WS_ROOT/api/hooks" 1)"
: > "$DEST/hook-states.txt"
step() {
  print -r -- "$2" | hook api/hooks "$win" "$1" >/dev/null
  T display -p -t "$win" '#{@ws_state}|#{@ws_main}|#{@ws_turn}|#{@ws_sub_a}|#{@ws_sub_b}|#{@ws_unread}' >> "$DEST/hook-states.txt"
}
step working '{"hook_event_name":"UserPromptSubmit","prompt_id":"p1"}'
step waiting '{"hook_event_name":"PermissionRequest"}'
step working '{"hook_event_name":"PreToolUse"}'
step subagent-start '{"agent_id":"a"}'
step subagent-start '{"agent_id":"b"}'
step idle '{}'
step working '{"agent_id":"a"}'
step subagent-stop '{"agent_id":"a"}'
step subagent-stop '{"agent_id":"b"}'
step working '{"hook_event_name":"UserPromptSubmit","turn_id":"t2"}'
step cancel '{"hook_event_name":"Interrupt","turn_id":"old"}'
step cancel '{"hook_event_name":"Interrupt","turn_id":"t2"}'
