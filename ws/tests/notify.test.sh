#!/bin/zsh
# ws tests: Notification adapters (terminal-notifier, osascript, boringnotch, custom), details, sounds, dismissal, notifier build.
# Not standalone: ws/tests/run.sh sources helpers.sh then the area files in order, in one process,
# because workspaces, tabs and global session ids carry over between areas.
[[ -n "${WS_TESTS_SUITE:-}" ]] || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

echo "ws hook: notifies via terminal-notifier (sound, click focuses the tab) when unfocused"
cat > "$TMP/bin/afplay" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_AFPLAY_OUT"
printf '%s\n' "$@" >> "$FAKE_AFPLAY_LOG"
[ -n "${FAKE_AFPLAY_SLEEP:-}" ] && sleep "$FAKE_AFPLAY_SLEEP" && : > "$FAKE_AFPLAY_OUT.done"
exit "${FAKE_AFPLAY_EXIT:-0}"
EOF
chmod +x "$TMP/bin/afplay"
export FAKE_AFPLAY_OUT="$TMP/afplay.out"
export FAKE_AFPLAY_LOG="$TMP/afplay.log"
cat > "$TMP/bin/terminal-notifier" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_NOTIFIER_OUT"
EOF
chmod +x "$TMP/bin/terminal-notifier"
export FAKE_NOTIFIER_OUT="$TMP/notifier.out"
NWIN="$(slot_window "$WT" 1)"
WS_NOTIFY=1 WS_TERMINAL_BUNDLE=com.example.term hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 WS_TERMINAL_BUNDLE=com.example.term hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)"
assert_contains "$nout" "api/feat-login needs input" "message names workspace + state"
assert_eq "$(afplay_out)" "$ROOT/assets/ws/sounds/waiting.wav" "waiting plays the bundled rising cue by default"
assert_eq "$([[ "$nout" == *-sound* ]] && print yes)" "" "terminal-notifier sound suppressed to avoid double playback"
assert_contains "$nout" $'-activate\ncom.example.term' "click activates the terminal app"
assert_contains "$nout" "switch-client" "click switches tmux to the tab"
assert_contains "$nout" "$NWIN" "click targets the notifying tab"
assert_contains "$nout" $'-group\nws-'"$NWIN" "one notification per tab (replaced, not stacked)"
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
: > "$TMP/icon.png"
WS_NOTIFY=1 WS_NOTIFY_ICON_DONE="$TMP/icon.png" hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)"
assert_eq "$(afplay_out)" "$ROOT/assets/ws/sounds/done.wav" "done plays the bundled soft chime by default"
assert_contains "$nout" $'-contentImage\n'"$TMP/icon.png" "done icon attached (configurable)"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT")"
assert_eq "$(afplay_out)" "$ROOT/assets/ws/sounds/waiting.wav" "waiting sound resolves independently from done"
assert_contains "$nout" $'-contentImage\n'"$ROOT/assets/ws/waiting.png" "needs input uses the bundled waiting icon"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 WS_NOTIFY_SOUND_DONE=Pop hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
nout="$(cat "$FAKE_NOTIFIER_OUT")"
assert_eq "$(afplay_out)" "/System/Library/Sounds/Pop.aiff" "done sound overridable with a system sound name"
assert_contains "$nout" $'-contentImage\n'"$ROOT/assets/ws/done.png" "done uses the bundled done icon"
echo "ws hook: a relative sound file with spaces resolves from the hook cwd"
cp "$ROOT/assets/ws/sounds/done.wav" "$TMP/custom done.wav"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
(cd "$TMP" && WS_NOTIFY=1 WS_NOTIFY_SOUND_DONE="custom done.wav" hook api/feat-login "$NWIN" idle </dev/null)
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_eq "$(afplay_out)" "custom done.wav" "relative sound path is passed as one argument"
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 hook api/feat-login "$NWIN" working </dev/null
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "no notification for working"
hook api/feat-login "$NWIN" idle </dev/null

echo "ws hook: custom adapter receives one v1 JSON event on stdin"
mkdir -p "$TMP/custom adapters"
cat > "$TMP/custom adapters/capture" <<'EOF'
#!/bin/sh
cat > "$FAKE_EVENT_OUT"
EOF
chmod +x "$TMP/custom adapters/capture"
export FAKE_EVENT_OUT="$TMP/event.json"
rm -f "$FAKE_AFPLAY_OUT"
hook api/feat-login "$NWIN" working </dev/null
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$([[ -e "$FAKE_AFPLAY_OUT" ]] && print yes)" "" "custom adapter owns sound playback"
jq -e --arg id "ws:$NWIN" '
  .v == 1 and .id == $id and .source == "ws" and
  .state == "waiting" and .prev == "working" and
  .title == "ws" and .message == "api/feat-login needs input" and
  .detail == "" and .agent == "claude" and .repo == "api" and .branch == "feat/login" and
  .focused == false and .sound == "waiting" and
  (.ts | type == "number") and .ts > 0 and
  (.actions | length == 1) and .actions[0].id == "focus" and .actions[0].label == "Focus tab" and
  (.actions[0].command | contains("select-window") and contains("switch-client"))
' "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "custom adapter receives the v1 schema and workspace metadata"
assert_eq "$(wc -l < "$FAKE_EVENT_OUT" 2>/dev/null | tr -d ' ')" "1" "event is one compact JSON line"
assert_contains "$(jq -r '.actions[0].command' "$FAKE_EVENT_OUT" 2>/dev/null)" "$NWIN" "event focus action targets the tab"
rm -f "$FAKE_EVENT_OUT" "$FAKE_NOTIFIER_OUT"
SPECIAL_WORKSPACE=$'api/feat-"quoted"\\line\nlogin'
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook "$SPECIAL_WORKSPACE" "$NWIN" idle </dev/null
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.message' "$FAKE_EVENT_OUT")" "$SPECIAL_WORKSPACE is idle" "JSON preserves quotes, backslashes and newlines"
assert_eq "$(jq -r '.id, .state, .prev, .sound' "$FAKE_EVENT_OUT")" $'ws:'"$NWIN"$'\nidle\nwaiting\ndone' "same tab identity persists across states"
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "custom adapter bypasses the built-in notifier"
rm -f "$FAKE_EVENT_OUT"
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle </dev/null
sleep 0.2
assert_eq "$([[ -e "$FAKE_EVENT_OUT" ]] && print yes)" "" "repeated state does not reach a custom adapter"
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" working </dev/null
wait_until test -s "$FAKE_EVENT_OUT"
jq -e '.state == "working" and .prev == "idle" and .focused == false and .sound == ""' \
  "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "custom adapter receives working transitions without a sound cue"
rm -f "$FAKE_EVENT_OUT"
WS_NOTIFY=0 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting </dev/null
sleep 0.2
assert_eq "$([[ -e "$FAKE_EVENT_OUT" ]] && print yes)" "" "WS_NOTIFY=0 disables custom adapters too"

echo "ws hook: events carry a hook-entry seq; stale or concurrent duplicate hooks are dropped"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_EVENT_OUT"
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" WS_TERMINAL_BUNDLE=com.example.term \
  hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_EVENT_OUT"
jq -e '(.seq | type == "number") and .seq == (.seq | floor) and .seq > 1000000000000000' \
  "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "event seq is integer microseconds since epoch"
assert_eq "$(T display -p -t "$NWIN" '#{@ws_seq}')" "$(jq -r '.seq' "$FAKE_EVENT_OUT" 2>/dev/null)" "window stores last applied seq"
assert_contains "$(jq -r '.actions[0].command' "$FAKE_EVENT_OUT" 2>/dev/null)" "open -b com.example.term" "focus action activates the terminal app"
rm -f "$FAKE_EVENT_OUT"
T set -w -t "$NWIN" @ws_seq 99999999999999999
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting </dev/null
sleep 0.2
assert_eq "$(T display -p -t "$NWIN" '#{@ws_state}')" "idle" "stale hook does not change state"
assert_eq "$([[ -e "$FAKE_EVENT_OUT" ]] && print yes)" "" "stale hook emits no event"
T set -wu -t "$NWIN" @ws_seq
cat > "$TMP/custom adapters/append" <<'EOF'
#!/bin/sh
cat >> "$FAKE_EVENT_LOG"
EOF
chmod +x "$TMP/custom adapters/append"
export FAKE_EVENT_LOG="$TMP/events.log"
rm -f "$FAKE_EVENT_LOG"
for i in 1 2 3 4 5 6; do
  WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/append" hook api/feat-login "$NWIN" waiting </dev/null &
done
wait
wait_until test -s "$FAKE_EVENT_LOG"
sleep 0.3
assert_eq "$(wc -l < "$FAKE_EVENT_LOG" 2>/dev/null | tr -d ' ')" "1" "simultaneous identical hooks emit exactly one event"
assert_eq "$(T display -p -t "$NWIN" '#{@ws_state}')" "waiting" "simultaneous hooks apply the state"
hook api/feat-login "$NWIN" idle </dev/null

echo "ws hook: seq is one realtime snapshot (no second-boundary skew)"
ws_fn() { ( source "$WS" && "$@" ) }
assert_eq "$(ws_fn realtime_us 1791187219.9999999)" "1791187219999999" "seq truncates to microseconds"
assert_eq "$(ws_fn realtime_us 1791187220.0000001)" "1791187220000000" "seq after the second boundary"
assert_eq "$(ws_fn realtime_us 1791187220.5)" "1791187220500000" "short fraction right-padded"

echo "ws hook: seq is captured at entry, before reading the payload"
hook api/feat-login "$NWIN" working </dev/null
mkfifo "$TMP/slow.in"
exec {slow_w}<>"$TMP/slow.in"
rm -f "$FAKE_EVENT_LOG"
# Children must not inherit the writer, or the slow hook never sees EOF.
(exec {slow_w}>&-; WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/append" hook api/feat-login "$NWIN" waiting <"$TMP/slow.in") &
slow_pid=$!
sleep 0.5 # older hook has taken its seq and blocks in payload read
(exec {slow_w}>&-; WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/append" hook api/feat-login "$NWIN" idle </dev/null)
exec {slow_w}>&-
wait "$slow_pid"
wait_until test -s "$FAKE_EVENT_LOG"
sleep 0.3
assert_eq "$(T display -p -t "$NWIN" '#{@ws_state}')" "idle" "newer hook wins over an older slow-payload hook"
assert_eq "$(jq -r '.state' "$FAKE_EVENT_LOG" 2>/dev/null)" "idle" "only the newer hook emits"

echo "ws hook: fails closed when the tab lock can't be taken"
hook api/feat-login "$NWIN" working </dev/null
HOOK_LOCK="${TMPDIR:-/tmp}/ws-hook-$WS_TMUX_SOCKET-$NWIN.lock"
rm -f "$TMP/locked" "$FAKE_EVENT_OUT"
zsh -fc 'zmodload zsh/system; zsystem flock -f fd "$1" && : > "$2" && sleep 4' _ "$HOOK_LOCK" "$TMP/locked" &
holder_pid=$!
wait_until test -e "$TMP/locked"
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle </dev/null
sleep 0.2
assert_eq "$(T display -p -t "$NWIN" '#{@ws_state}')" "working" "unlocked hook leaves state untouched"
assert_eq "$([[ -e "$FAKE_EVENT_OUT" ]] && print yes)" "" "unlocked hook emits nothing"
kill "$holder_pid" 2>/dev/null
wait "$holder_pid" 2>/dev/null
rm -f "$FAKE_EVENT_OUT"
hook api/feat-login "$NWIN" idle </dev/null

echo "ws notify: built-in adapter delivers per tab in seq order"
notify_event() {
  jq -nc --arg id "ws:$NWIN" --arg state "$1" --argjson seq "$2" \
    '{v:1, id:$id, source:"ws", state:$state, prev:"", title:"ws", message:("tab is " + $state),
      detail:"", focused:false, sound:"", actions:[{id:"focus", label:"Focus tab", command:"true"}], seq:$seq}'
}
deliver() { notify_event "$1" "$2" | WS_NOTIFIER=terminal-notifier ws_fn notify_adapter; }
NSEQ="$(ws_fn realtime_us "$(zsh -fc 'zmodload zsh/datetime; print $EPOCHREALTIME')")"
rm -f "$FAKE_NOTIFIER_OUT"
deliver waiting $(( NSEQ + 2 ))
assert_contains "$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "tab is waiting" "newer event posted"
rm -f "$FAKE_NOTIFIER_OUT"
deliver idle $(( NSEQ + 1 ))
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "older event arriving late is dropped"
deliver working $(( NSEQ + 4 ))
deliver idle $(( NSEQ + 3 ))
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "newer working (no banner) still supersedes an older idle"
deliver idle $(( NSEQ + 5 ))
assert_contains "$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "tab is idle" "next newer event posted"
rm -f "$FAKE_NOTIFIER_OUT"
deliver closed $(( NSEQ + 6 ))
nout="$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)"
assert_contains "$nout" "-remove" "closed removes the banner"
assert_contains "$nout" "ws-$NWIN" "closed targets the tab's banner group"
assert_eq "$([[ "$nout" == *-message* ]] && print yes)" "" "closed posts no new banner"
T set -wu -t "$NWIN" @ws_notify_seq
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"

echo "ws hook: Claude Notification includes its message as detail"
DETAIL=$'Approve "deploy"?\\line\nWaiting…'
hook api/feat-login "$NWIN" working </dev/null
jq -nc --arg message "$DETAIL" '{hook_event_name:"Notification", notification_type:"permission_prompt", message:$message}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.detail' "$FAKE_EVENT_OUT")" "$DETAIL" "Notification detail preserves quotes, backslashes, newlines and Unicode"

echo "ws hook: Claude Stop includes the last assistant message"
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"hook_event_name":"Stop","last_assistant_message":"Fixed the login flow."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.detail' "$FAKE_EVENT_OUT")" "Fixed the login flow." "Stop detail is the last assistant message"

echo "ws hook: Grok Stop camelCase lastAssistantMessage is detail"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"hookEventName":"Stop","lastAssistantMessage":"Grok finished."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.detail' "$FAKE_EVENT_OUT")" "Grok finished." "Grok Stop detail is lastAssistantMessage"

echo "ws hook: UserPromptSubmit includes the user's prompt"
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"hook_event_name":"UserPromptSubmit","prompt":"Fix login for guest users."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" working
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.detail' "$FAKE_EVENT_OUT")" "Fix login for guest users." "working detail is the submitted prompt"

echo "ws hook: detail is bounded to 1024 Unicode characters"
rm -f "$FAKE_EVENT_OUT"
jq -nc '{hook_event_name:"Stop", last_assistant_message:("Header: " + ("é" * 1016) + "TAIL")}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle
wait_until test -s "$FAKE_EVENT_OUT"
jq -e '(.detail | length) == 1024 and (.detail | startswith("Header: ") and endswith("é") and (contains("TAIL") | not))' \
  "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "detail truncates by characters without corrupting Unicode"

echo "ws hook: detail preserves trailing newlines"
rm -f "$FAKE_EVENT_OUT"
DETAIL=$'Please review.\n\n'
jq -nc --arg prompt "$DETAIL" '{hook_event_name:"UserPromptSubmit", prompt:$prompt}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" working
wait_until test -s "$FAKE_EVENT_OUT"
jq -e --arg detail "$DETAIL" '.detail == $detail' "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "detail preserves the submitted text's trailing newlines"

echo "ws hook: malformed or unavailable detail never drops the transition"
for payload in '{}' '{' '[]' 'null' '{} {}' '{"message":42,"last_assistant_message":[],"prompt":false}'; do
  hook api/feat-login "$NWIN" working </dev/null
  rm -f "$FAKE_EVENT_OUT"
  print -r -- "$payload" | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" idle
  assert_exit "$?" "0" "hook accepts unavailable detail: $payload"
  wait_until test -s "$FAKE_EVENT_OUT"
  jq -e '.state == "idle" and .prev == "working" and .detail == ""' "$FAKE_EVENT_OUT" >/dev/null 2>&1
  assert_exit "$?" "0" "transition survives unavailable detail: $payload"
done

echo "ws hook: non-text fields do not hide an available assistant message"
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"message":{},"last_assistant_message":"Available text.","prompt":"Lower-priority text."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting
wait_until test -s "$FAKE_EVENT_OUT"
assert_eq "$(jq -r '.detail' "$FAKE_EVENT_OUT")" "Available text." "detail selects the first available text field"

echo "ws hook: Codex Stop and UserPromptSubmit include detail and tab metadata"
"$WS" add api/feat-login --agent codex --detach
CWIN="$("$WS" list | awk -F '\t' -v p="$WT" '$3 == p && $4 == "codex" { print $5; exit }')"
hook api/feat-login "$CWIN" working </dev/null
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"session_id":"sid-codex-details","turn_id":"turn-1","hook_event_name":"Stop","transcript_path":null,"last_assistant_message":"Codex finished."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$CWIN" idle
wait_until test -s "$FAKE_EVENT_OUT"
jq -e --arg id "ws:$CWIN" '
  .id == $id and .agent == "codex" and .repo == "api" and .branch == "feat/login" and
  .state == "idle" and .prev == "working" and .focused == false and .detail == "Codex finished."
' "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "Codex Stop detail belongs to the Codex tab"
rm -f "$FAKE_EVENT_OUT"
print -r -- '{"hook_event_name":"UserPromptSubmit","prompt":"Continue the Codex task."}' \
  | WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$CWIN" working
wait_until test -s "$FAKE_EVENT_OUT"
jq -e '.agent == "codex" and .state == "working" and .detail == "Continue the Codex task."' "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "Codex working detail is the submitted prompt"
hook api/feat-login "$NWIN" waiting </dev/null

echo "ws hook: explicit osascript selection bypasses terminal-notifier"
cat > "$TMP/bin/osascript" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_OSASCRIPT_OUT"
EOF
chmod +x "$TMP/bin/osascript"
export FAKE_OSASCRIPT_OUT="$TMP/osascript.out"
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 WS_NOTIFIER=osascript hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_contains "$(cat "$FAKE_OSASCRIPT_OUT")" "api/feat-login is idle" "osascript receives the event message"
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" ]] && print yes)" "" "explicit osascript ignores an installed terminal-notifier"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 WS_NOTIFIER=terminal-notifier hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_eq "$(afplay_out)" "$ROOT/assets/ws/sounds/waiting.wav" "explicit terminal-notifier plays the bundled waiting sound"
assert_eq "$([[ -e "$FAKE_OSASCRIPT_OUT" ]] && print yes)" "" "successful terminal-notifier does not fall back"

echo "ws hook: every built-in adapter honors per-state sound file overrides"
cp "$ROOT/assets/ws/sounds/waiting.wav" "$TMP/custom waiting.wav"
for adapter in auto terminal-notifier osascript; do
  for state in idle waiting; do
    hook api/feat-login "$NWIN" working </dev/null
    rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_OUT"
    WS_NOTIFY=1 WS_NOTIFIER="$adapter" WS_NOTIFY_SOUND_DONE="$TMP/custom done.wav" \
      WS_NOTIFY_SOUND_WAITING="$TMP/custom waiting.wav" hook api/feat-login "$NWIN" "$state" </dev/null
    if [[ "$adapter" == osascript ]]; then
      wait_until test -s "$FAKE_OSASCRIPT_OUT"
    else
      wait_until test -s "$FAKE_NOTIFIER_OUT"
    fi
    sound_file="$TMP/custom done.wav"
    [[ "$state" == waiting ]] && sound_file="$TMP/custom waiting.wav"
    assert_eq "$(afplay_out)" "$sound_file" "$adapter resolves $state override with spaces"
  done
done

echo "ws hook: osascript resolves system and user-library sound names"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 WS_NOTIFIER=osascript WS_NOTIFY_SOUND_WAITING=Ping hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_eq "$(afplay_out)" "/System/Library/Sounds/Ping.aiff" "waiting sound accepts a system name"
mkdir -p "$TMP/sound-home/Library/Sounds"
cp "$ROOT/assets/ws/sounds/done.wav" "$TMP/sound-home/Library/Sounds/Pop.aiff"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_OUT"
HOME="$TMP/sound-home" WS_NOTIFY=1 WS_NOTIFIER=osascript WS_NOTIFY_SOUND_DONE=Pop \
  hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_eq "$(afplay_out)" "$TMP/sound-home/Library/Sounds/Pop.aiff" "user-library sound takes precedence over a system sound"

echo "ws hook: unavailable sound and playback failure do not prevent notifications"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
WS_NOTIFY=1 WS_NOTIFY_SOUND_DONE="$TMP/missing.wav" hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_contains "$(cat "$FAKE_NOTIFIER_OUT")" "api/feat-login is idle" "missing sound still displays the notification"
assert_eq "$([[ -e "$FAKE_AFPLAY_OUT" ]] && print yes)" "" "missing sound is not played"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 FAKE_AFPLAY_EXIT=1 hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_contains "$(cat "$FAKE_NOTIFIER_OUT")" "api/feat-login needs input" "failed playback still displays the notification"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"

echo "ws hook: sound playback never delays the notification"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT" "$FAKE_AFPLAY_OUT.done"
WS_NOTIFY=1 FAKE_AFPLAY_SLEEP=3 hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_eq "$([[ -e "$FAKE_AFPLAY_OUT.done" ]] && print yes)" "" "notification posts while the sound is still playing"

echo "ws hook: unknown WS_NOTIFIER falls back to auto"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 WS_NOTIFIER=boringNotch hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_contains "$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "api/feat-login is idle" "typo'd adapter name still notifies"

echo "ws hook: sound names match any extension in the sound libraries"
hook api/feat-login "$NWIN" working </dev/null
cp "$ROOT/assets/ws/sounds/done.wav" "$TMP/sound-home/Library/Sounds/MyChime.caf"
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_AFPLAY_OUT"
HOME="$TMP/sound-home" WS_NOTIFY=1 WS_NOTIFY_SOUND_DONE=MyChime hook api/feat-login "$NWIN" idle </dev/null
assert_eq "$(afplay_out)" "$TMP/sound-home/Library/Sounds/MyChime.caf" "non-aiff user sound resolves by name"

echo "ws hook: osascript uses the event title"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_OSASCRIPT_OUT"
WS_NOTIFY=1 WS_NOTIFIER=osascript hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
oout="$(cat "$FAKE_OSASCRIPT_OUT" 2>/dev/null)"
assert_eq "$([[ "$oout" == *'title "ws"'* ]] && print yes)" "" "osascript title is not hard-coded"
assert_contains "$oout" $'api/feat-login is idle\nws' "osascript receives the event title as an argument"
rm -f "$FAKE_OSASCRIPT_OUT"

echo "ws hook: boringnotch sends one NDJSON event and leaves sound to the app"
NOTCH_SOCKET="$TMP/notch notify.sock"
NOTCH_EVENT="$TMP/notch.ndjson"
start_socket_listener "$NOTCH_SOCKET" "$NOTCH_EVENT"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"
print -r -- '{"message":"Approve deployment?\nTarget: staging"}' \
  | WS_NOTIFY=1 WS_NOTIFIER=boringnotch WS_BORINGNOTCH_SOCKET="$NOTCH_SOCKET" \
    hook api/feat-login "$NWIN" waiting
wait_until test -s "$NOTCH_EVENT"
assert_exit "$?" "0" "notch receives the event"
jq -e --arg id "ws:$NWIN" '
  .v == 1 and .id == $id and .state == "waiting" and .prev == "working" and
  .focused == false and .sound == "waiting" and .detail == "Approve deployment?\nTarget: staging" and
  (.actions[0].command | contains("select-window") and contains("switch-client"))
' "$NOTCH_EVENT" >/dev/null 2>&1
assert_exit "$?" "0" "notch receives the event with detail and focus action"
assert_eq "$(wc -l < "$NOTCH_EVENT" | tr -d ' ')" "1" "notch receives exactly one newline-terminated JSON object"
sleep 0.3
assert_eq "$([[ -e "$FAKE_AFPLAY_LOG" ]] && print yes)" "" "successful notch delivery leaves sound to the app"
assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" || -e "$FAKE_OSASCRIPT_OUT" ]] && print yes)" "" "successful notch delivery bypasses macOS notifications"
stop_socket_listener
rm -f "$NOTCH_SOCKET" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"

echo "ws hook: boringnotch uses its default socket path"
DEFAULT_NOTCH_SOCKET="$NOTCH_HOME/Library/Application Support/boringNotch/notify.sock"
mkdir -p "${DEFAULT_NOTCH_SOCKET:h}"
start_socket_listener "$DEFAULT_NOTCH_SOCKET" "$NOTCH_EVENT"
HOME="$NOTCH_HOME" WS_NOTIFY=1 WS_NOTIFIER=boringnotch WS_BORINGNOTCH_SOCKET="" \
  WS_NOTIFY_SOUND_DONE="$TMP/custom done.wav" hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$NOTCH_EVENT"
jq -e '.state == "idle" and .prev == "waiting" and .sound == "done"' "$NOTCH_EVENT" >/dev/null 2>&1
assert_exit "$?" "0" "notch receives idle at the default socket under HOME"
sleep 0.3
assert_eq "$([[ -e "$FAKE_AFPLAY_LOG" ]] && print yes)" "" "notch delivery ignores ws sound overrides"
stop_socket_listener
rm -f "$DEFAULT_NOTCH_SOCKET" "$FAKE_AFPLAY_LOG"

echo "ws hook: boringnotch falls back to auto when the socket is missing"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 WS_NOTIFIER=boringnotch WS_BORINGNOTCH_SOCKET="$NOTCH_SOCKET" \
  hook api/feat-login "$NWIN" waiting </dev/null
assert_exit "$?" "0" "missing notch socket does not fail the hook"
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_contains "$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "api/feat-login needs input" "missing notch socket uses auto notification"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/waiting.wav" "missing socket fallback plays its cue exactly once"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"

echo "ws hook: boringnotch falls back when a socket has no listener"
start_socket_listener "$NOTCH_SOCKET" "$NOTCH_EVENT"
stop_socket_listener
assert_eq "$([[ -S "$NOTCH_SOCKET" ]] && print yes)" "yes" "stale socket remains without a listener"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 WS_NOTIFIER=boringnotch WS_BORINGNOTCH_SOCKET="$NOTCH_SOCKET" \
  hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_contains "$(cat "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "api/feat-login needs input" "unreachable notch socket uses auto notification"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/waiting.wav" "unreachable socket fallback plays its cue exactly once"
rm -f "$NOTCH_SOCKET" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"

echo "ws dismiss: clears an open notification once with a closed event"
APPEND="$TMP/custom adapters/append"
event_count() { wc -l < "$FAKE_EVENT_LOG" 2>/dev/null | tr -d ' '; }
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_EVENT_LOG"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_EVENT_LOG"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" "$WS" _dismiss "$NWIN"
assert_exit "$?" "0" "dismiss succeeds"
wait_until eval '(( $(event_count) >= 2 ))'
jq -se --arg id "ws:$NWIN" '
  .[1].v == 1 and .[1].id == $id and .[1].state == "closed" and .[1].prev == "waiting" and
  .[1].sound == "" and .[1].seq > .[0].seq and (.[1].actions | length) == 1
' "$FAKE_EVENT_LOG" >/dev/null 2>&1
assert_exit "$?" "0" "dismiss emits a newer closed event for the tab"
assert_eq "$(T display -p -t "$NWIN" '#{@ws_state}')" "waiting" "dismiss keeps the tab state"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" "$WS" _dismiss "$NWIN"
sleep 0.3
assert_eq "$(event_count)" "2" "second dismiss emits nothing"
KEEPALIVE_WIN="$(T display -p -t keepalive '#{window_id}')"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" "$WS" _dismiss "$KEEPALIVE_WIN"
assert_exit "$?" "0" "dismiss outside ws tabs is a no-op"
WS_NOTIFY=0 WS_NOTIFIER="$APPEND" hook api/feat-login "$NWIN" idle </dev/null
sleep 0.3
assert_eq "$(event_count)" "2" "no extra events from outside tabs or with WS_NOTIFY=0"

echo "ws dismiss: selecting the tab dismisses its notification"
NSESS="$(T display -p -t "$NWIN" '#{session_id}')"
OTHER_WIN="$(T new-window -d -P -F '#{window_id}' -t "$NSESS:")"
T select-window -t "$OTHER_WIN"
T set-environment -g WS_NOTIFY 1
T set-environment -g WS_NOTIFIER "$APPEND"
T set-environment -g FAKE_EVENT_LOG "$FAKE_EVENT_LOG"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_EVENT_LOG"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_EVENT_LOG"
T select-window -t "$NWIN"
wait_until eval 'jq -se "map(.state) == [\"waiting\", \"closed\"]" "$FAKE_EVENT_LOG" >/dev/null 2>&1'
assert_exit "$?" "0" "tab selection emits closed"
T kill-window -t "$OTHER_WIN"

echo "ws dismiss: agent exit dismisses its notification"
before="$(windows_of "$WT")"
"$WS" add api/feat-login --detach >/dev/null
XWIN="$(windows_of "$WT" | grep -vxF -- "$before" | head -1)"
wait_pane "$XWIN" FAKE-claude >/dev/null
rm -f "$FAKE_EVENT_LOG"
WS_NOTIFY=1 WS_NOTIFIER="$APPEND" hook api/feat-login "$XWIN" waiting </dev/null
wait_until test -s "$FAKE_EVENT_LOG"
T send-keys -t "$XWIN" Enter
wait_until eval 'jq -se --arg id "ws:$XWIN" "map(select(.id == \$id) | .state) == [\"waiting\", \"closed\"]" "$FAKE_EVENT_LOG" >/dev/null 2>&1'
assert_exit "$?" "0" "agent exit emits closed for its tab"
T set-environment -gu WS_NOTIFY
T set-environment -gu WS_NOTIFIER
T set-environment -gu FAKE_EVENT_LOG

echo "ws hook: failed terminal-notifier falls back to osascript"
cat > "$TMP/bin/terminal-notifier" <<'EOF'
#!/bin/sh
exit 1
EOF
WS_NOTIFY=1 WS_NOTIFIER=auto hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_contains "$(cat "$FAKE_OSASCRIPT_OUT")" "api/feat-login is idle" "rejected auto notification falls back with the same message"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/done.wav" "auto fallback plays the done cue exactly once"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"
WS_NOTIFY=1 WS_NOTIFIER=terminal-notifier hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_contains "$(cat "$FAKE_OSASCRIPT_OUT")" "api/feat-login needs input" "explicit terminal-notifier also falls back"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/waiting.wav" "explicit fallback plays the waiting cue exactly once"

echo "ws hook: boringnotch fallback also survives a rejected terminal notification"
hook api/feat-login "$NWIN" working </dev/null
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"
WS_NOTIFY=1 WS_NOTIFIER=boringnotch hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_contains "$(cat "$FAKE_OSASCRIPT_OUT" 2>/dev/null)" "api/feat-login needs input" "notch fallback reaches osascript when terminal-notifier fails"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/waiting.wav" "nested notch fallback plays its cue exactly once"

echo "ws hook: auto uses osascript when terminal-notifier is absent"
mkdir -p "$TMP/fallback-bin"
for cmd in tmux jq git date cat cut afplay; do
  ln -s "$(command -v "$cmd")" "$TMP/fallback-bin/$cmd"
done
ln -s "$TMP/bin/osascript" "$TMP/fallback-bin/osascript"
rm -f "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_LOG"
PATH="$TMP/fallback-bin" WS_NOTIFY=1 hook api/feat-login "$NWIN" idle </dev/null
wait_until test -s "$FAKE_OSASCRIPT_OUT"
assert_contains "$(cat "$FAKE_OSASCRIPT_OUT")" "api/feat-login is idle" "auto fallback works without any terminal-notifier executable"
assert_eq "$(afplay_log)" "$ROOT/assets/ws/sounds/done.wav" "missing-notifier fallback plays the done cue exactly once"

echo "ws hook: focused transitions reach custom adapters"
mkfifo "$TMP/control.in"
exec {control_fd}<>"$TMP/control.in"
T select-window -t "$NWIN"
T -C attach-session -t "$(session_of "$NWIN")" < "$TMP/control.in" > "$TMP/control.out" 2>&1 &
tab_focused() { [[ "$(T display -p -t "$NWIN" '#{&&:#{window_active},#{session_attached}}')" == 1 ]]; }
wait_until tab_focused
assert_exit "$?" "0" "test client focuses the notifying tab"
rm -f "$FAKE_EVENT_OUT"
WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" waiting </dev/null
wait_until test -s "$FAKE_EVENT_OUT"
jq -e '.state == "waiting" and .prev == "idle" and .focused == true' "$FAKE_EVENT_OUT" >/dev/null 2>&1
assert_exit "$?" "0" "focused waiting transition reaches the custom adapter"
for state in working idle; do
  rm -f "$FAKE_EVENT_OUT"
  WS_NOTIFY=1 WS_NOTIFIER="$TMP/custom adapters/capture" hook api/feat-login "$NWIN" "$state" </dev/null
  wait_until test -s "$FAKE_EVENT_OUT"
  jq -e --arg state "$state" '.state == $state and .focused == true' "$FAKE_EVENT_OUT" >/dev/null 2>&1
  assert_exit "$?" "0" "focused $state transition reaches the custom adapter"
done

echo "ws hook: boringnotch receives focused transitions with semantic sound keys (app plays them)"
for state in working waiting; do
  start_socket_listener "$NOTCH_SOCKET" "$NOTCH_EVENT"
  rm -f "$FAKE_AFPLAY_LOG" "$FAKE_NOTIFIER_OUT" "$FAKE_OSASCRIPT_OUT"
  WS_NOTIFY=1 WS_NOTIFIER=boringnotch WS_BORINGNOTCH_SOCKET="$NOTCH_SOCKET" \
    WS_NOTIFY_SOUND_WAITING="$TMP/custom waiting.wav" hook api/feat-login "$NWIN" "$state" </dev/null
  wait_until test -s "$NOTCH_EVENT"
  jq -e --arg state "$state" '.state == $state and .focused == true' "$NOTCH_EVENT" >/dev/null 2>&1
  assert_exit "$?" "0" "notch receives focused $state transition"
  jq -e --arg state "$state" '.sound == (if $state == "waiting" then "waiting" else "" end)' "$NOTCH_EVENT" >/dev/null 2>&1
  assert_exit "$?" "0" "focused $state event carries its semantic sound key"
  sleep 0.3
  assert_eq "$([[ -e "$FAKE_AFPLAY_LOG" ]] && print yes)" "" "ws plays no $state sound for the notch"
  assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" || -e "$FAKE_OSASCRIPT_OUT" ]] && print yes)" "" "focused notch delivery bypasses macOS notifications"
  stop_socket_listener
  rm -f "$NOTCH_SOCKET"
done
hook api/feat-login "$NWIN" idle </dev/null

echo "ws hook: macOS adapters and notch fallback suppress every focused transition"
for adapter in auto terminal-notifier osascript boringnotch; do
  rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_OUT"
  for state in waiting working idle; do
    WS_NOTIFY=1 WS_NOTIFIER="$adapter" hook api/feat-login "$NWIN" "$state" </dev/null
  done
  sleep 0.3
  assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" || -e "$FAKE_OSASCRIPT_OUT" || -e "$FAKE_AFPLAY_OUT" ]] && print yes)" "" \
    "$adapter suppresses notifications and sounds for focused idle, waiting and working"
done
T detach-client -s "$(session_of "$NWIN")"
exec {control_fd}>&-

echo "ws hook: macOS adapters and notch fallback suppress unfocused working transitions"
for adapter in auto terminal-notifier osascript boringnotch; do
  hook api/feat-login "$NWIN" idle </dev/null
  rm -f "$FAKE_NOTIFIER_OUT" "$FAKE_OSASCRIPT_OUT" "$FAKE_AFPLAY_OUT"
  WS_NOTIFY=1 WS_NOTIFIER="$adapter" hook api/feat-login "$NWIN" working </dev/null
  sleep 0.3
  assert_eq "$([[ -e "$FAKE_NOTIFIER_OUT" || -e "$FAKE_OSASCRIPT_OUT" || -e "$FAKE_AFPLAY_OUT" ]] && print yes)" "" \
    "$adapter suppresses notifications and sounds for unfocused working"
done

cat > "$TMP/bin/terminal-notifier" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_NOTIFIER_OUT"
EOF
hook api/feat-login "$NWIN" idle </dev/null

echo "ws _build-notifier: own-branded notifier app (name, bundle id, icon); used when present"
SRCAPP="$TMP/src/terminal-notifier.app"
mkdir -p "$SRCAPP/Contents/MacOS" "$SRCAPP/Contents/Resources"
cat > "$SRCAPP/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>fr.julienxx.oss.terminal-notifier</string>
<key>CFBundleName</key><string>terminal-notifier</string>
<key>CFBundleExecutable</key><string>terminal-notifier</string>
<key>CFBundleIconFile</key><string>Terminal</string>
</dict></plist>
EOF
cat > "$SRCAPP/Contents/MacOS/terminal-notifier" <<'EOF'
#!/bin/sh
{ echo "BRANDED"; printf '%s\n' "$@"; } > "$FAKE_NOTIFIER_OUT"
EOF
chmod +x "$SRCAPP/Contents/MacOS/terminal-notifier"
WS_NOTIFIER_SOURCE="$SRCAPP" "$WS" _build-notifier >/dev/null 2>&1
assert_exit "$?" "0" "builds the notifier app"
APP="$WS_HOME/ws.app"
assert_eq "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist" 2>/dev/null)" "sh.ratel.ws.notifier" "own bundle id"
assert_eq "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$APP/Contents/Info.plist" 2>/dev/null)" "ws" "named ws"
assert_eq "$([[ -s "$APP/Contents/Resources/Terminal.icns" ]] && file -b "$APP/Contents/Resources/Terminal.icns" | grep -c 'icon')" "1" "icon replaced with a real .icns"
assert_eq "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$SRCAPP/Contents/Info.plist")" "fr.julienxx.oss.terminal-notifier" "source app untouched"
rm -f "$FAKE_NOTIFIER_OUT"
WS_NOTIFY=1 hook api/feat-login "$(slot_window "$WT" 1)" waiting </dev/null
wait_until test -s "$FAKE_NOTIFIER_OUT"
assert_eq "$(head -1 "$FAKE_NOTIFIER_OUT" 2>/dev/null)" "BRANDED" "notifications sent through ws.app when built"
hook api/feat-login "$(slot_window "$WT" 1)" working </dev/null
rm -rf "$APP"
