#!/bin/zsh
# ws tests: `ws send` delivery: paste, inbox, Stop hooks, cancellation, readiness and robustness.
# Not standalone: ws/tests/run.sh sources helpers.sh then the area files in order, in one process,
# because workspaces, tabs and global session ids carry over between areas.
(( $+functions[ws_tests_require_sandbox] )) && ws_tests_require_sandbox || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

echo "ws send: idle named target receives one bracketed multiline prompt with sender id"
"$WS" new --repo "$REPO" --branch delivery --detach
SEND_WT="$WS_ROOT/api/delivery"
SEND_WIN="$(windows_of "$SEND_WT")"
SEND_ID="$(T display -p -t "$SEND_WIN" '#{@ws_session}')"
assert_contains "$(wait_pane "$SEND_WIN" PASTE-READY)" "PASTE-READY" "fake delivery TUI ready"
WS_SESSION="$SEND_ID" "$WS" name receiver
MESSAGE=$'first line\nsecond "quoted" line $literal'
print -r -- "$MESSAGE" | WS_SESSION=1 "$WS" send receiver -
assert_exit "$?" "0" "send accepts stdin and target name"
wait_until test -s "$WS_HOME/pane-$SEND_ID.jsonl"
assert_eq "$(jq -r '.text' "$WS_HOME/pane-$SEND_ID.jsonl" 2>/dev/null)" $'[ws #1]\nfirst line\nsecond "quoted" line $literal' "sender prefix and multiline text reach pane intact"
assert_eq "$(jq -r '.raw | startswith("\u001b[200~") and endswith("\u001b[201~\r")' "$WS_HOME/pane-$SEND_ID.jsonl" 2>/dev/null)" "true" "paste is bracketed with separate Enter"
assert_eq "$(wc -l < "$WS_HOME/pane-$SEND_ID.jsonl" 2>/dev/null | tr -d ' ')" "1" "multiline message submits exactly once"
echo "ws send: working and waiting sessions consume inbox once through synchronous Stop hooks"
for agent in claude codex grok; do
  if [[ "$agent" == claude ]]; then
    BUS_WIN="$SEND_WIN"
  else
    "$WS" add api/delivery --agent "$agent" --detach
    BUS_WIN="$("$WS" list | awk -F '\t' -v p="$SEND_WT" -v a="$agent" '$3 == p && $4 == a {print $5; exit}')"
    assert_contains "$(wait_pane "$BUS_WIN" PASTE-READY)" "PASTE-READY" "$agent fake TUI ready"
  fi
  BUS_ID="$(T display -p -t "$BUS_WIN" '#{@ws_session}')"
  for busy_state in working waiting; do
    hook api/delivery "$BUS_WIN" "$busy_state" </dev/null
    before="$(cat "$WS_HOME/pane-$BUS_ID.jsonl" 2>/dev/null)"
    WS_SESSION=1 "$WS" send "$BUS_ID" "$agent $busy_state message"
    assert_exit "$?" "0" "$agent queues while $busy_state"
    assert_eq "$(cat "$WS_HOME/pane-$BUS_ID.jsonl" 2>/dev/null)" "$before" "$agent busy message never goes into pane input"
    reply="$(print '{"hook_event_name":"Stop","stop_hook_active":false,"stopHookActive":false}' | hook api/delivery "$BUS_WIN" stop)"
    assert_eq "$(print -r -- "$reply" | jq -r '.decision' 2>/dev/null)" "block" "$agent Stop requests continuation"
    assert_eq "$(print -r -- "$reply" | jq -r '.reason' 2>/dev/null)" "[ws #1]"$'\n'"$agent $busy_state message" "$agent Stop delivers sender-prefixed message"
    assert_eq "$(T display -p -t "$BUS_WIN" '#{@ws_state}')" "working" "$agent block keeps tab working"
    assert_eq "$([[ -s "$WS_HOME/sessions/$BUS_ID/inbox" ]] && print pending)" "" "$agent Stop consumes durable inbox"
    reply="$(print '{"hook_event_name":"Stop","stop_hook_active":true,"stopHookActive":true}' | hook api/delivery "$BUS_WIN" stop)"
    assert_eq "$reply" "{}" "$agent continued Stop never repeats consumed messages"
    assert_eq "$(T display -p -t "$BUS_WIN" '#{@ws_state}')" "idle" "$agent empty Stop marks idle"
  done
done
assert_eq "$(jq -r '.hooks.Stop[0].hooks[0].command' "$WS_HOME/claude-settings.json")" "$WS hook stop" "Claude Stop uses synchronous bus delivery"
CODEX_STOP_ID="$(T display -p -t "$(slot_window "$SEND_WT" 2)" '#{@ws_session}')"
assert_eq "$(python3 - "$WS_HOME/argv-$CODEX_STOP_ID.json" <<'PY_STOP'
import json, sys, tomllib
config = next(arg for arg in json.load(open(sys.argv[1])) if arg.startswith("hooks.Stop="))
print(tomllib.loads(config)["hooks"]["Stop"][0]["hooks"][0]["command"])
PY_STOP
)" "$WS hook stop" "Codex Stop uses synchronous bus delivery"
assert_eq "$(jq -r '.hooks.Stop[0].hooks[0].command' "$GROK_HOME/hooks/ws.json")" "$WS hook stop" "Grok Stop uses synchronous bus delivery"

pane_received() { [[ "$(tail -1 "$WS_HOME/pane-$1.jsonl" 2>/dev/null | jq -r '.text')" == "$2" ]]; }
echo "ws send: continuation Stops block only for new messages; cancellation pastes on verified idle"
for agent in claude codex grok; do
  BUS_WIN="$("$WS" list | awk -F '\t' -v p="$SEND_WT" -v a="$agent" '$3 == p && $4 == a {print $5; exit}')"
  BUS_ID="$(T display -p -t "$BUS_WIN" '#{@ws_session}')"
  hook api/delivery "$BUS_WIN" working </dev/null
  WS_SESSION=1 "$WS" send "$BUS_ID" "$agent guarded message"
  if [[ "$agent" == grok ]]; then active_payload='{"stopHookActive":true}'; else active_payload='{"stop_hook_active":true}'; fi
  before="$(cat "$WS_HOME/pane-$BUS_ID.jsonl" 2>/dev/null)"
  reply="$(print -r -- "$active_payload" | hook api/delivery "$BUS_WIN" stop)"
  assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty' 2>/dev/null)" "[ws #1]"$'\n'"$agent guarded message" "$agent active Stop blocks for new messages"
  assert_eq "$(cat "$WS_HOME/pane-$BUS_ID.jsonl" 2>/dev/null)" "$before" "$agent active Stop never pastes into the blocked TUI"
  assert_eq "$(T display -p -t "$BUS_WIN" '#{@ws_state}')" "working" "$agent continued delivery is working"
  reply="$(print -r -- "$active_payload" | hook api/delivery "$BUS_WIN" stop)"
  assert_eq "$reply" "{}" "$agent active Stop with an empty inbox allows stopping"
  hook api/delivery "$BUS_WIN" working </dev/null
  WS_SESSION=1 "$WS" send "$BUS_ID" "$agent cancelled message"
  hook api/delivery "$BUS_WIN" idle </dev/null
  wait_until pane_received "$BUS_ID" "[ws #1]"$'\n'"$agent cancelled message"
  assert_eq "$(tail -1 "$WS_HOME/pane-$BUS_ID.jsonl" 2>/dev/null | jq -r '.text')" "[ws #1]"$'\n'"$agent cancelled message" "$agent cancellation delivers inbox at idle"
  assert_eq "$([[ -s "$WS_HOME/sessions/$BUS_ID/inbox" ]] && print pending)" "" "$agent fallback consumes inbox"
done

echo "ws send: a background subagent postpones idle paste until the whole tab is idle"
hook api/delivery "$SEND_WIN" working </dev/null
print '{"agent_id":"delivery-bg"}' | hook api/delivery "$SEND_WIN" subagent-start
WS_SESSION=1 "$WS" send "$SEND_ID" "after background"
before="$(cat "$WS_HOME/pane-$SEND_ID.jsonl")"
hook api/delivery "$SEND_WIN" idle </dev/null
assert_eq "$(cat "$WS_HOME/pane-$SEND_ID.jsonl")" "$before" "background activity prevents fallback input"
print '{"agent_id":"delivery-bg"}' | hook api/delivery "$SEND_WIN" subagent-stop
wait_until pane_received "$SEND_ID" $'[ws #1]\nafter background'
assert_eq "$(tail -1 "$WS_HOME/pane-$SEND_ID.jsonl" | jq -r '.text')" $'[ws #1]\nafter background' "last subagent stop drains idle inbox"

# --- M3 stale cancel ---
echo "ws hook: a late Grok cancellation cannot idle a newer turn or paste its inbox"
M3_WIN="$(slot_window "$SEND_WT" 3)"
M3_ID="$(T display -p -t "$M3_WIN" '#{@ws_session}')"
print '{"hookEventName":"user_prompt_submit","promptId":"grok-old"}' | hook api/delivery "$M3_WIN" working
print '{"hookEventName":"user_prompt_submit","promptId":"grok-new"}' | hook api/delivery "$M3_WIN" working
WS_SESSION=1 "$WS" send "$M3_ID" "after the current turn"
M3_INBOX="$(cat "$WS_HOME/sessions/$M3_ID/inbox")"
M3_PANE="$(cat "$WS_HOME/pane-$M3_ID.jsonl" 2>/dev/null)"
print '{"hookEventName":"stop_cancelled","promptId":"grok-old","reason":"user_interrupt"}' | hook api/delivery "$M3_WIN" idle
assert_contains "$("$WS" sessions "$M3_ID")" $'\tgrok\tworking\tactive' "stale Grok cancel leaves the newer turn working"
assert_eq "$(cat "$WS_HOME/sessions/$M3_ID/inbox")" "$M3_INBOX" "stale Grok cancel keeps the inbox intact"
assert_eq "$(cat "$WS_HOME/pane-$M3_ID.jsonl" 2>/dev/null)" "$M3_PANE" "stale Grok cancel submits no pane input"
print '{}' | hook api/delivery "$M3_WIN" stop >/dev/null

echo "ws hook: the configured Codex Interrupt without a turn id cannot settle or paste"
M3_WIN="$(slot_window "$SEND_WT" 2)"
M3_ID="$(T display -p -t "$M3_WIN" '#{@ws_session}')"
M3_CANCEL_COMMAND="$(python3 - "$WS_HOME/argv-$M3_ID.json" <<'PY_M3'
import json, sys, tomllib
argv = json.load(open(sys.argv[1]))
config = next(arg for arg in argv if arg.startswith("hooks.Interrupt="))
print(tomllib.loads(config)["hooks"]["Interrupt"][0]["hooks"][0]["command"])
PY_M3
)"
print '{"hook_event_name":"UserPromptSubmit","turn_id":"codex-current"}' | hook api/delivery "$M3_WIN" working
WS_SESSION=1 "$WS" send "$M3_ID" "idless interruption"
M3_INBOX="$(cat "$WS_HOME/sessions/$M3_ID/inbox")"
M3_PANE="$(cat "$WS_HOME/pane-$M3_ID.jsonl" 2>/dev/null)"
print '{}' | WS_WORKSPACE=api/delivery TMUX_PANE="$(T display -p -t "$M3_WIN" '#{pane_id}')" zsh -c "$M3_CANCEL_COMMAND"
assert_eq "$(cat "$WS_HOME/sessions/$M3_ID/inbox")" "$M3_INBOX" "idless Codex Interrupt keeps queued messages"
assert_eq "$(cat "$WS_HOME/pane-$M3_ID.jsonl" 2>/dev/null)" "$M3_PANE" "idless Codex Interrupt never pastes"
assert_contains "$("$WS" sessions "$M3_ID")" $'\tcodex\tworking\tactive' "idless Codex Interrupt leaves the turn working"
print '{}' | hook api/delivery "$M3_WIN" stop >/dev/null

echo "ws hook: matching cancellations deliver once and cannot cancel the pasted turn"
for agent field slot in claude prompt_id 1 codex turn_id 2 grok promptId 3; do
  M3_WIN="$(slot_window "$SEND_WT" "$slot")"
  M3_ID="$(T display -p -t "$M3_WIN" '#{@ws_session}')"
  jq -nc --arg field "$field" '{hook_event_name:"UserPromptSubmit", ($field):"old"}' | hook api/delivery "$M3_WIN" working
  jq -nc --arg field "$field" '{hook_event_name:"UserPromptSubmit", ($field):"current | opaque"}' | hook api/delivery "$M3_WIN" working
  print '{"hook_event_name":"PreToolUse"}' | hook api/delivery "$M3_WIN" working
  jq -nc --arg field "$field" '{hook_event_name:"Interrupt", ($field):"old"}' | hook api/delivery "$M3_WIN" idle
  assert_contains "$("$WS" sessions "$M3_ID")" "$agent"$'\tworking\tactive' "$agent stale cancel with empty inbox keeps working"
  WS_SESSION=1 "$WS" send "$M3_ID" "$agent matching cancellation"
  M3_CANCEL_PAYLOAD="$(jq -nc --arg field "$field" '{($field):"current | opaque"}')"
  print -r -- "$M3_CANCEL_PAYLOAD" | hook api/delivery "$M3_WIN" cancel
  wait_until pane_received "$M3_ID" "[ws #1]"$'\n'"$agent matching cancellation"
  assert_exit "$?" "0" "$agent matching cancel pastes after idless tool hook"
  assert_eq "$([[ -s "$WS_HOME/sessions/$M3_ID/inbox" ]] && print pending)" "" "$agent matching cancel drains the inbox"
  WS_SESSION=1 "$WS" send "$M3_ID" "$agent next pasted turn"
  M3_INBOX="$(cat "$WS_HOME/sessions/$M3_ID/inbox")"
  M3_PANE="$(cat "$WS_HOME/pane-$M3_ID.jsonl")"
  print -r -- "$M3_CANCEL_PAYLOAD" | hook api/delivery "$M3_WIN" cancel
  assert_eq "$(cat "$WS_HOME/sessions/$M3_ID/inbox")" "$M3_INBOX" "$agent repeated cancel preserves the next turn's inbox"
  assert_eq "$(cat "$WS_HOME/pane-$M3_ID.jsonl")" "$M3_PANE" "$agent repeated cancel never pastes twice"
  print '{}' | hook api/delivery "$M3_WIN" stop >/dev/null
  jq -nc --arg field "$field" '{hook_event_name:"UserPromptSubmit", ($field):"current | opaque"}' | hook api/delivery "$M3_WIN" working
  print -r -- "$M3_CANCEL_PAYLOAD" | hook api/delivery "$M3_WIN" cancel
  assert_contains "$("$WS" sessions "$M3_ID")" "$agent"$'\tidle\tactive' "$agent matching cancel with empty inbox settles idle"
  jq -nc --arg field "$field" '{hook_event_name:"UserPromptSubmit", ($field):"current | opaque"}' | hook api/delivery "$M3_WIN" working
  print '{"hook_event_name":"UserPromptSubmit"}' | hook api/delivery "$M3_WIN" working
  print -r -- "$M3_CANCEL_PAYLOAD" | hook api/delivery "$M3_WIN" cancel
  assert_contains "$("$WS" sessions "$M3_ID")" "$agent"$'\tworking\tactive' "$agent new prompt without an id invalidates the previous turn"
  print '{}' | hook api/delivery "$M3_WIN" stop >/dev/null
done

# --- delivery robustness: session-end Stop ---
echo "ws hook: Grok session-end Stop leaves accepted work for resume"
ROBUST_WIN="$(slot_window "$SEND_WT" 3)"
ROBUST_ID="$(T display -p -t "$ROBUST_WIN" '#{@ws_session}')"
hook api/delivery "$ROBUST_WIN" working </dev/null
WS_SESSION=1 "$WS" send "$ROBUST_ID" "survive session end"
ROBUST_INBOX="$(cat "$WS_HOME/sessions/$ROBUST_ID/inbox")"
for reason in channel_closed shutdown; do
  reply="$(jq -nc --arg reason "$reason" '{hook_event_name:"Stop",reason:$reason}' | hook api/delivery "$ROBUST_WIN" stop)"
  assert_eq "$reply" "{}" "$reason allows session end without continuation"
  assert_eq "$(cat "$WS_HOME/sessions/$ROBUST_ID/inbox")" "$ROBUST_INBOX" "$reason retains the accepted inbox"
done
reply="$(print '{"hook_event_name":"Stop"}' | hook api/delivery "$ROBUST_WIN" stop)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'[ws #1]\nsurvive session end' "ordinary Stop still delivers retained work"
print '{}' | hook api/delivery "$ROBUST_WIN" stop >/dev/null
# --- end delivery robustness: session-end Stop ---

# --- delivery robustness: idle paste acknowledgement ---
echo "ws send: idle paste awaits a newer prompt submission before acknowledgement"
wait_until test -s "$WS_HOME/argv-$ROBUST_ID.json"
hook api/delivery "$ROBUST_WIN" idle </dev/null
: > "$WS_HOME/no-submit-$ROBUST_ID" # model a picker consuming Enter without a prompt hook
mkfifo "$TMP/robust-prompt.in"
exec {robust_w}<>"$TMP/robust-prompt.in"
(exec {robust_w}>&-; hook api/delivery "$ROBUST_WIN" working < "$TMP/robust-prompt.in") &
robust_stale_pid=$!
sleep 0.3
(exec {robust_w}>&-; WS_SESSION=1 "$WS" send "$ROBUST_ID" "await confirmation")
wait_until pane_received "$ROBUST_ID" $'[ws #1]\nawait confirmation'
assert_eq "$(T show -wqv -t "$ROBUST_WIN" @ws_state)" "idle" "paste alone never forces working"
ROBUST_PENDING="$(cat "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste" 2>/dev/null)"
assert_contains "$ROBUST_PENDING" "await confirmation" "unconfirmed paste stays durably in flight"
print -r -- '{"hook_event_name":"UserPromptSubmit"}' >&$robust_w
exec {robust_w}>&-
wait "$robust_stale_pid"
assert_eq "$(cat "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste" 2>/dev/null)" "$ROBUST_PENDING" "older prompt hook cannot ack a newer paste"
print '{"hook_event_name":"PreToolUse"}' | hook api/delivery "$ROBUST_WIN" working
assert_eq "$(cat "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste" 2>/dev/null)" "$ROBUST_PENDING" "tool hook cannot ack an unconfirmed paste"
print '{"hook_event_name":"UserPromptSubmit"}' | hook api/delivery "$ROBUST_WIN" working
assert_eq "$([[ -e "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste" ]] && print pending)" "" "newer prompt submission acknowledges the pasted batch"
assert_eq "$(T show -wqv -t "$ROBUST_WIN" @ws_state)" "working" "confirmed submission marks working"
print '{}' | hook api/delivery "$ROBUST_WIN" stop >/dev/null

echo "ws send: an overtaken prompt hook still confirms its earlier paste"
WS_SESSION=1 "$WS" send "$ROBUST_ID" "overtaken confirmation"
wait_until pane_received "$ROBUST_ID" $'[ws #1]\novertaken confirmation'
mkfifo "$TMP/robust-overtaken.in"
exec {robust_w}<>"$TMP/robust-overtaken.in"
(exec {robust_w}>&-; hook api/delivery "$ROBUST_WIN" working < "$TMP/robust-overtaken.in") &
robust_stale_pid=$!
sleep 0.5
print '{"hook_event_name":"PreToolUse"}' | hook api/delivery "$ROBUST_WIN" working
print -r -- '{"hook_event_name":"UserPromptSubmit"}' >&$robust_w
exec {robust_w}>&-
wait "$robust_stale_pid"
assert_eq "$([[ -e "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste" ]] && print pending)" "" "newer prompt acknowledges even when another hook overtakes its state update"
print '{}' | hook api/delivery "$ROBUST_WIN" stop >/dev/null

echo "ws send: swallowed Enter times out and requeues without forcing working"
WS_PASTE_ACK_TIMEOUT=1.5 WS_SESSION=1 "$WS" send "$ROBUST_ID" "swallowed Enter"
wait_until pane_received "$ROBUST_ID" $'[ws #1]\nswallowed Enter'
ROBUST_PANE="$(cat "$WS_HOME/pane-$ROBUST_ID.jsonl")"
WS_SESSION=1 "$WS" send "$ROBUST_ID" "queued after paste"
assert_eq "$(cat "$WS_HOME/pane-$ROBUST_ID.jsonl")" "$ROBUST_PANE" "sends during unconfirmed paste queue without a second Enter"
wait_until test -s "$WS_HOME/sessions/$ROBUST_ID/inbox"
wait_until test ! -e "$WS_HOME/sessions/$ROBUST_ID/inbox.inflight.paste"
assert_exit "$?" "0" "timeout requeues the unconfirmed batch"
assert_eq "$(jq -r '.message' "$WS_HOME/sessions/$ROBUST_ID/inbox")" $'[ws #1]\nswallowed Enter\n[ws #1]\nqueued after paste' "timeout retains both messages in delivery order"
assert_eq "$(T show -wqv -t "$ROBUST_WIN" @ws_state)" "idle" "timeout leaves the unconfirmed tab idle"
reply="$(print '{}' | hook api/delivery "$ROBUST_WIN" stop)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'[ws #1]\nswallowed Enter\n\n[ws #1]\nqueued after paste' "later Stop delivers the timed-out batch and queued sends"
rm -f "$WS_HOME/no-submit-$ROBUST_ID"
print '{}' | hook api/delivery "$ROBUST_WIN" stop >/dev/null
# --- end delivery robustness: idle paste acknowledgement ---

echo "ws send: concurrent senders retain every queued message"
hook api/delivery "$SEND_WIN" working </dev/null
send_pids=()
for i in {1..8}; do
  WS_SESSION=1 "$WS" send "$SEND_ID" "concurrent-$i" > "$TMP/send-$i.log" 2>&1 &
  send_pids+=($!)
done
for pid in $send_pids; do wait "$pid"; assert_exit "$?" "0" "concurrent send succeeds"; done
reply="$(print '{}' | hook api/delivery "$SEND_WIN" stop)"
assert_eq "$(print -r -- "$reply" | jq -r '[.reason | scan("concurrent-[1-8]")] | length')" "8" "one Stop delivers all concurrent messages"
for i in {1..8}; do assert_contains "$reply" "concurrent-$i" "queued sender $i survives"; done
assert_eq "$(print '{}' | hook api/delivery "$SEND_WIN" stop)" "{}" "concurrent inbox never repeats"

echo "ws send: done sessions warn; force resumes closed spawned sessions by retained id"
for agent in claude codex grok; do
  BUS_WIN="$("$WS" list | awk -F '\t' -v p="$SEND_WT" -v a="$agent" '$3 == p && $4 == a {print $5; exit}')"
  BUS_ID="$(T display -p -t "$BUS_WIN" '#{@ws_session}')"
  META="$WS_HOME/sessions/$BUS_ID/meta"
  jq '.status = "done" | .spawned = true | .resume_id = "resume-delivery"' "$META" > "$META.fixture"
  mv "$META.fixture" "$META"
  T kill-window -t "$BUS_WIN"
  out="$(WS_SESSION=1 "$WS" send "$BUS_ID" "wake again" 2>&1)"
  assert_exit "$?" "1" "$agent done send requires force"
  assert_contains "$out" "done" "$agent done target warns"
  assert_contains "$out" "--force" "$agent warning explains override"
  assert_eq "$("$WS" sessions "$BUS_ID" | cut -f5)" "stopped" "$agent rejected send never resumes target"
  out="$(WS_SESSION=1 "$WS" send "$BUS_ID" "wake again" --force 2>&1)"
  forced_exit=$?
  assert_exit "$forced_exit" "0" "$agent force send resumes and delivers"
  assert_contains "$out" "done" "$agent force send still warns"
  BUS_WIN="$(jq -r '.window' "$META")"
  if (( forced_exit == 0 )); then
    wait_until pane_received "$BUS_ID" $'[ws #1]\nwake again'
  fi
  assert_eq "$(tail -1 "$WS_HOME/pane-$BUS_ID.jsonl" | jq -r '.text')" $'[ws #1]\nwake again' "$agent resumed TUI receives message after delayed startup"
  assert_contains "$(T capture-pane -p -t "$BUS_WIN" 2>/dev/null)" "resume-delivery" "$agent resumes its recorded conversation"
  assert_eq "$("$WS" sessions "$BUS_ID" | cut -f6)" "done" "$agent force preserves declared done status"
done

echo "ws send: terminal sender and validation"
hook api/delivery "$BUS_WIN" idle </dev/null
env -u WS_SESSION "$WS" send --force "$BUS_ID" "from terminal" 2>/dev/null
assert_exit "$?" "0" "force is also accepted before target"
wait_until pane_received "$BUS_ID" $'[ws terminal]\nfrom terminal'
assert_eq "$(tail -1 "$WS_HOME/pane-$BUS_ID.jsonl" | jq -r '.text')" $'[ws terminal]\nfrom terminal' "untracked sender has terminal prefix"
"$WS" send no-such-session text 2>/dev/null
assert_exit "$?" "1" "unknown target refused"
print -n '' | "$WS" send "$BUS_ID" - 2>/dev/null
assert_exit "$?" "1" "empty stdin refused"
"$WS" send "$BUS_ID" 2>/dev/null
assert_exit "$?" "1" "missing message refused"

echo "ws send: startup safety also applies to a newly opened live target"
"$WS" add api/delivery --agent claude --detach
FRESH_WIN="$(slot_window "$SEND_WT" 4)"
FRESH_ID="$(T display -p -t "$FRESH_WIN" '#{@ws_session}')"
WS_SESSION=1 "$WS" send "$FRESH_ID" "fresh target"
assert_exit "$?" "0" "send waits for a new target's TUI before submitting"
wait_until pane_received "$FRESH_ID" $'[ws #1]\nfresh target'
assert_eq "$(tail -1 "$WS_HOME/pane-$FRESH_ID.jsonl" 2>/dev/null | jq -r '.text')" $'[ws #1]\nfresh target' "startup input is never discarded by terminal initialization"

echo "ws send: a slow older Stop cannot overwrite a newer pasted turn"
hook api/delivery "$FRESH_WIN" idle </dev/null
mkfifo "$TMP/send-slow.in"
exec {send_slow_w}<>"$TMP/send-slow.in"
(exec {send_slow_w}>&-; hook api/delivery "$FRESH_WIN" stop <"$TMP/send-slow.in" >"$TMP/send-slow.out") &
send_slow_pid=$!
sleep 0.5 # Stop captures its sequence before blocking on payload input.
(exec {send_slow_w}>&-; WS_SESSION=1 "$WS" send "$FRESH_ID" "newer turn")
assert_exit "$?" "0" "new message submits while old Stop awaits payload"
wait_until pane_received "$FRESH_ID" $'[ws #1]\nnewer turn'
assert_exit "$?" "0" "newer pasted turn confirms prompt submission"
exec {send_slow_w}>&-
wait "$send_slow_pid"
assert_eq "$(T display -p -t "$FRESH_WIN" '#{@ws_state}')" "working" "older Stop never idles a newer pasted turn"
assert_eq "$(cat "$TMP/send-slow.out")" "{}" "stale Stop still returns valid allow JSON"

echo "ws send: a split tab pastes only into the recorded agent pane"
hook api/delivery "$FRESH_WIN" idle </dev/null
SPLIT_PANE="$(T split-window -d -P -F '#{pane_id}' -t "$FRESH_WIN" "cat > '$TMP/split-shell.out'")"
T select-pane -t "$SPLIT_PANE"
WS_SESSION=1 "$WS" send "$FRESH_ID" "agent pane only"
assert_exit "$?" "0" "send to split tab succeeds"
wait_until pane_received "$FRESH_ID" $'[ws #1]\nagent pane only'
assert_eq "$(tail -1 "$WS_HOME/pane-$FRESH_ID.jsonl" | jq -r '.text')" $'[ws #1]\nagent pane only' "agent pane receives message"
assert_eq "$(cat "$TMP/split-shell.out" 2>/dev/null)" "" "selected shell pane never receives input"
T kill-pane -t "$SPLIT_PANE"

echo "ws send: a vanished agent pane keeps the message queued"
AGENT_PANE="$(T show -wqv -t "$FRESH_WIN" @ws_pane)"
T set -w -t "$FRESH_WIN" @ws_pane %99999
hook api/delivery "$FRESH_WIN" idle </dev/null
out="$(WS_SESSION=1 "$WS" send "$FRESH_ID" "no pane" 2>&1)"
assert_exit "$?" "75" "missing agent pane refuses paste (queued, exit 75)"
assert_contains "$out" "message queued" "missing pane explains pending delivery"
assert_eq "$(jq -r '.message' "$WS_HOME/sessions/$FRESH_ID/inbox")" $'[ws #1]\nno pane' "missing pane retains inbox"
T set -w -t "$FRESH_WIN" @ws_pane "$AGENT_PANE"
print '{}' | hook api/delivery "$FRESH_WIN" stop >/dev/null

# --- G2: send revalidates the target window after taking the hook lock ---
echo "ws send: a window closed while send waits for the hook lock resumes instead of stranding"
vanish_before="$(windows_of "$SEND_WT")"
"$WS" add api/delivery --agent claude --detach
VANISH_WIN="$(windows_of "$SEND_WT" | grep -vxF -f <(print -r -- "$vanish_before"))"
VANISH_ID="$(T display -p -t "$VANISH_WIN" '#{@ws_session}')"
assert_contains "$(wait_pane "$VANISH_WIN" PASTE-READY)" "PASTE-READY" "vanishing target TUI ready"
hook api/delivery "$VANISH_WIN" working </dev/null
VANISH_LOCK="${TMPDIR:-/tmp}/ws-hook-$WS_TMUX_SOCKET-$VANISH_WIN.lock"
(zmodload zsh/system; : >> "$VANISH_LOCK"; zsystem flock -f vanish_fd "$VANISH_LOCK"; : > "$TMP/vanish-held"; sleep 2) &
vanish_holder=$!
wait_until test -e "$TMP/vanish-held"
(WS_SESSION=1 "$WS" send "$VANISH_ID" "after vanish" >"$TMP/vanish.out" 2>&1; print $? > "$TMP/vanish.rc") &
vanish_sender=$!
sleep 1 # send resolved the live window and now blocks on the held hook lock
T kill-window -t "$VANISH_WIN"
wait "$vanish_holder"; wait "$vanish_sender"
assert_exit "$(cat "$TMP/vanish.rc")" "0" "send resumes a target whose window vanished before the hook lock"
wait_until pane_received "$VANISH_ID" $'[ws #1]\nafter vanish'
assert_eq "$(tail -1 "$WS_HOME/pane-$VANISH_ID.jsonl" 2>/dev/null | jq -r '.text')" $'[ws #1]\nafter vanish' "resumed target receives the message instead of stranding it"
# --- end G2 send revalidation ---

echo "ws send: done declared while send waits for the inbox lock is honored"
META="$WS_HOME/sessions/$FRESH_ID/meta"
cp "$META" "$TMP/fresh-meta"
: >> "$WS_HOME/sessions/$FRESH_ID/inbox.lock"
zsh -c 'zmodload zsh/system; zsystem flock "$1" && sleep 1.5' _ "$WS_HOME/sessions/$FRESH_ID/inbox.lock" &
race_lock_pid=$!
sleep 0.3
WS_SESSION=1 "$WS" send "$FRESH_ID" "after done" > "$TMP/race.out" 2>&1 &
race_send_pid=$!
sleep 0.3
jq '.status = "done"' "$META" > "$META.fixture" && mv "$META.fixture" "$META"
wait "$race_send_pid"
assert_exit "$?" "1" "send re-checks done under the inbox lock"
assert_contains "$(cat "$TMP/race.out")" "--force required" "racing send explains done guard"
assert_eq "$([[ -s "$WS_HOME/sessions/$FRESH_ID/inbox" ]] && print pending)" "" "unforced send never queues for a done session"
wait "$race_lock_pid"
cp "$TMP/fresh-meta" "$META"

echo "ws send: overlapping sends to a slow-starting tab never drop a message"
print 3.6 > "$WS_HOME/tui-delay"
"$WS" add api/delivery --agent claude --detach
SLOW_WIN="$(slot_window "$SEND_WT" 5)"
SLOW_ID="$(T display -p -t "$SLOW_WIN" '#{@ws_session}')"
WS_SESSION=1 "$WS" send "$SLOW_ID" "overlap-1" > "$TMP/overlap-1.log" 2>&1 &
overlap_1=$!
sleep 0.2
WS_SESSION=1 "$WS" send "$SLOW_ID" "overlap-2" > "$TMP/overlap-2.log" 2>&1 &
overlap_2=$!
wait "$overlap_1"; overlap_1_rc=$?
wait "$overlap_2"; overlap_2_rc=$?
for i in 1 2; do
  rc_var="overlap_${i}_rc"
  (( ${(P)rc_var} == 0 )) || assert_contains "$(cat "$TMP/overlap-$i.log")" "message queued" "overlap sender $i failure is reported"
done
overlap_seen() { [[ "$(cat "$WS_HOME/pane-$SLOW_ID.jsonl" "$WS_HOME/sessions/$SLOW_ID/inbox" 2>/dev/null)" == *overlap-1*overlap-2* ]]; }
wait_until overlap_seen
for i in 1 2; do
  assert_contains "$(cat "$WS_HOME/pane-$SLOW_ID.jsonl" "$WS_HOME/sessions/$SLOW_ID/inbox" 2>/dev/null)" "overlap-$i" "overlap message $i delivered or queued"
done
rm -f "$WS_HOME/tui-delay"

echo "ws send (F7): a message queued on a slow-starting idle tab is delivered without a re-send"
F7_BEFORE="$(windows_of "$SEND_WT")"
print 4 > "$WS_HOME/tui-delay"
"$WS" add api/delivery --agent claude --detach
F7_WIN="$(windows_of "$SEND_WT" | grep -vxF -f <(print -r -- "$F7_BEFORE"))"
F7_ID="$(T show -wqv -t "$F7_WIN" @ws_session)"
WS_READY_TIMEOUT=1 WS_DELIVER_RETRY_INTERVAL=1 WS_SESSION=1 "$WS" send "$F7_ID" "f7 late" >/dev/null 2>&1
assert_exit "$?" "75" "slow-start send reports queued"
wait_until pane_received "$F7_ID" "[ws #1]"$'\n'"f7 late"
assert_exit "$?" "0" "bounded retry delivers once the TUI is ready"
rm -f "$WS_HOME/tui-delay"

# --- delivery robustness: readiness and lock diagnostics ---
echo "ws send: readiness races release the inbox lock before waiting for input"
ROBUST_BEFORE="$(windows_of "$SEND_WT")"
print 8 > "$WS_HOME/tui-delay"
"$WS" add api/delivery --agent claude --detach
ROBUST_READY_WIN="$(windows_of "$SEND_WT" | grep -vxF -f <(print -r -- "$ROBUST_BEFORE"))"
ROBUST_READY_ID="$(T show -wqv -t "$ROBUST_READY_WIN" @ws_session)"
T set -w -t "$ROBUST_READY_WIN" @ws_main idle
ROBUST_LOCK="${TMPDIR:-/tmp}/ws-hook-$WS_TMUX_SOCKET-$ROBUST_READY_WIN.lock"
(zmodload zsh/system; : >> "$ROBUST_LOCK"; zsystem flock -f robust_fd "$ROBUST_LOCK"; : > "$TMP/robust-ready-held"; sleep 2.5) &
robust_holder=$!
wait_until test -e "$TMP/robust-ready-held"
(WS_SESSION=1 "$WS" send "$ROBUST_READY_ID" "after readiness race" > "$TMP/robust-ready.out" 2>&1; print $? > "$TMP/robust-ready.rc") &
robust_sender=$!
wait_until test -s "$WS_HOME/sessions/$ROBUST_READY_ID/inbox"
sleep 1 # sender has passed its unlocked main-state read and is waiting for the hook lock
T set -wu -t "$ROBUST_READY_WIN" @ws_main # races the sender's unlocked readiness check
wait "$robust_holder"
sleep 0.3 # sender now sees the missing main state under its hook/inbox locks
out="$(WS_SESSION="$ROBUST_READY_ID" "$WS" done 2>&1)"
assert_exit "$?" "1" "done refuses work while readiness waits"
assert_contains "$out" "pending messages" "readiness wait leaves the inbox lock available to done"
wait "$robust_sender"
assert_exit "$(cat "$TMP/robust-ready.rc")" "0" "send retries readiness after releasing its locks"
wait_until pane_received "$ROBUST_READY_ID" $'[ws #1]\nafter readiness race'
assert_exit "$?" "0" "unlocked readiness retry eventually delivers"
T kill-window -t "$ROBUST_READY_WIN"
rm -f "$WS_HOME/tui-delay"

echo "ws send: inbox lock timeout reports the lock instead of failing silently"
ROBUST_INBOX_LOCK="$WS_HOME/sessions/$SEND_ID/inbox.lock"
(zmodload zsh/system; : >> "$ROBUST_INBOX_LOCK"; zsystem flock -f robust_fd "$ROBUST_INBOX_LOCK"; : > "$TMP/robust-inbox-held"; sleep 4) &
robust_holder=$!
wait_until test -e "$TMP/robust-inbox-held"
out="$(WS_SESSION=1 "$WS" send "$SEND_ID" "lock timeout" 2>&1)"
assert_exit "$?" "1" "contended inbox lock times out"
assert_contains "$out" "timed out waiting for lock" "lock timeout emits a diagnostic"
wait "$robust_holder"
# --- end delivery robustness: readiness and lock diagnostics ---

echo "ws send: failed resume retains the message and reports it queued"
META="$WS_HOME/sessions/$FRESH_ID/meta"
jq --arg missing "$TMP/missing-workspace" '.path = $missing' "$META" > "$META.fixture"
mv "$META.fixture" "$META"
T kill-window -t "$FRESH_WIN"
out="$(WS_SESSION=1 "$WS" send "$FRESH_ID" "retain on failure" 2>&1)"
assert_exit "$?" "1" "missing workspace refuses resume"
assert_contains "$out" "message queued" "failure explains that delivery remains pending"
assert_eq "$(jq -r '.message' "$WS_HOME/sessions/$FRESH_ID/inbox")" $'[ws #1]\nretain on failure' "failed resume leaves a durable sender-prefixed message"

"$WS" rm api/delivery --force --delete-branch >/dev/null
