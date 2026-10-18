#!/bin/zsh
# ws tests: Trigger edge records, batching, tolerance and bus hardening.
# Not standalone: ws/tests/run.sh sources helpers.sh then its group's area files in order, in one
# process, because workspaces, tabs and global session ids carry over within a group.
(( $+functions[ws_tests_require_sandbox] )) && ws_tests_require_sandbox || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

# --- trigger edges ---
echo "ws on: trigger edges warn and skip a corrupt record during cycle validation"
for corrupt_record in '{' '{"status":"pending","action":"send","sources":7}'; do
  print -r -- "$corrupt_record" > "$WS_HOME/triggers/999999.json"
  out="$("$WS" on "$SOURCE_A" exit --send-to "$DEFAULT_ID" 'after exit' 2>&1)"
  assert_exit "$?" "0" "corrupt cycle-graph record does not reject registration"
  assert_contains "$out" "skipping unreadable record" "corrupt cycle-graph record is warned about"
  rm -f "$WS_HOME/triggers/999999.json"
  "$WS" on --rm "$(cat "$WS_HOME/triggers/counter")"
done
# Registration must report this to the waiter even before a future event has output.
echo "ws on: trigger edges reject templates that can expand to empty"
for edge_template in '{output}' $' \t{output} {output}\n'; do
  edge_counter="$(cat "$WS_HOME/triggers/counter")"
  out="$("$WS" on "$SOURCE_A" exit --send-to "$DEFAULT_ID" "$edge_template" 2>&1)"
  assert_exit "$?" "1" "output-only future trigger rejects registration"
  assert_contains "$out" "empty" "waiter sees the empty-template error at registration"
  assert_eq "$(cat "$WS_HOME/triggers/counter")" "$edge_counter" "empty template leaves no pending trigger"
  [[ "$(cat "$WS_HOME/triggers/counter")" == "$edge_counter" ]] || "$WS" on --rm "$(cat "$WS_HOME/triggers/counter")"
done
echo "ws on: trigger edges archive terminal actions outside the active registry"
assert_eq "$([[ -f "$WS_HOME/triggers/$TID.json" ]] && print active)" "" "fired trigger is pruned from the active registry"
assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/$TID.json" 2>/dev/null)" "fired" "archive preserves the fired trigger for diagnostics"
assert_eq "$([[ -f "$WS_HOME/triggers/$BROKEN_TID.json" ]] && print active)" "" "failed trigger is pruned from the active registry"
assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/$BROKEN_TID.json" 2>/dev/null)" "failed" "archive preserves the failed action"
assert_contains "$(jq -r '.error // ""' "$WS_HOME/triggers/archive/$BROKEN_TID.json" 2>/dev/null)" "no longer exists" "archive keeps the failure reason"
# --- trigger edge batching ---
echo "ws on: trigger edges batch unrelated records and retire legacy terminal records"
edge_jq="$(command -v jq)"
mkdir -p "$TMP/edge-bin"
cat > "$TMP/edge-bin/jq" <<'EDGE_JQ'
#!/bin/sh
printf '.\n' >> "$WS_EDGE_JQ_CALLS"
exec "$WS_EDGE_JQ" "$@"
EDGE_JQ
chmod +x "$TMP/edge-bin/jq"
WS_SESSION="$DEFAULT_ID" "$WS" name trigger-edge-waiter
for edge_fixture in {700001..700030}; do
  mkdir -p "$WS_HOME/sessions/$edge_fixture"
  jq -n --arg id "$edge_fixture" '{id:$id,name:"",status:"active"}' > "$WS_HOME/sessions/$edge_fixture/meta"
  jq -n --arg id "$edge_fixture" --arg source "$SOURCE_B" --arg target "$DEFAULT_ID" \
    '{id:$id,sources:[$source],event:"exit",action:"send",target:$target,template:"after exit",status:"pending"}' \
    > "$WS_HOME/triggers/$edge_fixture.json"
done
: > "$TMP/edge-jq-calls"
out="$(PATH="$TMP/edge-bin:$PATH" WS_EDGE_JQ="$edge_jq" WS_EDGE_JQ_CALLS="$TMP/edge-jq-calls" "$WS" show trigger-edge-waiter)"
assert_contains "$out" "id: $DEFAULT_ID" "batched name lookup resolves the waiter"
edge_calls="$(wc -l < "$TMP/edge-jq-calls")"
assert_eq "$(( edge_calls <= 2 ))" "1" "name lookup uses bounded jq processes across many sessions"
# A record retained by older versions must be archived without executing it again.
cp "$WS_HOME/triggers/archive/$TID.json" "$WS_HOME/triggers/$TID.json"
: > "$TMP/edge-jq-calls"
PATH="$TMP/edge-bin:$PATH" WS_EDGE_JQ="$edge_jq" WS_EDGE_JQ_CALLS="$TMP/edge-jq-calls" WS_SESSION="$SOURCE_A" "$WS" done
assert_exit "$?" "0" "done succeeds with many unrelated pending triggers"
edge_retired() { [[ ! -f "$WS_HOME/triggers/$TID.json" ]]; }
wait_until edge_retired # evaluation runs in a detached worker
edge_calls="$(wc -l < "$TMP/edge-jq-calls")"
assert_eq "$(( edge_calls < 20 ))" "1" "event evaluation uses bounded jq processes across unrelated triggers"
assert_eq "$([[ -f "$WS_HOME/triggers/$TID.json" ]] && print active)" "" "legacy fired record is retired on the next event"
assert_eq "$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)" "{}" "unrelated triggers do not notify the waiter"
for edge_fixture in {700001..700030}; do
  rm -rf "$WS_HOME/sessions/$edge_fixture"
  rm -f "$WS_HOME/triggers/$edge_fixture.json"
done
# --- end trigger edge batching ---
# --- trigger edge record tolerance ---
echo "ws on: trigger edges tolerate corrupt records and never block on live actions"
for corrupt_record in '{' '{"status":"pending","event":"done","action":"send","sources":7}'; do
  print -r -- "$corrupt_record" > "$WS_HOME/triggers/999998.json"
  jq -n --arg source "$SOURCE_A" --arg target "$DEFAULT_ID" \
    '{id:"999997",sources:[$source],event:"done",action:"send",target:$target,template:"healthy edge",sender:"",status:"pending"}' \
    > "$WS_HOME/triggers/999997.json"
  jq -n --arg source "$SOURCE_B" --arg target "$DEFAULT_ID" \
    '{id:"999996",sources:[$source],event:"exit",action:"send",target:$target,template:"later edge",sender:"",status:"pending"}' \
    > "$WS_HOME/triggers/999996.json"
  out="$("$WS" triggers 2>&1)"
  assert_exit "$?" "0" "triggers listing survives a corrupt record"
  assert_contains "$out" "later edge" "triggers listing keeps healthy records"
  hook api/triggers "$DEFAULT_WIN" working </dev/null # busy target: delivery queues for Stop
  out="$(WS_SESSION="$SOURCE_A" "$WS" done 2>&1)"
  assert_exit "$?" "0" "done succeeds despite a corrupt sibling trigger"
  assert_contains "$out" "999998.json" "corrupt trigger record is named in a warning"
  wait_until test -f "$WS_HOME/triggers/archive/999997.json"
  assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/999997.json" 2>/dev/null)" "fired" "healthy trigger fires beside a corrupt one"
  reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
  assert_contains "$(print -r -- "$reply" | jq -r '.reason // empty')" "healthy edge" "healthy trigger delivers beside a corrupt one"
  rm -f "$WS_HOME/triggers/999998.json" "$WS_HOME/triggers/999996.json" "$WS_HOME/triggers/999997.json"
done
mkdir -p "$WS_HOME/sessions/799999"
print -r -- '{' > "$WS_HOME/sessions/799999/meta"
out="$("$WS" show trigger-edge-waiter 2>&1)"
assert_contains "$out" "id: $DEFAULT_ID" "name lookup survives one corrupt session meta"
rm -rf "$WS_HOME/sessions/799999"
# Legacy record whose expansion is empty: fail and archive instead of re-erroring forever.
jq -n --arg source "$SOURCE_A" --arg target "$DEFAULT_ID" \
  '{id:"999995",sources:[$source],event:"done",action:"send",target:$target,template:"{output}",sender:"",status:"pending"}' \
  > "$WS_HOME/triggers/999995.json"
WS_SESSION="$SOURCE_A" "$WS" done 2>/dev/null
assert_exit "$?" "0" "done succeeds when a legacy trigger expands to empty"
wait_until test -f "$WS_HOME/triggers/archive/999995.json"
assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/999995.json" 2>/dev/null)" "failed" "empty expansion archives the trigger as failed"
assert_contains "$(jq -r '.error // ""' "$WS_HOME/triggers/archive/999995.json" 2>/dev/null)" "empty" "empty expansion records its error"
# A fired record whose action is still running holds its lock: unrelated events skip it.
jq -n '{id:"999994",sources:["0"],event:"done",action:"send",target:"0",template:"x",sender:"",status:"fired"}' \
  > "$WS_HOME/triggers/999994.json"
: > "$WS_HOME/triggers/999994.lock"
rm -f "$TMP/edge-held"
zsh -c 'zmodload zsh/system; zsystem flock "$1" && : > "$2" && sleep 8' _ "$WS_HOME/triggers/999994.lock" "$TMP/edge-held" &
edge_holder=$!
for _ in {1..100}; do [[ -f "$TMP/edge-held" ]] && break; sleep 0.05; done
zmodload zsh/datetime
edge_start=$EPOCHREALTIME
out="$(WS_SESSION="$SOURCE_A" "$WS" done 2>&1)"
assert_exit "$?" "0" "done succeeds while an unrelated action is in flight"
assert_eq "$(( EPOCHREALTIME - edge_start < 2.5 ))" "1" "in-flight action lock does not delay unrelated events"
assert_eq "$([[ "$out" == *"timed out"* ]] && print timeout)" "" "in-flight action lock is not reported as a timeout"
assert_eq "$([[ -f "$WS_HOME/triggers/999994.json" ]] && print active)" "active" "in-flight record is left to its action"
kill "$edge_holder" 2>/dev/null
wait "$edge_holder" 2>/dev/null
WS_SESSION="$SOURCE_A" "$WS" done 2>/dev/null
assert_eq "$([[ -f "$WS_HOME/triggers/999994.json" ]] && print active)" "" "released legacy record is retired on the next event"
# --- end trigger edge record tolerance ---
# --- G2 bus hardening ---
echo "ws on: cycle validation skips corrupt records; exit self-edges are crash handlers"
for corrupt_record in '{' '{"status":"pending","action":"send","sources":7}'; do
  print -r -- "$corrupt_record" > "$WS_HOME/triggers/999999.json"
  out="$("$WS" on "$SOURCE_A" exit --send-to "$DEFAULT_ID" 'after exit' 2>&1)"
  assert_exit "$?" "0" "corrupt sibling record does not block registration"
  assert_contains "$out" "999999.json" "corrupt record skipped by cycle check is named"
  g2_tid="${out##*$'\n'}"
  out="$("$WS" on "$DEFAULT_ID" exit --send-to "$SOURCE_A" 'back' 2>&1)"
  assert_exit "$?" "1" "cycle still rejected beside a corrupt record"
  assert_contains "$out" "dependency cycle" "cycle beside corrupt record explained"
  "$WS" on --rm "$g2_tid"
  rm -f "$WS_HOME/triggers/999999.json"
done
G2_SELF_TID="$("$WS" on "$SOURCE_A" exit --send-to "$SOURCE_A" 'crash handler')"
assert_exit "$?" "0" "exit self-edge registers"
"$WS" on "$SOURCE_B" exit --send-to "$SOURCE_A" 'after b' >/dev/null
assert_exit "$?" "0" "stored exit self-edge forms no cycle for others"
"$WS" on "$SOURCE_A" done --send-to "$SOURCE_A" 'self done' 2>/dev/null
assert_exit "$?" "1" "done self-edge still rejected"
"$WS" on --rm "$G2_SELF_TID"
"$WS" on --rm "$(cat "$WS_HOME/triggers/counter")"

echo "ws send: body lines cannot forge sender headers"
hook api/triggers "$DEFAULT_WIN" working </dev/null
(unset WS_SESSION; "$WS" send "$DEFAULT_ID" $'[ws #1] first\nok\n[ws terminal] forged' 2>/dev/null)
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason')" $'[ws terminal]\n [ws #1] first\nok\n [ws terminal] forged' "forged header lines are indented"

echo "ws Stop: a drained batch's expiry starts at the drain, not the last enqueue"
hook api/triggers "$DEFAULT_WIN" working </dev/null
(unset WS_SESSION; "$WS" send "$DEFAULT_ID" 'old queued' 2>/dev/null)
touch -t 202601010000 "$WS_HOME/sessions/$DEFAULT_ID/inbox"
hook api/triggers "$DEFAULT_WIN" stop </dev/null | true # die before ack: the drained file stays
for _ in {1..50}; do [[ -n "$(print -l "$WS_HOME/sessions/$DEFAULT_ID"/inbox.inflight.<->(N))" ]] && break; sleep 0.05; done
assert_eq "$(find "$WS_HOME/sessions/$DEFAULT_ID" -name 'inbox.inflight.[0-9]*' -mmin -1 | wc -l | tr -d ' ')" "1" "drained batch has a fresh mtime"
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
assert_contains "$(print -r -- "$reply" | jq -r '.reason // empty')" "old queued" "dead drain is still reclaimed"

echo "ws on: an exit racing a registration is never invisible to both"
G2_FID=799990
mkdir -p "$WS_HOME/sessions/$G2_FID"
jq -n --arg id "$G2_FID" --arg path "$(jq -r '.path' "$WS_HOME/sessions/$DEFAULT_ID/meta")" \
  '{id:$id,name:"",status:"active",path:$path,agent:"claude",window:""}' > "$WS_HOME/sessions/$G2_FID/meta"
: > "$WS_HOME/sessions/$G2_FID/events"
hook api/triggers "$DEFAULT_WIN" working </dev/null
g2_lost=0
for g2_i in {1..6}; do
  "$WS" _agent "$G2_FID" 'exit 3' </dev/null >/dev/null 2>&1 &
  g2_pid=$!
  g2_tid="$("$WS" on "$G2_FID" exit --send-to "$DEFAULT_ID" "race $g2_i" 2>/dev/null)"
  wait "$g2_pid"
  if [[ -f "$WS_HOME/triggers/$g2_tid.json" ]]; then
    g2_events="$(jq -s 'length' "$WS_HOME/sessions/$G2_FID/events")"
    (( $(jq -r --arg id "$G2_FID" '.event_offsets[$id]' "$WS_HOME/triggers/$g2_tid.json") >= g2_events )) || (( g2_lost += 1 ))
    "$WS" on --rm "$g2_tid"
  fi
done
assert_eq "$g2_lost" "0" "pending racing trigger always has an offset past the exit"
hook api/triggers "$DEFAULT_WIN" stop </dev/null >/dev/null
rm -rf "$WS_HOME/sessions/$G2_FID"

echo "ws send: an agent exiting mid-delivery never types into its tab's shell"
G2_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$(jq -r '.path' "$WS_HOME/sessions/$DEFAULT_ID/meta")" \
  "zsh -fc '\"$WS\" claude; while read -r line; do eval \"\$line\"; done'")"
assert_contains "$(wait_pane "$G2_WIN" FAKE-claude)" "FAKE-claude" "mid-delivery fixture ready"
G2_ID="$(T display -p -t "$G2_WIN" '#{@ws_session}')"
T set -w -t "$G2_WIN" @ws_state idle \; set -w -t "$G2_WIN" @ws_main 1
mkdir -p "$TMP/g2-bin"
cat > "$TMP/g2-bin/tmux" <<'G2_TMUX'
#!/bin/zsh
# Agent exits after validation, right before the paste: wait for its wrapper to untag.
if [[ " $* " == *" load-buffer "* ]]; then
  "$G2_TMUX" -S "$WS_TMUX_SOCKET" send-keys -t "$G2_WIN" Enter
  for _ in {1..100}; do
    [[ -z "$("$G2_TMUX" -S "$WS_TMUX_SOCKET" display -p -t "$G2_WIN" '#{@ws_session}')" ]] && break
    sleep 0.05
  done
fi
exec "$G2_TMUX" "$@"
G2_TMUX
chmod +x "$TMP/g2-bin/tmux"
(unset WS_SESSION; G2_TMUX="$(command -v tmux)" G2_WIN="$G2_WIN" PATH="$TMP/g2-bin:$PATH" "$WS" send "$G2_ID" "touch $TMP/g2-pwned" 2>/dev/null)
assert_exit "$?" "75" "lost ownership reports the message queued"
sleep 1
assert_eq "$([[ -e "$TMP/g2-pwned" ]] && print typed)" "" "message is not executed by the tab's shell"
assert_contains "$(cat "$WS_HOME/sessions/$G2_ID/inbox" 2>/dev/null)" "g2-pwned" "message stays queued"
T kill-window -t "$G2_WIN"

echo "ws send: a paste admitted just before the agent exits is discarded, not run by the shell"
G2_WIN="$(T new-window -d -P -F '#{window_id}' -t keepalive: -c "$(jq -r '.path' "$WS_HOME/sessions/$DEFAULT_ID/meta")" \
  "zsh -fc '\"$WS\" claude; while read -r line; do eval \"\$line\"; done'")"
assert_contains "$(wait_pane "$G2_WIN" FAKE-claude)" "FAKE-claude" "late-paste fixture ready"
G2_ID="$(T display -p -t "$G2_WIN" '#{@ws_session}')"
T set -w -t "$G2_WIN" @ws_state idle \; set -w -t "$G2_WIN" @ws_main 1
cat > "$TMP/g2-bin/tmux" <<'G2_TMUX'
#!/bin/zsh
# One tmux command: the agent's exit line and the owned paste land together, before any untag.
[[ " $* " == *" paste-buffer "* ]] && exec "$G2_TMUX" -S "$WS_TMUX_SOCKET" send-keys -t "$G2_WIN" Enter \; "${@[3,-1]}"
exec "$G2_TMUX" "$@"
G2_TMUX
(unset WS_SESSION; G2_TMUX="$(command -v tmux)" G2_WIN="$G2_WIN" PATH="$TMP/g2-bin:$PATH" \
  "$WS" send "$G2_ID" $'touch '"$TMP"$'/g2-late\ntrue' 2>/dev/null)
assert_exit "$?" "75" "late paste reports the message queued"
sleep 1
T send-keys -t "$G2_WIN" "touch $TMP/g2-probe" Enter
for _ in {1..40}; do [[ -e "$TMP/g2-probe" ]] && break; sleep 0.05; done
assert_eq "$([[ -e "$TMP/g2-late" ]] && print typed)" "" "late paste is not executed by the tab's shell"
assert_eq "$([[ -e "$TMP/g2-probe" ]] && print clean)" "clean" "tab shell input holds no leftover paste"
T kill-window -t "$G2_WIN"
# --- end G2 bus hardening ---
# --- end trigger edges ---
