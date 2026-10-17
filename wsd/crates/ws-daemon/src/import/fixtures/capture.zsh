#!/bin/zsh
# Capture the zsh formats in a caller-owned sandbox.
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
# This fixture has no terminal. The paste remains unconfirmed.
tmux_() { return 0; }
ws_bus_paste() { return 0; }
WS_BIN=/usr/bin/true
ws_bus_allocate /repo 1 codex '' >/dev/null
ws_bus_allocate /repo 2 claude '' >/dev/null
print -rn -- 'completed output' > "$WS_HOME/completed"
with_lock "$WS_SESSIONS/1/output" ws_bus_done_locked 1 "$WS_HOME/completed"
ws_bus_event 1 done '{"output":"/legacy/output.md"}'
ws_bus_event 1 exit '{"code":0}'
print -rn -- $'initial prompt\nsecond line' > "$WS_SESSIONS/2/prompt"
with_lock "$WS_SESSIONS/2/inbox" ws_bus_enqueue 2 $'[ws terminal]\nunconfirmed paste'
with_lock "$WS_SESSIONS/2/inbox" ws_bus_idle_locked 2 @1
with_lock "$WS_SESSIONS/2/inbox" ws_bus_enqueue 2 $'[ws #1]\ninterrupted Stop reply'
with_lock "$WS_SESSIONS/2/inbox" ws_bus_stop_locked 2 >/dev/null
with_lock "$WS_SESSIONS/2/inbox" ws_bus_enqueue 2 $'[ws terminal]\nqueued message'
print -r -- 12 > "$WS_SESSIONS/counter"
