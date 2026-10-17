#!/bin/zsh
# Capture the zsh command output in the ws test sandbox.
set -e
DEST="${0:A:h}"
source "${0:A:h}/../../../../../ws/tests/helpers.sh"
source "$WS"
mkdir -p "$WS_SESSIONS"/{1,2,3}
win="$(T display -p -t keepalive '#{window_id}')"
T set -w -t "$win" @ws_session 1
T set -w -t "$win" @ws_state idle
for id in 1 2 3; do
  label="" agent=claude lifecycle=active window="$win"
  if (( id == 2 )); then label=finisher; agent=codex; lifecycle=done; window=""; fi
  if (( id == 3 )); then label=old; agent=grok; lifecycle=closed; window="$win"; fi
  jq -nc --arg id "$id" --arg name "$label" --arg agent "$agent" --arg status "$lifecycle" --arg window "$window" \
    '{id:$id,name:$name,workspace:"api/seed",agent:$agent,status:$status,window:$window}' > "$WS_SESSIONS/$id/meta"
done
print result > "$WS_SESSIONS/2/output.md"
# Only ids and sandbox paths change in the Rust output.
"$WS" sessions | sed -E 's/^([0-9]+)/#\1/' > "$DEST/sessions.txt"
"$WS" sessions finisher | sed -E 's/^([0-9]+)/#\1/' > "$DEST/selected.txt"
"$WS" show finisher | sed -E -e 's/^id: ([0-9]+)/id: #\1/' -e 's|^output: .+|output: <OUTPUT>|' > "$DEST/show.txt"
"$WS" show 1 | sed -E 's/^id: ([0-9]+)/id: #\1/' > "$DEST/show-empty.txt"
WS_SESSION=1 "$WS" whoami | sed -E 's/^([0-9]+)/#\1/' > "$DEST/whoami.txt"
WS_SESSION=1 "$WS" name builder > "$DEST/name.txt"
WS_SESSION=1 "$WS" name finisher 2> "$DEST/name-taken.txt" && exit 1
WS_SESSION=1 "$WS" name me 2> "$DEST/name-invalid.txt" && exit 1
WS_SESSION= "$WS" whoami 2> "$DEST/whoami-outside.txt" && exit 1
"$WS" show absent 2> "$DEST/missing.txt" && exit 1
exit 0
