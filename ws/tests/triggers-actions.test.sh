#!/bin/zsh
# ws tests: `ws on` registration, cycles, spawn, failures, queued sends, spawn prompts, ready timeout.
# Not standalone: ws/tests/run.sh sources helpers.sh then the area files in order, in one process,
# because workspaces, tabs and global session ids carry over between areas.
[[ -n "${WS_TESTS_SUITE:-}" ]] || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

echo "ws on: rejects direct and transitive cycles; removal cancels a pending action"
CYCLE_TID="$("$WS" on "$SOURCE_A" exit --send-to "$TARGET_ID" cycle-forward)"
assert_exit "$?" "0" "forward dependency registers"
out="$("$WS" on "$TARGET_ID" exit --send-to "$SOURCE_A" cycle-backward 2>&1)"
assert_exit "$?" "1" "reverse edge forming a cycle is rejected"
assert_contains "$out" "cycle" "cycle error explains rejection"
"$WS" on "$TARGET_ID" done --send-to "$TARGET_ID" self-cycle 2>/dev/null
assert_exit "$?" "1" "direct self-dependency is rejected"
"$WS" on "$TARGET_ID,$SOURCE_B" done --send-to "$TARGET_ID" fan-in-cycle 2>/dev/null
assert_exit "$?" "1" "fan-in cannot include its own target"
CHAIN_TID="$("$WS" on "$TARGET_ID" exit --send-to "$SOURCE_B" cycle-chain)"
out="$("$WS" on "$SOURCE_B" exit --send-to "$SOURCE_A" cycle-end 2>&1)"
assert_exit "$?" "1" "three-session cycle is rejected"
assert_contains "$out" "cycle" "transitive cycle is explained"
"$WS" on --rm "$CHAIN_TID"
assert_exit "$?" "0" "removes pending chain trigger"
"$WS" on --rm "$CYCLE_TID"
assert_exit "$?" "0" "removes pending trigger"
assert_eq "$("$WS" triggers)" "" "removed triggers are absent"
"$WS" on --rm "$CYCLE_TID" 2>/dev/null
assert_exit "$?" "1" "removing an absent trigger is rejected"
hook api/triggers "$TARGET_WIN" working </dev/null
T send-keys -t "$SOURCE_A_WIN" Enter
wait_until lifecycle_exit_count "$SOURCE_A" 1
assert_eq "$(hook api/triggers "$TARGET_WIN" stop </dev/null)" "{}" "cancelled trigger never delivers"

echo "ws on: spawn expands a literal prompt and starts a tracked child in the same worktree"
printf 'spawn result {id} {output}' | WS_SESSION="$SOURCE_B" "$WS" done -
SPAWN_PROMPT=$'Follow "quoted" $literal\nsource {id}: {output}; $(touch never-execute-ws-prompt)'
for spawn_agent in claude codex grok; do
  before_ids="$("$WS" sessions | cut -f1 | paste -sd, -)"
  SPAWN_TID="$(WS_SESSION="$TARGET_ID" "$WS" on "$SOURCE_B" done --spawn "$SPAWN_PROMPT" --agent "$spawn_agent")"
  assert_exit "$?" "0" "$spawn_agent spawn trigger registers and fires"
  CHILD_ID="$("$WS" sessions | awk -F '\t' -v a="$spawn_agent" -v ids="$before_ids" '$3 == "api/triggers" && $4 == a { if (index("," ids ",", "," $1 ",") == 0) print $1 }')"
  assert_eq "${CHILD_ID:+found}" "found" "$spawn_agent spawn creates one new session"
  if [[ -n "$CHILD_ID" ]]; then
    CHILD_WIN="$(jq -r '.window' "$WS_HOME/sessions/$CHILD_ID/meta")"
    assert_contains "$(wait_pane "$CHILD_WIN" "FAKE-$spawn_agent")" "WHOAMI=$CHILD_ID" "$spawn_agent child has WS_SESSION and can query it"
    wait_until test -s "$WS_HOME/argv-$CHILD_ID.json"
    assert_agent_preamble "$spawn_agent" "$CHILD_ID" spawned
    assert_eq "$(jq -r '.[-1]' "$WS_HOME/argv-$CHILD_ID.json")" "Follow \"quoted\" \$literal"$'\n'"source $SOURCE_B: spawn result {id} {output}; \$(touch never-execute-ws-prompt)" "$spawn_agent initial prompt is one literal final argument"
    assert_eq "$(jq -r '.spawned' "$WS_HOME/sessions/$CHILD_ID/meta")" "true" "$spawn_agent child is marked spawned"
    assert_eq "$(jq -r '.path' "$WS_HOME/sessions/$CHILD_ID/meta")" "$TRIGGER_WT" "$spawn_agent child uses registering session worktree"
    T send-keys -t "$CHILD_WIN" done Enter
    wait_until spawn_done_closed "$CHILD_ID"
    assert_exit "$?" "0" "$spawn_agent spawned child closes on done"
    assert_eq "$(cat "$WS_HOME/sessions/$CHILD_ID/output.md")" "agent result" "$spawn_agent child publishes output before closure"
  fi
done
assert_eq "$([[ -e "$TRIGGER_WT/never-execute-ws-prompt" ]] && print yes)" "" "spawn prompt never executes shell substitution"

echo "ws on: concurrent last-source exits spawn once and default agent is claude"
DEFAULT_TID="$(WS_SESSION="$TARGET_ID" "$WS" on "$SOURCE_B" done --spawn default-child)"
assert_exit "$?" "0" "spawn accepts omitted agent"
DEFAULT_ID="$("$WS" sessions | awk -F '\t' '$3 == "api/triggers" && $5 != "stopped" && $1 != "'"$TARGET_ID"'" && $1 != "'"$SOURCE_B"'" {print $1}')"
assert_eq "$("$WS" sessions "$DEFAULT_ID" | cut -f4)" "claude" "spawn defaults to claude"
DEFAULT_WIN="$(jq -r '.window' "$WS_HOME/sessions/$DEFAULT_ID/meta")"
assert_contains "$(wait_pane "$DEFAULT_WIN" FAKE-claude)" "default-child" "default child receives initial prompt"
exit_before="$("$WS" sessions | wc -l | tr -d ' ')"
EXIT_TID="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B,$TARGET_ID" exit --spawn 'after exit {id}' --agent codex)"
assert_exit "$?" "0" "exit fan-in registers while both sources are live"
assert_eq "$("$WS" sessions | wc -l | tr -d ' ')" "$exit_before" "exit trigger does not confuse done with exit"
assert_contains "$("$WS" triggers)" "$EXIT_TID"$'\t'"$SOURCE_B,$TARGET_ID"$'\texit\tspawn' "pending spawn trigger is listed"
T send-keys -t "$SOURCE_B_WIN" Enter &
exit_a_pid=$!
T send-keys -t "$TARGET_WIN" Enter &
exit_b_pid=$!
wait "$exit_a_pid"; wait "$exit_b_pid"
wait_until lifecycle_exit_count "$SOURCE_B" 1
wait_until lifecycle_exit_count "$TARGET_ID" 1
exit_child_ready() { [[ "$("$WS" sessions | awk -F '\t' '$3 == "api/triggers" && $4 == "codex" && $5 != "stopped" {print $1}')" == <-> ]]; }
wait_until exit_child_ready
assert_exit "$?" "0" "concurrent exits start one codex child"
EXIT_CHILD="$("$WS" sessions | awk -F '\t' '$3 == "api/triggers" && $4 == "codex" && $5 != "stopped" {print $1}')"
assert_eq "$("$WS" sessions | wc -l | tr -d ' ')" "$((exit_before + 1))" "two racing evaluations allocate exactly one child"
assert_contains "$(wait_pane "$(jq -r '.window' "$WS_HOME/sessions/$EXIT_CHILD/meta")" FAKE-codex)" "after exit $SOURCE_B,$TARGET_ID" "exit child receives expanded source ids"
assert_eq "$("$WS" triggers)" "" "exit action is no longer pending"

echo "ws on: sends a literal dash instead of interpreting the trigger message as stdin"
hook api/triggers "$DEFAULT_WIN" working </dev/null
DASH_TID="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B" done --send-to me - </dev/null)"
assert_exit "$?" "0" "literal dash trigger registers"
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$DEFAULT_ID]"$'\n-' "trigger dash is delivered literally"

# --- G2: empty templates reject registration; failed actions keep their stderr ---
echo "ws on: output-only send and spawn templates reject registration"
empty_before="$(windows_of "$TRIGGER_WT")"
"$WS" add api/triggers --detach
EMPTY_WIN="$(windows_of "$TRIGGER_WT" | grep -vxF -f <(print -r -- "$empty_before"))"
EMPTY_SRC="$(T display -p -t "$EMPTY_WIN" '#{@ws_session}')"
WS_SESSION="$EMPTY_SRC" "$WS" done </dev/null
assert_exit "$?" "0" "empty-output source declares done"
empty_sessions="$("$WS" sessions | wc -l | tr -d ' ')"
for empty_action in --send-to --spawn; do
  empty_counter="$(cat "$WS_HOME/triggers/counter")"
  if [[ "$empty_action" == --send-to ]]; then
    out="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$EMPTY_SRC" done --send-to me '{output}' 2>&1)"
  else
    out="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$EMPTY_SRC" done --spawn ' {output}' 2>&1)"
  fi
  assert_exit "$?" "1" "$empty_action empty expansion reports failure"
  assert_contains "$out" "empty" "$empty_action empty expansion explains itself"
  assert_eq "$(cat "$WS_HOME/triggers/counter")" "$empty_counter" "$empty_action empty template allocates no trigger"
done
assert_eq "$("$WS" sessions | wc -l | tr -d ' ')" "$empty_sessions" "empty spawn prompt never starts a blank child"
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
assert_eq "$reply" "{}" "empty send expansion queues nothing"

echo "ws on: a failed trigger action records its error"
BROKEN_ID=999999
mkdir -p "$WS_HOME/sessions/$BROKEN_ID"
jq -n --arg id "$BROKEN_ID" '{id:$id,name:"",workspace:"api/gone",agent:"claude",status:"active",path:"/nonexistent/ws-gone",slot:1,window:""}' > "$WS_HOME/sessions/$BROKEN_ID/meta"
out="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$EMPTY_SRC" done --send-to "$BROKEN_ID" unreachable 2>&1)"
assert_exit "$?" "1" "unreachable trigger target fails"
BROKEN_TID="${${(f)out}[1]}"
assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/$BROKEN_TID.json")" "failed" "failed action is marked failed"
assert_contains "$(jq -r '.error // ""' "$WS_HOME/triggers/archive/$BROKEN_TID.json")" "no longer exists" "failed action record keeps stderr"
rm -rf "$WS_HOME/sessions/$BROKEN_ID"
T send-keys -t "$EMPTY_WIN" Enter
# --- end G2 trigger failures ---

# --- G4: queued-not-delivered sends are not failures ---
echo "ws send/on: a durably queued but undelivered send exits 75 and triggers record queued"
QUEUED_LOCK="${TMPDIR:-/tmp}/ws-hook-$WS_TMUX_SOCKET-$DEFAULT_WIN.lock"
rm -f "$TMP/queued-held" "$TMP/queued-release"
(zmodload zsh/system; : >> "$QUEUED_LOCK"; zsystem flock -f queued_fd "$QUEUED_LOCK"; : > "$TMP/queued-held"
  while [[ ! -e "$TMP/queued-release" ]]; do sleep 0.05; done) &
queued_holder=$!
wait_until test -e "$TMP/queued-held"
out="$(WS_SESSION=1 "$WS" send "$DEFAULT_ID" "queued direct" 2>&1)"
assert_exit "$?" "75" "busy target: direct send exits 75 (queued, not delivered)"
assert_contains "$out" "message queued" "queued send keeps its warning"
out="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$EMPTY_SRC" done --send-to me 'queued trigger' 2>&1)"
assert_exit "$?" "0" "ws on: queued inline action is not a failure"
QUEUED_TID="${${(f)out}[1]}"
assert_eq "$(jq -r '.status' "$WS_HOME/triggers/archive/$QUEUED_TID.json" 2>/dev/null)" "queued" "queued action archived as queued"
assert_contains "$(jq -r '.warning // ""' "$WS_HOME/triggers/archive/$QUEUED_TID.json" 2>/dev/null)" "message queued" "queued record keeps the warning"
: > "$TMP/queued-release"
wait "$queued_holder"
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null | jq -r '.reason // empty')"
assert_eq "$(print -r -- "$reply" | grep -c '^queued direct$')" "1" "queued direct send delivered exactly once"
assert_eq "$(print -r -- "$reply" | grep -c '^queued trigger$')" "1" "queued trigger send delivered exactly once"
# --- end G4 queued sends ---

echo "ws on: validates source lists, events and action options without creating triggers"
for bad_sources in '' ',' "$SOURCE_B," ",$SOURCE_B" "$SOURCE_B,,${TARGET_ID}" no-such-session; do
  "$WS" on "$bad_sources" done --send-to "$DEFAULT_ID" invalid 2>/dev/null
  assert_exit "$?" "1" "invalid source list rejected: $bad_sources"
done
"$WS" on "$SOURCE_B" idle --send-to "$DEFAULT_ID" invalid 2>/dev/null
assert_exit "$?" "1" "unsupported lifecycle event rejected"
"$WS" on "$SOURCE_B" done --send-to missing-target invalid 2>/dev/null
assert_exit "$?" "1" "unknown target rejected"
"$WS" on "$SOURCE_B" done --send-to "$DEFAULT_ID" '' 2>/dev/null
assert_exit "$?" "1" "empty message rejected"
WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B" done --spawn '' 2>/dev/null
assert_exit "$?" "1" "empty spawn prompt rejected"
WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B" done --spawn invalid --agent invalid 2>/dev/null
assert_exit "$?" "1" "unknown spawn agent rejected"
WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B" done --spawn invalid --send-to "$DEFAULT_ID" 2>/dev/null
assert_exit "$?" "1" "mixed send/spawn options rejected"
(cd "$REPO" && env -u WS_SESSION -u TMUX -u WS_WORKSPACE "$WS" on "$SOURCE_B" done --spawn invalid 2>/dev/null)
assert_exit "$?" "1" "terminal spawn outside ws worktree rejected"
"$WS" triggers unexpected 2>/dev/null
assert_exit "$?" "1" "triggers rejects unexpected arguments"
"$WS" on --rm ../counter 2>/dev/null
assert_exit "$?" "1" "remove rejects unsafe trigger ids"
assert_eq "$("$WS" triggers)" "" "invalid registrations create no pending triggers"

# --- G3: spawn prompt via file, open failure cleanup, ready timeout, non-interactive ws ---
echo "ws on: spawn prompts beyond tmux's command limit reach the agent argv intact"
new_session_id() { comm -13 <(print -r -- "$1" | sort) <(print -rl -- "$WS_HOME"/sessions/<->(N:t) | sort); }
BIG_PAD=""
BIG_PROMPT="big-start ${(l:24000::x:)BIG_PAD} \$(touch never-execute-big) big-end"
before_ids="$(print -rl -- "$WS_HOME"/sessions/<->(N:t))"
WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_B" done --spawn "$BIG_PROMPT" --agent claude >/dev/null
assert_exit "$?" "0" "large spawn trigger fires"
BIG_ID="$(new_session_id "$before_ids")"
# n5: session discovery must fail closed even when ls color is forced.
assert_eq "$([[ "$BIG_ID" == <-> ]] && print yes)" "yes" "large spawn discovers one numeric session id"
wait_until test -s "$WS_HOME/argv-$BIG_ID.json"
assert_eq "$(jq -r '.[-1]' "$WS_HOME/argv-$BIG_ID.json" 2>/dev/null)" "$BIG_PROMPT" "24KB spawn prompt is the literal final argument"
assert_eq "$(jq -r '.[-2]' "$WS_HOME/argv-$BIG_ID.json" 2>/dev/null)" "--" "large prompt still follows --"
assert_eq "$([[ -e "$WS_HOME/sessions/$BIG_ID/prompt" ]] && print yes)" "" "prompt file is consumed at launch"
assert_eq "$([[ -e "$TRIGGER_WT/never-execute-big" ]] && print yes)" "" "large prompt never executes substitution"
if [[ "$BIG_ID" == <-> ]]; then
  T send-keys -t "$(jq -r '.window' "$WS_HOME/sessions/$BIG_ID/meta")" done Enter
  wait_until spawn_done_closed "$BIG_ID"
fi

echo "ws on: a spawn whose tab fails to open leaves no slot row and a closed identity"
FAIL_RECORD="$TMP/fail-trigger.json"
jq -n --arg p "$TRIGGER_WT" --arg s "$DEFAULT_ID" '{id:"t-fail",path:$p,agent:"codex",sender:$s}' > "$FAIL_RECORD"
before_ids="$(print -rl -- "$WS_HOME"/sessions/<->(N:t))"
( source "$WS"; open_session() { return 1; }; ws_bus_spawn_trigger "$FAIL_RECORD" never-opened ) >/dev/null 2>&1
assert_exit "$?" "1" "failed spawn open reports failure"
FAIL_ID="$(new_session_id "$before_ids")"
assert_eq "$([[ "$FAIL_ID" == <-> ]] && print yes)" "yes" "failed spawn allocated one numeric identity"
assert_eq "$(awk -F '\t' -v id="$FAIL_ID" '$4 == id' "$(git -C "$TRIGGER_WT" rev-parse --absolute-git-dir)/ws-sessions")" "" "failed spawn frees its slot row"
assert_eq "$(jq -r '.status' "$WS_HOME/sessions/$FAIL_ID/meta" 2>/dev/null)" "closed" "failed spawn identity is closed"

echo "ws send: input readiness waits up to WS_READY_TIMEOUT seconds"
# ready_probe <polls-until-ready> — runs ws_bus_wait_ready against a stub pane.
ready_probe() {
  ( local after="$1"; source "$WS"
    WS_SESSIONS="$TMP/ready"; mkdir -p "$WS_SESSIONS/1"; print queued > "$WS_SESSIONS/1/inbox"; print 0 > "$TMP/ready-polls"
    ws_bus_pane() { print %0; }
    tmux_() { local n=$(( $(<"$TMP/ready-polls") + 1 )); print $n > "$TMP/ready-polls"; (( n > after )) && print 1 || print 0; }
    ws_bus_wait_ready 1 @0 )
}
ready_probe 55
assert_exit "$?" "0" "TUI ready after >5s of polling succeeds within the default budget"
WS_READY_TIMEOUT=0.3 ready_probe 1000
assert_exit "$?" "1" "WS_READY_TIMEOUT bounds the wait"
assert_eq "$(( $(<"$TMP/ready-polls") < 10 ))" "1" "short WS_READY_TIMEOUT gives up early"

echo "ws add: agent shells resolve a non-interactive ws command"
cat > "$TMP/env-shell" <<'EOF'
#!/bin/zsh -f
print -r -- "$(command -v ws)|${WS_BIN:-}" >> "$WS_HOME/agent-env"
exec zsh -fc "$@"
EOF
chmod +x "$TMP/env-shell"
WS_AGENT_SHELL="$TMP/env-shell" "$WS" add api/triggers --agent claude --detach >/dev/null
# env-shell logs twice: the tmux tab shell (before run's PATH entry), then the agent shell.
agent_env_lines() { (( $(wc -l < "$WS_HOME/agent-env" 2>/dev/null || print 0) >= 2 )); }
WS_TEST_WAIT=5 wait_until agent_env_lines
ENV_LINE="$(sed -n 2p "$WS_HOME/agent-env" 2>/dev/null)"
assert_eq "$([[ "${ENV_LINE%%|*}" -ef "$WS" ]] && print yes)" "yes" "ws on agent PATH resolves to ws.sh"
assert_eq "${ENV_LINE#*|}" "$WS" "agent env exports WS_BIN"

# --- M2 stale exit: resumed sources wait for a new exit ---
echo "ws on: old exits do not satisfy a new watcher on a resumed session"
"$WS" open "$SOURCE_A" --detach >/dev/null
M2_SOURCE_WIN="$(jq -r '.window' "$WS_HOME/sessions/$SOURCE_A/meta")"
assert_contains "$(wait_pane "$M2_SOURCE_WIN" FAKE-claude)" "WHOAMI=$SOURCE_A" "stale-exit source resumes with the same id"
hook api/triggers "$DEFAULT_WIN" working </dev/null
M2_TID="$(WS_SESSION="$DEFAULT_ID" "$WS" on "$SOURCE_A" exit --send-to me 'new exit {id}')"
assert_exit "$?" "0" "resumed source exit watcher registers"
assert_contains "$("$WS" triggers)" "$M2_TID"$'\t'"$SOURCE_A"$'\texit\tsend' "old exit leaves new watcher pending"
assert_eq "$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)" "{}" "registration delivers nothing for an old exit"
hook api/triggers "$DEFAULT_WIN" working </dev/null
T send-keys -t "$M2_SOURCE_WIN" Enter
wait_until lifecycle_exit_count "$SOURCE_A" 2
m2_trigger_finished() {
  [[ -z "$("$WS" triggers | awk -F '\t' -v id="$M2_TID" '$1 == id')" &&
     "$("$WS" sessions "$SOURCE_A" | cut -f5)" == stopped ]]
}
wait_until m2_trigger_finished
assert_exit "$?" "0" "next exit fires the pending watcher"
reply="$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "[ws #$DEFAULT_ID]"$'\n'"new exit $SOURCE_A" "next exit delivers the continuation"
assert_eq "$(hook api/triggers "$DEFAULT_WIN" stop </dev/null)" "{}" "exit watcher delivers once"
# --- end M2 stale exit ---
