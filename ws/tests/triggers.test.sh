#!/bin/zsh
# ws tests: `ws on` fan-in basics.
# Not standalone: ws/tests/run.sh sources helpers.sh then the area files in order, in one process,
# because workspaces, tabs and global session ids carry over between areas.
[[ -n "${WS_TESTS_SUITE:-}" ]] || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

echo "ws on: fan-in waits for all sources and expands outputs in source order"
"$WS" new --repo "$REPO" --branch triggers --detach
TRIGGER_WT="$WS_ROOT/api/triggers"
SOURCE_A_WIN="$(windows_of "$TRIGGER_WT")"
SOURCE_A="$(T display -p -t "$SOURCE_A_WIN" '#{@ws_session}')"
"$WS" add api/triggers --detach
SOURCE_B_WIN="$(slot_window "$TRIGGER_WT" 2)"
SOURCE_B="$(T display -p -t "$SOURCE_B_WIN" '#{@ws_session}')"
"$WS" add api/triggers --detach
TARGET_WIN="$(slot_window "$TRIGGER_WT" 3)"
TARGET_ID="$(T display -p -t "$TARGET_WIN" '#{@ws_session}')"
assert_contains "$(wait_pane "$TARGET_WIN" FAKE-claude)" "FAKE-claude" "trigger target ready"
hook api/triggers "$TARGET_WIN" working </dev/null
TID="$(WS_SESSION="$TARGET_ID" "$WS" on "$SOURCE_A,$SOURCE_B" done --send-to me 'results {id}: {output}')"
assert_exit "$?" "0" "fan-in trigger registers"
assert_contains "$("$WS" triggers)" "$TID"$'\t'"$SOURCE_A,$SOURCE_B"$'\tdone\tsend' "pending trigger is listed"
printf 'alpha "quoted" $literal' | WS_SESSION="$SOURCE_A" "$WS" done -
assert_exit "$?" "0" "first source completion publishes"
reply="$(hook api/triggers "$TARGET_WIN" stop </dev/null)"
assert_eq "$reply" "{}" "first source alone does not deliver"
hook api/triggers "$TARGET_WIN" working </dev/null
printf beta | WS_SESSION="$SOURCE_B" "$WS" done -
assert_exit "$?" "0" "last source completion publishes and fires"
reply="$(hook api/triggers "$TARGET_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$TARGET_ID]"$'\n'"results $SOURCE_A,$SOURCE_B: alpha \"quoted\" \$literal"$'\n'"beta" "fan-in expands literal output text and ordered ids"
assert_eq "$("$WS" triggers)" "" "fired trigger is no longer pending"
echo "ws on: already-satisfied registration fires once, even when events repeat concurrently"
hook api/triggers "$TARGET_WIN" working </dev/null
SATISFIED_TID="$(WS_SESSION="$TARGET_ID" "$WS" on "$SOURCE_A,$SOURCE_B" done --send-to me 'already {id}')"
assert_exit "$?" "0" "already-done fan-in registers and fires immediately"
repeat_pids=()
for source_id in "$SOURCE_A" "$SOURCE_B"; do
  WS_SESSION="$source_id" "$WS" done > "$TMP/repeat-$source_id.log" 2>&1 &
  repeat_pids+=($!)
done
for pid in $repeat_pids; do wait "$pid"; assert_exit "$?" "0" "concurrent repeat completion succeeds"; done
reply="$(hook api/triggers "$TARGET_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$TARGET_ID]"$'\n'"already $SOURCE_A,$SOURCE_B" "already-satisfied trigger delivers exactly once"
assert_eq "$(hook api/triggers "$TARGET_WIN" stop </dev/null)" "{}" "repeat completion never queues another action"

echo "ws on: send triggers reach done targets; registration is the explicit override"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
WS_SESSION="$TARGET_ID" "$WS" on "$SOURCE_B" done --send-to "$SOURCE_A" 'to done {id}' >/dev/null
assert_exit "$?" "0" "trigger to done target fires"
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$TARGET_ID]"$'\n'"to done $SOURCE_B" "done target receives trigger send"
