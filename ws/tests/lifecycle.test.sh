#!/bin/zsh
# ws tests: `ws done`/`show`, exit events, exit path ownership and late/nested hooks.
# Not standalone: ws/tests/run.sh sources helpers.sh then the area files in order, in one process,
# because workspaces, tabs and global session ids carry over between areas.
(( $+functions[ws_tests_require_sandbox] )) && ws_tests_require_sandbox || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

echo "ws done: file output is durable; user-opened tab stays; show resolves names"
"$WS" new --repo "$REPO" --branch lifecycle --detach
DONE_WT="$WS_ROOT/api/lifecycle"
DONE_WIN="$(windows_of "$DONE_WT")"
assert_contains "$(wait_pane "$DONE_WIN" FAKE-claude)" "FAKE-claude" "lifecycle agent ready"
DONE_ID="$(T display -p -t "$DONE_WIN" '#{@ws_session}')"
WS_SESSION="$DONE_ID" "$WS" name finisher
printf '# Result\n\nquoted "value" $literal\n\n' > "$TMP/result file.md"
WS_SESSION="$DONE_ID" "$WS" done --output "$TMP/result file.md"
assert_exit "$?" "0" "done accepts output file"
cmp -s "$TMP/result file.md" "$WS_HOME/sessions/$DONE_ID/output.md"
assert_exit "$?" "0" "done preserves exact output bytes"
assert_eq "$("$WS" sessions "$DONE_ID" | cut -f6)" "done" "done records lifecycle status"
assert_eq "$(windows_of "$DONE_WT")" "$DONE_WIN" "user-opened tab stays after done"
assert_eq "$(jq -r '.event' "$WS_HOME/sessions/$DONE_ID/events")" "done" "done appends lifecycle event"
jq -e --arg id "$DONE_ID" --arg output "$WS_HOME/sessions/$DONE_ID/output.md" \
  '.id == $id and .output == $output and (.at | type == "string")' "$WS_HOME/sessions/$DONE_ID/events" >/dev/null
assert_exit "$?" "0" "done event identifies source, output and time"
out="$("$WS" show finisher)"
assert_exit "$?" "0" "show accepts session name"
assert_contains "$out" "status: done" "show reports lifecycle status"
assert_contains "$out" "output: $WS_HOME/sessions/$DONE_ID/output.md" "show reports durable output path"
echo "ws done: repeated declarations replace output, including stdin and empty default"
printf 'replacement\n\n' | WS_SESSION="$DONE_ID" "$WS" done -
assert_exit "$?" "0" "done accepts stdin shorthand"
assert_eq "$(cat "$WS_HOME/sessions/$DONE_ID/output.md"; print -n .)" $'replacement\n\n.' "repeat overwrites output preserving trailing newlines"
assert_eq "$(jq -sr 'map(.event) | join(",")' "$WS_HOME/sessions/$DONE_ID/events")" "done,done" "repeat appends another done event"
printf 'explicit stdin' | WS_SESSION="$DONE_ID" "$WS" done --output -
assert_exit "$?" "0" "done accepts explicit stdin output"
assert_eq "$(cat "$WS_HOME/sessions/$DONE_ID/output.md")" "explicit stdin" "explicit stdin replaces previous output"
WS_SESSION="$DONE_ID" "$WS" done
assert_exit "$?" "0" "done without output succeeds without reading stdin"
assert_eq "$(wc -c < "$WS_HOME/sessions/$DONE_ID/output.md" | tr -d ' ')" "0" "default done writes empty output"
assert_eq "$(jq -s length "$WS_HOME/sessions/$DONE_ID/events")" "4" "each declaration appends one event"

echo "ws done/show: invalid input leaves lifecycle intact"
for args in '--output' '--bad' '--output missing-file' 'extra argument'; do
  WS_SESSION="$DONE_ID" "$WS" done ${=args} 2>/dev/null
  assert_exit "$?" "1" "invalid done rejected: $args"
done
env -u WS_SESSION "$WS" done 2>/dev/null
assert_exit "$?" "1" "done outside a session is refused"
WS_SESSION=999999 "$WS" done 2>/dev/null
assert_exit "$?" "1" "done with unknown identity is refused"
assert_eq "$(jq -s length "$WS_HOME/sessions/$DONE_ID/events")" "4" "invalid declarations append no events"
assert_eq "$(wc -c < "$WS_HOME/sessions/$DONE_ID/output.md" | tr -d ' ')" "0" "invalid declarations preserve output"
"$WS" show no-such-session 2>/dev/null
assert_exit "$?" "1" "show rejects missing session"
"$WS" show 2>/dev/null
assert_exit "$?" "1" "show requires target"
out="$("$WS" send "$DONE_ID" should-refuse 2>&1)"
assert_exit "$?" "1" "send rejects session declared done through CLI"
assert_contains "$out" "--force required" "send explains done guard"

echo "ws lifecycle: agent exit emits its actual code and preserves done output"
lifecycle_exit_count() { [[ "$(jq -sr 'map(select(.event == "exit")) | length' "$WS_HOME/sessions/$1/events")" == "$2" ]]; }
lifecycle_done_stopped() { [[ "$("$WS" sessions "$1" | cut -f5,6)" == $'stopped\tdone' ]]; }
T send-keys -t "$DONE_WIN" Enter
wait_until lifecycle_exit_count "$DONE_ID" 1
assert_exit "$?" "0" "normal agent exit appends lifecycle event"
assert_eq "$(jq -sr 'map(select(.event == "exit")) | .[0].code' "$WS_HOME/sessions/$DONE_ID/events")" "0" "normal exit event records code zero"
# Exit publication precedes the launch wrapper's _end cleanup; wait for both transitions.
wait_until lifecycle_done_stopped "$DONE_ID"
assert_exit "$?" "0" "normal exit settles stopped with done lifecycle"
assert_eq "$("$WS" sessions "$DONE_ID" | cut -f5,6)" $'stopped\tdone' "agent exit preserves done lifecycle"
assert_contains "$("$WS" show "$DONE_ID")" "output: $WS_HOME/sessions/$DONE_ID/output.md" "output stays addressable after agent exit"
"$WS" open "$DONE_ID" --detach
DONE_WIN="$(windows_of "$DONE_WT")"
assert_contains "$(wait_pane "$DONE_WIN" FAKE-claude)" "FAKE-claude" "done identity can resume"
T send-keys -t "$DONE_WIN" 17 Enter
wait_until lifecycle_exit_count "$DONE_ID" 2
assert_exit "$?" "0" "failed agent exit appends lifecycle event"
assert_eq "$(jq -sr 'map(select(.event == "exit")) | .[1].code' "$WS_HOME/sessions/$DONE_ID/events")" "17" "failed exit retains actual nonzero code"
assert_contains "$(wait_pane "$DONE_WIN" 'agent exited (17)')" "agent exited (17)" "failure keeps existing close/resume behavior"
assert_eq "$(T display -p -t "$DONE_WIN" '#{@ws_session}')" "" "crashed tab no longer resolves as the live session"
T send-keys -t "$DONE_WIN" Enter

# A shell tab adopted with ws claude must report exits as well as ws new/add tabs.
MANUAL_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$DONE_WT" \
  "zsh -fc '\"$WS\" claude; echo EXIT=\$?; sleep 30'")"
assert_contains "$(wait_pane "$MANUAL_WIN" FAKE-claude)" "FAKE-claude" "manually opened agent ready"
MANUAL_ID="$(T display -p -t "$MANUAL_WIN" '#{@ws_session}')"
T send-keys -t "$MANUAL_WIN" Enter
assert_contains "$(wait_pane "$MANUAL_WIN" EXIT=0)" "EXIT=0" "adopted tab returns to shell"
assert_eq "$(jq -r '.event, .code' "$WS_HOME/sessions/$MANUAL_ID/events")" $'exit\n0' "adopted tab also emits actual process exit"
T kill-window -t "$MANUAL_WIN"

echo "ws lifecycle: closing a manually adopted tab reports SIGHUP too"
MANUAL_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$DONE_WT" \
  "zsh -fc '\"$WS\" claude; sleep 30'")"
assert_contains "$(wait_pane "$MANUAL_WIN" FAKE-claude)" "FAKE-claude" "adopted closure fixture ready"
MANUAL_ID="$(T display -p -t "$MANUAL_WIN" '#{@ws_session}')"
T kill-window -t "$MANUAL_WIN"
wait_until lifecycle_exit_count "$MANUAL_ID" 1
assert_exit "$?" "0" "closing adopted tab emits termination"
jq -se 'length == 1 and .[0].event == "exit" and (.[0].code == 0 or .[0].code == 129)' "$WS_HOME/sessions/$MANUAL_ID/events" >/dev/null
assert_exit "$?" "0" "adopted closure records actual EOF or SIGHUP exit once"

echo "ws done: spawned agent closes its own tab after publishing output"
"$WS" add api/lifecycle --detach
SPAWN_WIN="$(windows_of "$DONE_WT")"
assert_contains "$(wait_pane "$SPAWN_WIN" FAKE-claude)" "FAKE-claude" "spawn fixture ready"
SPAWN_ID="$(T display -p -t "$SPAWN_WIN" '#{@ws_session}')"
META="$WS_HOME/sessions/$SPAWN_ID/meta"
jq '.spawned = true | .resume_id = "spawn-resume"' "$META" > "$META.fixture"
mv "$META.fixture" "$META"
T send-keys -t "$SPAWN_WIN" done Enter
spawn_done_closed() { [[ "$("$WS" sessions "$1" | cut -f5,6)" == $'stopped\tdone' ]]; }
wait_until spawn_done_closed "$SPAWN_ID"
assert_exit "$?" "0" "spawned tab closes when its agent declares done"
assert_eq "$(awk -F '\t' -v id="$SPAWN_ID" '$4 == id' "$(git -C "$DONE_WT" rev-parse --absolute-git-dir)/ws-sessions")" "" "spawned done frees its workspace slot row"
assert_eq "$(cat "$WS_HOME/sessions/$SPAWN_ID/output.md")" "agent result" "spawned output is durable before tab closes"
assert_eq "$(jq -sr 'map(select(.event == "done")) | length' "$WS_HOME/sessions/$SPAWN_ID/events")" "1" "spawned completion appends one done event"
assert_eq "$("$WS" sessions "$SPAWN_ID" | cut -f6)" "done" "closing spawned tab preserves done status"
assert_contains "$("$WS" show "$SPAWN_ID")" "output: $WS_HOME/sessions/$SPAWN_ID/output.md" "closed spawned result is addressable"
# G2: done's self-kill is cleanup, not a crash: no exit event (would fire `ws on X exit`).
agent_gone() { ! pgrep -f "_agent $1 " >/dev/null; }
wait_until agent_gone "$SPAWN_ID"
assert_exit "$?" "0" "spawned agent wrapper finishes after done closure"
assert_eq "$(jq -sr 'map(.event) | join(",")' "$WS_HOME/sessions/$SPAWN_ID/events")" "done" "spawned done closure records one done and no exit"
"$WS" open "$SPAWN_ID" --detach
SPAWN_WIN="$(windows_of "$DONE_WT")"
assert_contains "$(wait_pane "$SPAWN_WIN" FAKE-claude)" "--resume spawn-resume" "closed spawned identity retains agent resume data"
T send-keys -t "$SPAWN_WIN" Enter
wait_until lifecycle_exit_count "$SPAWN_ID" 1
assert_exit "$?" "0" "resumed spawned session still reports its own exit"
assert_eq "$(jq -sr 'map(.event) | join(",")' "$WS_HOME/sessions/$SPAWN_ID/events")" "done,exit" "resumed exit follows the earlier done"

echo "ws lifecycle: a crashed spawned session reports one nonzero exit and no done"
"$WS" add api/lifecycle --detach
CRASH_WIN="$(windows_of "$DONE_WT")"
assert_contains "$(wait_pane "$CRASH_WIN" FAKE-claude)" "FAKE-claude" "spawned crash fixture ready"
CRASH_ID="$(T display -p -t "$CRASH_WIN" '#{@ws_session}')"
META="$WS_HOME/sessions/$CRASH_ID/meta"
jq '.spawned = true' "$META" > "$META.fixture"
mv "$META.fixture" "$META"
T send-keys -t "$CRASH_WIN" 17 Enter
wait_until lifecycle_exit_count "$CRASH_ID" 1
assert_exit "$?" "0" "spawned crash emits exit"
assert_contains "$(wait_pane "$CRASH_WIN" 'agent exited (17)')" "agent exited (17)" "spawned crash keeps close prompt"
assert_eq "$(jq -sr 'map("\(.event):\(.code // "")") | join(",")' "$WS_HOME/sessions/$CRASH_ID/events")" "exit:17" "spawned crash records single nonzero exit, no done"
T send-keys -t "$CRASH_WIN" Enter

echo "ws lifecycle: tab closure reports exit while a TUI still holds the tty"
"$WS" new --repo "$REPO" --branch delivery --detach
HUP_WT="$WS_ROOT/api/delivery"
HUP_WIN="$(windows_of "$HUP_WT")"
assert_contains "$(wait_pane "$HUP_WIN" PASTE-READY)" "PASTE-READY" "closure TUI ready"
HUP_ID="$(T display -p -t "$HUP_WIN" '#{@ws_session}')"
HUP_PID="$(T display -p -t "$HUP_WIN" '#{pane_pid}')"
# Retain only this private fixture's direct children for cleanup if the exit contract fails.
HUP_CHILDREN=( ${(f)"$(ps -Ao pid,ppid | awk -v p="$HUP_PID" '$2 == p {print $1}')"} )
T kill-window -t "$HUP_WIN"
wait_until lifecycle_exit_count "$HUP_ID" 1
assert_exit "$?" "0" "TUI closure emits an exit without waiting for terminal reads"
assert_eq "$(jq -r '.event, .code' "$WS_HOME/sessions/$HUP_ID/events")" $'exit\n129' "TUI closure records SIGHUP once"
kill -KILL "$HUP_PID" "${HUP_CHILDREN[@]}" 2>/dev/null
"$WS" rm api/delivery --force --delete-branch >/dev/null

echo "ws lifecycle: interactive launch shells keep the crash prompt and actual exit code"
for agent in claude codex grok; do
  WS_AGENT_SHELL="zsh -fic" "$WS" new --repo "$REPO" --branch "interactive-$agent" --agent "$agent" --detach
  INTERACTIVE_WIN="$(windows_of "$WS_ROOT/api/interactive-$agent")"
  assert_contains "$(wait_pane "$INTERACTIVE_WIN" "FAKE-$agent")" "FAKE-$agent" "$agent interactive shell starts TUI"
  INTERACTIVE_ID="$(T display -p -t "$INTERACTIVE_WIN" '#{@ws_session}')"
  T send-keys -t "$INTERACTIVE_WIN" 17 Enter
  assert_contains "$(wait_pane "$INTERACTIVE_WIN" 'agent exited (17)')" 'agent exited (17)' "$agent interactive shell keeps crash prompt"
  assert_eq "$(jq -r '.event, .code' "$WS_HOME/sessions/$INTERACTIVE_ID/events")" $'exit\n17' "$agent interactive shell records actual exit"
  "$WS" rm "api/interactive-$agent" --force --delete-branch >/dev/null
done

# --- exit path ---
echo "ws exit path: every exit stops delivery before slow exit actions run"
"$WS" new --repo "$REPO" --branch exit-path --detach
EXIT_WT="$WS_ROOT/api/exit-path"
"$WS" new --repo "$REPO" --branch delivery --detach
EXIT_TARGET_WIN="$(windows_of "$WS_ROOT/api/delivery")"
assert_contains "$(wait_pane "$EXIT_TARGET_WIN" PASTE-READY)" "PASTE-READY" "exit action target ready"
EXIT_TARGET_ID="$(T display -p -t "$EXIT_TARGET_WIN" '#{@ws_session}')"
for exit_code in 0 17; do
  [[ "$exit_code" == 0 ]] || "$WS" add api/exit-path --detach
  EXIT_WIN="$(windows_of "$EXIT_WT")"
  assert_contains "$(wait_pane "$EXIT_WIN" FAKE-claude)" "FAKE-claude" "exit $exit_code source ready"
  EXIT_ID="$(T display -p -t "$EXIT_WIN" '#{@ws_session}')"
  hook api/exit-path "$EXIT_WIN" stop </dev/null >/dev/null
  T kill-window -t "$EXIT_TARGET_WIN"
  wait_until lifecycle_exit_count "$EXIT_TARGET_ID" "$(( exit_code == 0 ? 1 : 2 ))"
  EXIT_TID="$("$WS" on "$EXIT_ID" exit --send-to "$EXIT_TARGET_ID" "exit action $exit_code")"
  # Keep the exit action waiting for its resumed TUI while a concurrent send runs.
  print 3 > "$WS_HOME/tui-delay"
  # Resume is deliberately unavailable: send must retain the message, never paste into
  # the dead source. This private fixture releases the lock immediately after send.
  EXIT_OPEN_LOCK="$WS_HOME/sessions/$EXIT_ID/open.lock"
  : >> "$EXIT_OPEN_LOCK"
  zmodload zsh/system
  zsystem flock -f exit_open_fd "$EXIT_OPEN_LOCK"
  T send-keys -t "$EXIT_WIN" "$exit_code" Enter
  wait_until lifecycle_exit_count "$EXIT_ID" 1
  assert_exit "$?" "0" "exit $exit_code publishes while its action is still waiting"
  assert_eq "$(T show -wqv -t "$EXIT_WIN" @ws_session)" "" "exit $exit_code source is no longer deliverable"
  assert_eq "$(T show -wqv -t "$EXIT_WIN" @ws_main)" "" "exit $exit_code clears its idle delivery tag"
  # WS_LOCK_SCALE: send gives up on the held open lock after 0.3s, well within the 3s TUI delay.
  out="$(WS_LOCK_SCALE=0.1 "$WS" send "$EXIT_ID" "during exit $exit_code" 2>&1)"
  assert_exit "$?" "75" "exit $exit_code send reports unavailable resume"
  assert_contains "$(cat "$WS_HOME/sessions/$EXIT_ID/inbox")" "during exit $exit_code" "exit $exit_code send stays durable"
  exec {exit_open_fd}>&-
  wait_until pane_received "$EXIT_TARGET_ID" "[ws terminal]"$'\n'"exit action $exit_code"
  assert_exit "$?" "0" "exit $exit_code action finishes after delivery guard"
  EXIT_TARGET_WIN="$(jq -r '.window' "$WS_HOME/sessions/$EXIT_TARGET_ID/meta")"
  if [[ "$exit_code" == 17 ]]; then
    assert_contains "$(wait_pane "$EXIT_WIN" 'agent exited (17)')" 'agent exited (17)' "exit guard preserves crash prompt"
    T send-keys -t "$EXIT_WIN" Enter
  fi
done
echo "ws exit path: an old wrapper cannot untag a resumed or reassigned window"
"$WS" add api/exit-path --detach
EXIT_OLD_WIN="$(windows_of "$EXIT_WT")"
assert_contains "$(wait_pane "$EXIT_OLD_WIN" FAKE-claude)" "FAKE-claude" "old generation ready"
EXIT_ID="$(T display -p -t "$EXIT_OLD_WIN" '#{@ws_session}')"
T set -wu -t "$EXIT_OLD_WIN" @ws_session
"$WS" open "$EXIT_ID" --detach
EXIT_NEW_WIN="$(jq -r '.window' "$WS_HOME/sessions/$EXIT_ID/meta")"
assert_contains "$(wait_pane "$EXIT_NEW_WIN" FAKE-claude)" "FAKE-claude" "new generation ready"
hook api/exit-path "$EXIT_NEW_WIN" stop </dev/null >/dev/null
# Reassign the old tab's delivery tags too: this wrapper no longer owns either tag.
T set -w -t "$EXIT_OLD_WIN" @ws_session "$EXIT_TARGET_ID" \; set -w -t "$EXIT_OLD_WIN" @ws_main working
T send-keys -t "$EXIT_OLD_WIN" 17 Enter
wait_until lifecycle_exit_count "$EXIT_ID" 1
assert_exit "$?" "0" "old generation exit still publishes"
assert_eq "$(T show -wqv -t "$EXIT_NEW_WIN" @ws_session)" "$EXIT_ID" "old exit preserves resumed session tag"
assert_eq "$(T show -wqv -t "$EXIT_NEW_WIN" @ws_main)" idle "old exit preserves resumed delivery state"
assert_eq "$(T show -wqv -t "$EXIT_OLD_WIN" @ws_session)" "$EXIT_TARGET_ID" "old exit preserves reassigned session tag"
assert_eq "$(T show -wqv -t "$EXIT_OLD_WIN" @ws_main)" working "old exit preserves reassigned delivery state"
T kill-window -t "$EXIT_OLD_WIN"

echo "ws exit path: old wrapper cleanup spares a session resumed in a new window"
"$WS" new --repo "$REPO" --branch exit-gen --detach
EXIT_GEN_WT="$WS_ROOT/api/exit-gen"
EXIT_GEN_WIN="$(windows_of "$EXIT_GEN_WT")"
EXIT_GEN_ID="$(T display -p -t "$EXIT_GEN_WIN" '#{@ws_session}')"
EXIT_GEN_SLOT="$(T display -p -t "$EXIT_GEN_WIN" '#{@ws_slot}')"
EXIT_GEN_META="$WS_HOME/sessions/$EXIT_GEN_ID/meta"
EXIT_GEN_ROWS="$(git -C "$EXIT_GEN_WT" rev-parse --absolute-git-dir)/ws-sessions"
# Simulate `ws open` re-pointing the ID to a new window while the old wrapper waits.
jq '.window = "@resumed"' "$EXIT_GEN_META" > "$EXIT_GEN_META.tmp" && mv "$EXIT_GEN_META.tmp" "$EXIT_GEN_META"
"$WS" _end "$EXIT_GEN_WT" "$EXIT_GEN_SLOT" "$EXIT_GEN_ID" "$EXIT_GEN_WIN"
assert_eq "$(jq -r '.window, .status' "$EXIT_GEN_META")" $'@resumed\nactive' "stale cleanup keeps resumed meta"
assert_eq "$(awk -F '\t' -v s="$EXIT_GEN_SLOT" '$1 == s {print $4}' "$EXIT_GEN_ROWS")" "$EXIT_GEN_ID" "stale cleanup keeps resumed slot row"
"$WS" _end "$EXIT_GEN_WT" "$EXIT_GEN_SLOT" "$EXIT_GEN_ID" "@resumed"
assert_eq "$(jq -r '.window, .status' "$EXIT_GEN_META")" $'\nclosed' "matching-generation cleanup clears meta"
assert_eq "$(awk -F '\t' -v s="$EXIT_GEN_SLOT" '$1 == s' "$EXIT_GEN_ROWS")" "" "matching-generation cleanup removes slot row"
"$WS" rm api/exit-gen --force --delete-branch >/dev/null

# --- G1: generation/ownership gates for adopted tabs and late or nested hooks ---
echo "ws claude: adopted-tab clean exit spares a session resumed in a new window"
"$WS" new --repo "$REPO" --branch run-gen --detach
RUN_GEN_WT="$WS_ROOT/api/run-gen"
RUN_GEN_ROWS="$(git -C "$RUN_GEN_WT" rev-parse --absolute-git-dir)/ws-sessions"
RUN_GEN_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$RUN_GEN_WT" \
  "zsh -fc '\"$WS\" claude; echo EXIT=\$?; sleep 30'")"
assert_contains "$(wait_pane "$RUN_GEN_WIN" FAKE-claude)" "FAKE-claude" "adopted generation ready"
RUN_GEN_ID="$(T show -wqv -t "$RUN_GEN_WIN" @ws_session)"
RUN_GEN_SLOT="$(T show -wqv -t "$RUN_GEN_WIN" @ws_slot)"
RUN_GEN_META="$WS_HOME/sessions/$RUN_GEN_ID/meta"
jq '.window = "@resumed"' "$RUN_GEN_META" > "$RUN_GEN_META.tmp" && mv "$RUN_GEN_META.tmp" "$RUN_GEN_META"
T send-keys -t "$RUN_GEN_WIN" Enter
assert_contains "$(wait_pane "$RUN_GEN_WIN" EXIT=0)" "EXIT=0" "adopted old generation exits cleanly"
assert_eq "$(jq -r '.window, .status' "$RUN_GEN_META")" $'@resumed\nactive' "adopted exit keeps resumed meta"
assert_eq "$(awk -F '\t' -v s="$RUN_GEN_SLOT" '$1 == s {print $4}' "$RUN_GEN_ROWS")" "$RUN_GEN_ID" "adopted exit keeps resumed slot row"
T kill-window -t "$RUN_GEN_WIN"

echo "ws hook: a late Stop after exit cleanup never drains the (resumed) global inbox"
RUN_GEN_WIN="$(windows_of "$RUN_GEN_WT" | head -1)"
assert_contains "$(wait_pane "$RUN_GEN_WIN" FAKE-claude)" "FAKE-claude" "late-stop fixture ready"
LATE_ID="$(T show -wqv -t "$RUN_GEN_WIN" @ws_session)"
LATE_META="$WS_HOME/sessions/$LATE_ID/meta"
T set -wu -t "$RUN_GEN_WIN" @ws_session # exit cleanup untagged the old pane
print -r -- '{"message":"for the resumed pane"}' >> "$WS_HOME/sessions/$LATE_ID/inbox"
for late_win in "@resumed" ""; do
  jq --arg w "$late_win" '.window = $w' "$LATE_META" > "$LATE_META.tmp" && mv "$LATE_META.tmp" "$LATE_META"
  reply="$(print '{}' | WS_SESSION="$LATE_ID" hook api/run-gen "$RUN_GEN_WIN" stop)"
  assert_eq "$reply" "{}" "late Stop (meta.window='$late_win') answers {}"
  assert_contains "$(cat "$WS_HOME/sessions/$LATE_ID/inbox")" "for the resumed pane" "late Stop (meta.window='$late_win') leaves inbox"
done
jq --arg w "$RUN_GEN_WIN" '.window = $w' "$LATE_META" > "$LATE_META.tmp" && mv "$LATE_META.tmp" "$LATE_META"
T set -w -t "$RUN_GEN_WIN" @ws_session "$LATE_ID"
: > "$WS_HOME/sessions/$LATE_ID/inbox"

echo "ws hook: a nested agent with inherited tab env cannot drain the parent's inbox"
mkdir -p "$TMP/nest"
cat > "$TMP/nest/claude" <<EOF
#!/bin/sh
while read line; do
  case "\$line" in
  nested) sh -c '"$TMP/nest/grok"; :' ;; # tool shell runs a headless agent
  direct) printf '{}' | "\$WS_TEST_CLI" hook stop > "\$WS_HOME/nest-direct" ;;
  esac
done
EOF
cat > "$TMP/nest/grok" <<'EOF'
#!/bin/sh
printf '{}' | "$WS_TEST_CLI" hook stop > "$WS_HOME/nest-nested"
EOF
chmod +x "$TMP/nest/claude" "$TMP/nest/grok"
NEST_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$RUN_GEN_WT" \
  -e "WS_WORKSPACE=api/run-gen" -e "WS_SESSION=$LATE_ID" -e "WS_HOME=$WS_HOME" -e "WS_TEST_CLI=$WS" "$TMP/nest/claude")"
T set -w -t "$NEST_WIN" @ws_session "$LATE_ID" \; set -w -t "$NEST_WIN" @ws_path "$RUN_GEN_WT" \; set -w -t "$NEST_WIN" @ws_slot 1
jq --arg w "$NEST_WIN" '.window = $w' "$LATE_META" > "$LATE_META.tmp" && mv "$LATE_META.tmp" "$LATE_META"
print -r -- '{"message":"for the parent agent"}' >> "$WS_HOME/sessions/$LATE_ID/inbox"
T send-keys -t "$NEST_WIN" nested Enter
wait_until test -s "$WS_HOME/nest-nested"
assert_eq "$(cat "$WS_HOME/nest-nested")" "{}" "nested agent Stop answers {}"
assert_contains "$(cat "$WS_HOME/sessions/$LATE_ID/inbox")" "for the parent agent" "nested agent Stop leaves parent inbox"
T send-keys -t "$NEST_WIN" direct Enter
wait_until test -s "$WS_HOME/nest-direct"
assert_eq "$(jq -r '.reason' "$WS_HOME/nest-direct" 2>/dev/null)" "for the parent agent" "tab agent's own Stop still drains"
T kill-window -t "$NEST_WIN"
"$WS" rm api/run-gen --force --delete-branch >/dev/null

echo "ws exit path: HUP stops delivery even when the pane remains open"
"$WS" new --repo "$REPO" --branch exit-hup --detach
EXIT_HUP_WIN="$(windows_of "$WS_ROOT/api/exit-hup")"
assert_contains "$(wait_pane "$EXIT_HUP_WIN" FAKE-claude)" "FAKE-claude" "HUP source ready"
EXIT_HUP_ID="$(T display -p -t "$EXIT_HUP_WIN" '#{@ws_session}')"
hook api/exit-hup "$EXIT_HUP_WIN" stop </dev/null >/dev/null
EXIT_PANE_PID="$(T display -p -t "$EXIT_HUP_WIN" '#{pane_pid}')"
EXIT_WRAPPER_PID="$(ps -Ao pid,ppid,command | awk -v p="$EXIT_PANE_PID" -v id="$EXIT_HUP_ID" '$2 == p && index($0, "_agent " id " ") {print $1}')"
[[ "$EXIT_WRAPPER_PID" != <-> ]] || kill -HUP "$EXIT_WRAPPER_PID"
wait_until lifecycle_exit_count "$EXIT_HUP_ID" 1
assert_exit "$?" "0" "HUP publishes its lifecycle event"
assert_eq "$(jq -r '.code' "$WS_HOME/sessions/$EXIT_HUP_ID/events")" 129 "HUP records termination code"
assert_eq "$(T show -wqv -t "$EXIT_HUP_WIN" @ws_session)" "" "HUP clears session delivery tag"
assert_eq "$(T show -wqv -t "$EXIT_HUP_WIN" @ws_main)" "" "HUP clears idle delivery tag"
"$WS" rm api/exit-hup --force --delete-branch >/dev/null
"$WS" rm api/exit-path --force --delete-branch >/dev/null
"$WS" rm api/delivery --force --delete-branch >/dev/null

echo "ws exit path: agent children inherit default SIGINT and SIGQUIT"
# Replace only this private fixture's executable; running agents keep the old inode.
mv "$TMP/bin/claude" "$TMP/bin/claude-exit-normal"
cat > "$TMP/bin/claude" <<'SIGNAL_AGENT'
#!/usr/bin/env python3
import os, signal
print("SIGNAL-READY", flush=True)
os.kill(os.getpid(), getattr(signal, "SIG" + input().strip()))
print("SIGNAL-SURVIVED", flush=True)
SIGNAL_AGENT
chmod +x "$TMP/bin/claude"
for signal in INT QUIT; do
  signal_code=130
  [[ "$signal" != QUIT ]] || signal_code=131
  signal_branch="exit-signal-${(L)signal}"
  "$WS" new --repo "$REPO" --branch "$signal_branch" --detach
  SIGNAL_WIN="$(windows_of "$WS_ROOT/api/$signal_branch")"
  assert_contains "$(wait_pane "$SIGNAL_WIN" SIGNAL-READY)" SIGNAL-READY "$signal agent ready"
  SIGNAL_ID="$(T display -p -t "$SIGNAL_WIN" '#{@ws_session}')"
  T send-keys -t "$SIGNAL_WIN" "$signal" Enter
  wait_until lifecycle_exit_count "$SIGNAL_ID" 1
  assert_exit "$?" "0" "$signal agent termination publishes once"
  assert_eq "$(jq -r '.code' "$WS_HOME/sessions/$SIGNAL_ID/events")" "$signal_code" "$signal retains its default termination disposition"
  "$WS" rm "api/$signal_branch" --force --delete-branch >/dev/null
done
mv "$TMP/bin/claude-exit-normal" "$TMP/bin/claude"
# --- end exit path ---

# --- M1 done vs pending inbox ---
echo "ws done: a spawned session keeps accepted messages deliverable until Stop"
M1_TID="$(WS_SESSION="$DONE_ID" "$WS" on "$DONE_ID" done --spawn 'Finish after checking follow-ups')"
assert_exit "$?" "0" "M1 spawns a tracked child through a done trigger"
M1_ID="$(jq -r '.spawned_session' "$WS_HOME/triggers/archive/$M1_TID.json")"
M1_WIN="$(jq -r '.window' "$WS_HOME/sessions/$M1_ID/meta")"
assert_contains "$(wait_pane "$M1_WIN" FAKE-claude)" "FAKE-claude" "M1 spawned agent ready"
hook api/lifecycle "$M1_WIN" working </dev/null
WS_SESSION="$DONE_ID" "$WS" send "$M1_ID" 'Check this follow-up before finishing'
assert_exit "$?" "0" "M1 working child accepts follow-up"
out="$(printf 'premature result\n' | WS_SESSION="$M1_ID" "$WS" done - 2>&1)"
assert_exit "$?" "1" "M1 done refuses accepted but undelivered work"
assert_contains "$out" "pending messages" "M1 refusal explains why completion must wait"
assert_contains "$out" "Stop hook" "M1 refusal tells the agent how to receive pending work"
assert_contains "$("$WS" show "$M1_ID")" "status: active" "M1 refused done preserves active lifecycle"
assert_eq "$(T display -p -t "$M1_WIN" '#{@ws_session}' 2>/dev/null)" "$M1_ID" "M1 refused done keeps spawned tab alive"
assert_eq "$(jq -s length "$WS_HOME/sessions/$M1_ID/events")" "0" "M1 refused done publishes no lifecycle event"
assert_eq "$([[ -e "$WS_HOME/sessions/$M1_ID/output.md" ]] && print yes)" "" "M1 refused done publishes no premature output"
reply="$(hook api/lifecycle "$M1_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$DONE_ID]"$'\nCheck this follow-up before finishing' "M1 Stop delivers the accepted follow-up"
echo "ws done: an interrupted Stop reply still prevents spawned completion"
WS_SESSION="$DONE_ID" "$WS" send "$M1_ID" 'Recover this interrupted reply'
assert_exit "$?" "0" "M1 child accepts work before an interrupted Stop"
hook api/lifecycle "$M1_WIN" stop </dev/null 2>/dev/null | true # close stdout before the reply lands
out="$(WS_SESSION="$M1_ID" "$WS" done 2>&1)"
assert_exit "$?" "1" "M1 done refuses an unacked in-flight reply even with an empty inbox"
assert_contains "$out" "pending messages" "M1 unacked reply explains refusal"
assert_eq "$(T display -p -t "$M1_WIN" '#{@ws_session}' 2>/dev/null)" "$M1_ID" "M1 unacked reply keeps spawned tab alive"
reply="$(hook api/lifecycle "$M1_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$DONE_ID]"$'\nRecover this interrupted reply' "M1 Stop redelivers the unacked follow-up"
T send-keys -t "$M1_WIN" done Enter
wait_until spawn_done_closed "$M1_ID"
assert_exit "$?" "0" "M1 spawned child completes and closes after delivery"
assert_eq "$(cat "$WS_HOME/sessions/$M1_ID/output.md")" "agent result" "M1 successful completion publishes final output"
# --- end M1 done vs pending inbox ---

"$WS" rm api/lifecycle --force --delete-branch >/dev/null
