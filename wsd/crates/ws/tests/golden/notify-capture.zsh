#!/bin/zsh
# Record the notification transitions from ws/tests/notify.test.sh in a private sandbox.
set -e
DEST="${0:A:h}"
source "${0:A:h}/../../../../../ws/tests/helpers.sh"
repo="$(make_repo api)"
"$WS" new --repo "$repo" --branch feat-login --detach >/dev/null
win="$(slot_window "$WS_ROOT/api/feat-login" 1)"
T set -w -t "$win" @ws_state idle
mkdir -p "$TMP/adapter"
cat > "$TMP/adapter/capture" <<'ADAPTER'
#!/bin/sh
cat >> "$FAKE_EVENT_OUT"
ADAPTER
chmod +x "$TMP/adapter/capture"
export FAKE_EVENT_OUT="$TMP/events.jsonl"
event_count_at_least() { (( $(wc -l < "$FAKE_EVENT_OUT") >= $1 )); }
step() {
  print -r -- "$2" | WS_NOTIFY=1 WS_NOTIFIER="$TMP/adapter/capture" hook api/feat-login "$win" "$1" >/dev/null
  wait_until event_count_at_least "$3"
}
: > "$FAKE_EVENT_OUT"
step working '{}' 1
step waiting '{"message":"Need permission"}' 2
step waiting '{"message":"Duplicate"}' 2
step working '{}' 3
step idle '{"last_assistant_message":"Done"}' 4
WS_NOTIFY=1 WS_NOTIFIER="$TMP/adapter/capture" "$WS" _dismiss "$win"
wait_until event_count_at_least 5
jq -c '{state,prev,message,detail,agent,repo,branch,focused,sound}' "$FAKE_EVENT_OUT" > "$DEST/notify-events.jsonl"
