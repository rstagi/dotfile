#!/bin/zsh
# Capture pending triggers in a caller-owned sandbox.
set -eu
zmodload zsh/system
WS_HOME="$1"
REPO="${0:A:h:h:h:h:h:h:h}"
[[ "$WS_HOME" == /tmp/* || "$WS_HOME" == /private/* ]] || exit 2
mkdir -p "$WS_HOME/sessions" "$WS_HOME/triggers"
source "$REPO/ws/lib/core.sh"
source "$REPO/ws/lib/bus.sh"
source "$REPO/ws/lib/triggers.sh"
workspace_name() { print -r -- 'repo/main'; }
WS_BIN=/usr/bin/true
ws_bus_allocate /repo 1 codex '' >/dev/null
ws_bus_allocate /repo 2 claude '' >/dev/null
ws_bus_event 1 exit '{"code":0}'
with_lock "$WS_TRIGGERS/registry" ws_bus_register_trigger '["1"]' exit send 2 'exited {id}' '' '' claude >/dev/null

if [[ "${2:-}" == done ]]; then
  print -rn -- $'completed {id} and {output}\n' > "$WS_HOME/result"
  with_lock "$WS_SESSIONS/1/output" ws_bus_done_locked 1 "$WS_HOME/result"
  ws_bus_event 1 done '{"output":"/legacy/output.md"}'
  with_lock "$WS_TRIGGERS/registry" ws_bus_register_trigger '["1"]' done send 2 'result {id}: {output}' 1 '' claude >/dev/null
fi
