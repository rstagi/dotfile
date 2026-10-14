#!/bin/zsh
# Global session registry, lifecycle and message delivery, sourced by ws.sh. Workspace slots remain local; ids never reuse.
# Live state comes from tmux; metadata keeps durable identity and lifecycle status.
WS_SESSIONS="$WS_HOME/sessions"
WS_TRIGGERS="$WS_HOME/triggers"

# Trigger ids are independent of session ids. Records retain fired actions for diagnostics.
cmd_on() {
  if [[ "${1:-}" == --rm ]]; then
    [[ $# == 2 && "$2" == <-> ]] || die "on --rm: expected <trigger-id>"
    [[ -d "$WS_TRIGGERS" ]] || die "on --rm: no such pending trigger: $2"
    with_lock "$WS_TRIGGERS/registry" ws_bus_remove_trigger "$2"
    return $?
  fi
  [[ $# -ge 4 ]] || die "on: expected <ids> <done|exit> (--send-to <id|me> <msg> | --spawn <prompt> [--agent A])"
  local event="$2" target="" sender="" id tid sources action template wt="" agent=claude
  local -a ids=()
  [[ "$event" == done || "$event" == exit ]] || die "on: event must be done or exit"
  case "$3" in
  --send-to)
    [[ $# == 5 ]] || die "on: --send-to expects <id|me> <message>"
    action=send
    target="$(ws_bus_resolve "$4")" || die "on: no such target session: $4"
    template="$5"
    ;;
  --spawn)
    [[ $# == 4 || ( $# == 6 && "$5" == --agent ) ]] || die "on: --spawn expects <prompt> [--agent A]"
    action=spawn
    template="$4"
    [[ $# == 4 ]] || agent="$6"
    (( ${AGENTS[(Ie)$agent]} )) || die "on: unknown agent: $agent"
    ;;
  *) die "on: expected --send-to or --spawn" ;;
  esac
  [[ -n "$template" ]] || die "on: empty message or prompt"
  for id in "${(@s:,:)1}"; do
    id="$(ws_bus_resolve "$id")" || die "on: no such source session"
    (( ${ids[(Ie)$id]} )) || ids+=("$id")
  done
  (( ${#ids} )) || die "on: expected source sessions"
  [[ -z "${WS_SESSION:-}" ]] || sender="$(cmd_whoami)" || return 1
  if [[ "$action" == spawn ]]; then
    if [[ -n "$sender" ]]; then
      wt="$(jq -r '.path' "$WS_SESSIONS/$sender/meta")"
    else
      wt="$(current_workspace)" || die "on: spawn requires a ws worktree"
    fi
    wt="$(resolve_workspace "$wt")" || die "on: spawn workspace no longer exists"
  fi
  sources="$(printf '%s\n' "${ids[@]}" | jq -Rsc 'split("\n")[:-1]')" || return 1
  mkdir -p -m 700 "$WS_TRIGGERS" || return 1
  tid="$(with_lock "$WS_TRIGGERS/registry" ws_bus_register_trigger "$sources" "$event" "$action" "$target" "$template" "$sender" "$wt" "$agent")" || return 1
  print -r -- "$tid"
  # Already-satisfied sources fire now; report this registration's own action failure.
  ws_bus_evaluate_trigger "$tid"
}

# TSV: trigger id, ordered source ids, event, action, target, template.
cmd_triggers() {
  [[ $# == 0 ]] || die "triggers: expected no arguments"
  local record
  for record in "$WS_TRIGGERS"/<->.json(Nn); do
    jq -r 'select(.status == "pending") | [.id, (.sources | join(",")), .event, .action, .target, .template] | @tsv' "$record"
  done
}

cmd_done() {
  local source="" id win
  case "$#:${1:-}" in
  0:) ;;
  1:-) source=- ;;
  2:--output) source="$2" ;;
  *) die "done: expected [--output <file>|-]" ;;
  esac
  id="$(cmd_whoami)" || return 1
  [[ -z "$source" || "$source" == - || ( -f "$source" && -r "$source" ) ]] \
    || die "done: cannot read output file: $source"
  with_lock "$WS_SESSIONS/$id/output" ws_bus_done_locked "$id" "$source" || return 1
  # Emit (and evaluate triggers) only after releasing the output lock.
  ws_bus_event "$id" done "$(jq -nc --arg output "$WS_SESSIONS/$id/output.md" '{output:$output}')" || return 1
  if jq -e '.spawned == true' "$WS_SESSIONS/$id/meta" >/dev/null; then
    win="$(ws_bus_window "$id")"
    if [[ -n "$win" ]]; then
      # Publish first and release the lifecycle lock before killing our own process group.
      # Mark this tab's HUP as cleanup, not a crash: ws_bus_exit skips its exit event.
      ws_bus_update "$id" --arg win "$win" '.done_window = $win'
      # The kill skips the launch wrapper's `ws _end`: free the slot row here.
      session_remove "$(jq -r '.path' "$WS_SESSIONS/$id/meta")" "$(jq -r '.slot' "$WS_SESSIONS/$id/meta")"
      cmd_dismiss "$win"
      tmux_ kill-window -t "$win"
    fi
  fi
}

cmd_show() {
  [[ $# == 1 ]] || die "show: expected <id|name>"
  local id output=""
  id="$(ws_bus_resolve "$1")" || die "no such session: $1"
  [[ ! -f "$WS_SESSIONS/$id/output.md" ]] || output="$WS_SESSIONS/$id/output.md"
  jq -r --arg output "$output" '
    "id: \(.id)\nname: \(.name)\nworkspace: \(.workspace)\nagent: \(.agent)\nstatus: \(.status)\noutput: \($output)"
  ' "$WS_SESSIONS/$id/meta"
}

cmd_send() {
  local force=0
  [[ "${1:-}" != --force ]] || { force=1; shift; }
  if [[ $# == 3 && "$2" == --force ]]; then
    force=1
    set -- "$1" "$3"
  elif [[ $# == 3 && "$3" == --force ]]; then
    force=1
    set -- "$1" "$2"
  fi
  [[ $# == 2 ]] || die "send: expected [--force] <id|name> <message|->"
  local id message="$2" sender=terminal
  id="$(ws_bus_resolve "$1")" || die "no such session: $1"
  if [[ -n "${WS_SESSION:-}" ]]; then
    sender="#$(cmd_whoami)" || return 1
  fi
  if [[ "$message" == - ]]; then
    message="$(cat; print -n .)"
    message="${message%.}"
  fi
  [[ -n "$message" ]] || die "send: empty message"
  message="[ws $sender]"$'\n'"$message"
  local win lock lockfd rc attempt
  # Durable first: a slow TUI or busy hook lock can delay, never drop, the message.
  with_lock "$WS_SESSIONS/$id/inbox" ws_bus_send_enqueue "$id" "$message" "$force" || return 1
  # The window may close while we wait for its hook lock: re-resolve (and resume) once.
  for attempt in 1 2; do
    win="$(ws_bus_window "$id")"
    if [[ -z "$win" ]]; then
      (ws_bus_open "$id" 1) || die "send: message queued; cannot resume session #$id"
      win="$(ws_bus_window "$id")" || die "send: message queued; session #$id did not open"
      ws_bus_wait_ready "$id" "$win" || die "send: message queued; session #$id input not ready"
    elif [[ "$(tmux_ show -wqv -t "$win" @ws_state)" == idle && -z "$(tmux_ show -wqv -t "$win" @ws_main)" ]]; then
      ws_bus_wait_ready "$id" "$win" || die "send: message queued; session #$id input not ready"
    fi
    lock="${TMPDIR:-/tmp}/ws-hook-${WS_TMUX_SOCKET:-default}-$win.lock"
    { : >> "$lock" && zsystem flock -t 3 -i 0.01 -f lockfd "$lock"; } 2>/dev/null ||
      die "send: message queued; session #$id busy"
    with_lock "$WS_SESSIONS/$id/inbox" ws_bus_send_locked "$id" "$win" "$message" 1
    rc=$?
    exec {lockfd}>&-
    (( rc == 3 )) || return "$rc"
  done
  die "send: message queued; session #$id not delivered"
}

cmd_whoami() {
  local id="${WS_SESSION:-}"
  [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" ]] || die "whoami: not inside a ws session"
  print -r -- "$id"
}

cmd_name() {
  [[ $# == 1 ]] || die "name: expected <label>"
  local label="$1" id="$(cmd_whoami)"
  [[ -n "$id" ]] || return 1
  [[ "$label" == [A-Za-z]* && "$label" != *[^A-Za-z0-9_.-]* && "$label" != me ]] \
    || die "name: use a letter followed by letters, digits, dots, underscores or hyphens (me is reserved)"
  with_lock "$WS_SESSIONS/names" ws_bus_name_locked "$id" "$label"
}

# TSV: id, name, workspace, agent, state, status. A missing tab is stopped, not done.
cmd_sessions() {
  [[ $# -le 1 ]] || die "sessions: expected [id|name]"
  local meta id win state selected=""
  [[ $# == 0 ]] || selected="$(ws_bus_resolve "$1")" || die "no such session: $1"
  for meta in "$WS_SESSIONS"/<->/meta(Nn); do
    id="${meta:h:t}"
    [[ -z "$selected" || "$selected" == "$id" ]] || continue
    win="$(ws_bus_window "$id")"
    state=stopped
    if [[ -n "$win" ]]; then
      state="$(tmux_ show -wqv -t "$win" @ws_state 2>/dev/null)"
    fi
    jq -r --arg state "$state" '[.id, .name, .workspace, .agent, $state, .status] | @tsv' "$meta"
  done
}

ws_bus_register_trigger() {
  # Serialize graph validation with registration/removal; concurrent reverse edges lose.
  if [[ "$3" == send ]] && ws_bus_trigger_cycle "$1" "$4"; then
    print -u2 "ws: on: dependency cycle rejected"
    return 1
  fi
  local tid=0 id offset offsets='{}'
  # Snapshot complete event records under each source's append lock. Exits before
  # this registration must not satisfy a new watcher on a resumed session.
  for id in "${(@f)$(print -r -- "$1" | jq -r '.[]')}"; do
    offset="$(with_lock "$WS_SESSIONS/$id/events" jq -s 'length' "$WS_SESSIONS/$id/events")" || return 1
    offsets="$(jq -nc --argjson offsets "$offsets" --arg id "$id" --argjson offset "$offset" \
      '$offsets + {($id):$offset}')" || return 1
  done
  [[ ! -f "$WS_TRIGGERS/counter" ]] || read -r tid < "$WS_TRIGGERS/counter"
  [[ "$tid" == <-> ]] || { print -u2 'ws: on: invalid trigger counter'; return 1; }
  (( tid += 1 ))
  print -r -- "$tid" > "$WS_TRIGGERS/counter.tmp" && mv "$WS_TRIGGERS/counter.tmp" "$WS_TRIGGERS/counter" || return 1
  jq -n --arg id "$tid" --argjson sources "$1" --arg event "$2" --arg action "$3" --arg target "$4" \
    --arg template "$5" --arg sender "$6" --arg wt "$7" --arg agent "$8" --argjson offsets "$offsets" \
    '{id:$id,sources:$sources,event:$event,action:$action,target:$target,template:$template,sender:$sender,
      path:$wt,agent:$agent,status:"pending",event_offsets:$offsets}' \
    > "$WS_TRIGGERS/$tid.json.tmp" && mv "$WS_TRIGGERS/$tid.json.tmp" "$WS_TRIGGERS/$tid.json" || return 1
  print -r -- "$tid"
}

ws_bus_trigger_cycle() {
  local -a records=("$WS_TRIGGERS"/<->.json(N))
  print -n '' | jq -se --argjson sources "$1" --arg target "$2" '
    map(select(.status == "pending" and .action == "send")) as $edges |
    def reaches($node; $seen):
      if ($sources | index($node)) != null then true
      elif ($seen | index($node)) != null then false
      else any($edges[];
        if (.sources | index($node)) != null
        then reaches(.target; $seen + [$node]) else false end)
      end;
    reaches($target; [])' "${records[@]}" >/dev/null
}

ws_bus_remove_trigger() {
  with_lock "$WS_TRIGGERS/$1" ws_bus_remove_trigger_locked "$1"
}

ws_bus_remove_trigger_locked() {
  local record="$WS_TRIGGERS/$1.json"
  [[ -f "$record" && "$(jq -r '.status' "$record")" == pending ]] \
    || { print -u2 "ws: on --rm: no such pending trigger: $1"; return 1; }
  rm -f "$record"
}

# Evaluate only subscribers to this source/event, after releasing the append lock.
ws_bus_evaluate() {
  local record tid
  for record in "$WS_TRIGGERS"/<->.json(Nn); do
    tid="$(jq -r --arg id "$1" --arg event "$2" 'select(.status == "pending" and .event == $event and (.sources | index($id))) | .id' "$record")"
    [[ -z "$tid" ]] || ws_bus_evaluate_trigger "$tid" # failures are recorded; keep going
  done
  return 0
}

ws_bus_evaluate_trigger() {
  with_lock "$WS_TRIGGERS/$1" ws_bus_fire_trigger "$1"
}

ws_bus_fire_trigger() {
  local record="$WS_TRIGGERS/$1.json" id event offset target sender template output="" piece message action rc errfile
  [[ -f "$record" ]] || return 0
  [[ "$(jq -r '.status' "$record")" == pending ]] || return 0
  event="$(jq -r '.event' "$record")"
  local -a ids=("${(@f)$(jq -r '.sources[]' "$record")}")
  for id in "${ids[@]}"; do
    offset=0 # done retains already-recorded completion fan-in; legacy triggers start at zero.
    [[ "$event" != exit ]] || offset="$(jq -r --arg id "$id" '.event_offsets[$id] // 0' "$record")"
    jq -se --arg event "$event" --argjson offset "$offset" 'any(.[$offset:][]; .event == $event)' \
      "$WS_SESSIONS/$id/events" >/dev/null || return 0
  done
  for id in "${ids[@]}"; do
    piece=""
    if [[ -f "$WS_SESSIONS/$id/output.md" ]]; then
      piece="$(cat "$WS_SESSIONS/$id/output.md"; print -n .)"
      piece="${piece%.}"
    fi
    [[ "$id" == "${ids[1]}" ]] || output+=$'\n'
    output+="$piece"
  done
  template="$(jq -rj '.template' "$record"; print -n .)"
  template="${template%.}"
  # Output via stdin: large results must not hit ARG_MAX.
  message="$(print -rn -- "$output" | jq -Rsrj --arg template "$template" --arg ids "${(j:,:)ids}" \
    '. as $output | $template | gsub("\\{(?<key>id|output)\\}"; if .key == "id" then $ids else $output end)'; print -n .)"
  message="${message%.}"
  # An empty expansion is not an action: stay pending, a later output may fill it.
  if [[ -z "${message//[[:space:]]/}" ]]; then
    print -u2 "ws: trigger #$1: empty message or prompt after expansion; left pending"
    return 1
  fi
  target="$(jq -r '.target' "$record")"
  sender="$(jq -r '.sender' "$record")"
  # Claim before any side effect. Repeated and simultaneous events cannot execute twice.
  jq '.status = "fired"' "$record" > "$record.tmp" && mv "$record.tmp" "$record" || return 1
  action="$(jq -r '.action' "$record")"
  errfile="$(mktemp "$WS_TRIGGERS/.error.XXXXXX")" || return 1
  if [[ "$action" == spawn ]]; then
    ws_bus_spawn_trigger "$record" "$message" >/dev/null 2>"$errfile"
  else
    # Registering the trigger is the explicit override for done targets.
    print -rn -- "$message" | (WS_SESSION="$sender" cmd_send --force "$target" - 2>"$errfile") >/dev/null
  fi
  rc=$?
  if (( rc )); then
    jq --argjson rc "$rc" --rawfile error "$errfile" '.status = "failed" | .exit_code = $rc | .error = $error' \
      "$record" > "$record.tmp" && mv "$record.tmp" "$record"
    print -u2 "ws: trigger #$1 action failed: $(tail -1 "$errfile"); see $record"
  fi
  rm -f "$errfile"
  return "$rc"
}

# Allocate through normal workspace slots and mark origin before the agent can run.
ws_bus_spawn_trigger() {
  local record="$1" prompt="$2" wt agent slot id tid sender
  wt="$(jq -r '.path' "$record")"
  wt="$(resolve_workspace "$wt")" || return 1
  agent="$(jq -r '.agent' "$record")"
  tid="$(jq -r '.id' "$record")"
  sender="$(jq -r '.sender' "$record")"
  slot="$(session_add "$wt" "$agent")" || return 1
  id="$(ws_bus_identity "$wt" "$slot" "$agent" "")" || return 1
  ws_bus_update "$id" --arg trigger "$tid" --arg parent "$sender" \
    '.spawned = true | .trigger = $trigger | .parent = $parent' || return 1
  jq --arg child "$id" '.spawned_session = $child' "$record" > "$record.tmp" && mv "$record.tmp" "$record" || return 1
  open_session "$wt" "$slot" "$agent" "" "$prompt" && return 0
  # No tab: free the slot and close the identity rather than orphan a stopped session.
  rm -f "$WS_SESSIONS/$id/prompt"
  session_remove "$wt" "$slot"
  ws_bus_close "$id"
  return 1
}

# Output publication and status are serialized across repeat declarations; cmd_done emits after.
ws_bus_done_locked() {
  local id="$1" source="$2" dir="$WS_SESSIONS/$1" tmp
  tmp="$(mktemp "$dir/.output.XXXXXX")" || return 1
  if [[ -n "$source" ]] && ! cat -- "$source" > "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  if ! with_lock "$dir/inbox" ws_bus_publish_done_locked "$id" "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
}

# Share the enqueue lock: accepted work must be delivered before completion can close a tab.
ws_bus_publish_done_locked() {
  local id="$1" tmp="$2" dir="$WS_SESSIONS/$1"
  local -a inflight=("$dir"/inbox.inflight*(N))
  if [[ -s "$dir/inbox" ]] || (( $#inflight )); then
    print -u2 "ws: done: pending messages; finish this turn so the Stop hook delivers them, then retry ws done"
    return 1
  fi
  mv "$tmp" "$dir/output.md" || return 1
  ws_bus_update "$id" '.status = "done"'
}

# Single lifecycle append boundary: subscribers attach here after the durable JSONL write.
# Event payloads carry id/event/at plus event-specific fields (done: output; exit: code).
ws_bus_event() {
  local id="$1" event="$2" detail="${3:-}" record
  [[ -n "$detail" ]] || detail='{}'
  [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" ]] || return 1
  record="$(jq -nc --arg id "$id" --arg event "$event" --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --argjson detail "$detail" '$detail + {id:$id,event:$event,at:$at}')" || return 1
  with_lock "$WS_SESSIONS/$id/events" ws_bus_event_locked "$id" "$record" || return 1
  ws_bus_evaluate "$id" "$event"
}

ws_bus_event_locked() {
  print -r -- "$2" >> "$WS_SESSIONS/$1/events"
}

# A background child plus wait lets HUP interrupt supervision even if the TUI holds the
# closed tty. Explicit stdin preserves interactive input; only the watcher ignores Ctrl-C.
ws_bus_run_agent() {
  setopt localtraps
  local id="$1" command="$2" code agent_pid input_fd prompt="$WS_SESSIONS/$1/prompt"
  local -a args
  # One-shot initial prompt (open_session writes it): literal positional $1 after `--`.
  if [[ -f "$prompt" ]]; then
    args=(ws "$(cat -- "$prompt"; print x)") # x: keep trailing newlines
    args[2]="${args[2]%x}"
    rm -f "$prompt"
    command+=' -- "$1"'
  fi
  exec {input_fd}<&0
  ${(z)WS_AGENT_SHELL} "$command" "${args[@]}" <&$input_fd &
  agent_pid=$!
  exec {input_fd}<&-
  trap 'trap "" HUP; kill -HUP "$agent_pid" 2>/dev/null; ws_bus_exit "$id" 129 </dev/null >/dev/null 2>&1; exit 129' HUP
  trap '' INT
  wait "$agent_pid"
  code=$?
  # EOF may finish the child just before the tty's HUP arrives. Protect publication,
  # including its subprocesses, rather than restoring the default HUP action early.
  trap '' HUP INT
  # A crashed tab only shows the close prompt: untag it so sends queue and resume instead of
  # pasting there. @ws_state stays for the wrapper's dismiss.
  local win
  if (( code )) && win="$(ws_bus_window "$id")"; then
    tmux_ set -wu -t "$win" @ws_session \; set -wu -t "$win" @ws_main 2>/dev/null
  fi
  ws_bus_exit "$id" "$code" </dev/null

  return "$code"
}

# Called by the launch wrapper immediately after wait, before slot cleanup or crash prompt.
ws_bus_exit() {
  [[ $# == 2 && "$2" == <-> && "$2" -le 255 ]] || die "_exit: expected <id> <exit-code>"
  local detail
  # Spawned `ws done` closed this very tab (its slot cleanup already cleared .window): the
  # done event is the completion. A resumed tab has a new window id and reports exits again.
  [[ ! -f "$WS_SESSIONS/$1/meta" ]] || ! jq -e '.status == "done" and .done_window != null and (.window == .done_window or .window == "")' \
    "$WS_SESSIONS/$1/meta" >/dev/null || return 0
  detail="$(jq -nc --argjson code "$2" '{code:$code}')" || return 1
  ws_bus_event "$1" exit "$detail"
}

# Both send and Stop hold the window's hook lock before the inbox lock: no idle/queue race.
ws_bus_send_locked() {
  local id="$1" win="$2" message="$3" queued="${4:-0}" state
  (( queued )) || ws_bus_enqueue "$id" "$message" || return 1
  # Revalidate under the hook lock: a closed/retagged window or unknown state is not a live
  # busy target. 3 = queued, caller re-resolves (resume) instead of reporting delivery.
  [[ "$(ws_bus_window "$id")" == "$win" ]] || return 3
  state="$(tmux_ show -wqv -t "$win" @ws_state)"
  [[ -n "$state" ]] || return 3
  if [[ "$state" == idle && -s "$WS_SESSIONS/$id/inbox" ]]; then
    if [[ -z "$(tmux_ show -wqv -t "$win" @ws_main)" ]]; then
      ws_bus_wait_ready "$id" "$win" || { print -u2 "ws: send: message queued; session #$id input not ready"; return 1; }
    fi
    # Order the submitted turn against hooks that entered before this paste. Capture
    # before input, so the resulting UserPromptSubmit remains newer than this watermark.
    zmodload zsh/datetime
    local seq="$(realtime_us "$EPOCHREALTIME")"
    ws_bus_idle_locked "$id" "$win" || { print -u2 "ws: send: message queued; session #$id agent pane unavailable"; return 1; }
    tmux_ set -w -t "$win" @ws_seq "$seq" \; set -w -t "$win" @ws_main working \; set -w -t "$win" @ws_state working
  fi
}

# Lifecycle check shares the inbox lock with done's status write: no enqueue after done.
ws_bus_send_enqueue() {
  local id="$1" message="$2" force="$3" lifecycle
  lifecycle="$(jq -r '.status' "$WS_SESSIONS/$id/meta")" || return 1
  if [[ "$lifecycle" == done ]]; then
    print -u2 "ws: session #$id is done; --force required to send"
    (( force )) || return 1
  fi
  ws_bus_enqueue "$id" "$message"
}

# Called under the inbox lock. Message via stdin: large payloads must not hit ARG_MAX.
ws_bus_enqueue() {
  ws_bus_reclaim "$1" || return 1
  print -rn -- "$2" | jq -Rsc '{message:.}' >> "$WS_SESSIONS/$1/inbox"
}

# Under the inbox lock: a Stop killed before its reply landed left inbox.inflight; requeue it first.
ws_bus_reclaim() {
  local inbox="$WS_SESSIONS/$1/inbox"
  [[ -e "$inbox.inflight" ]] || return 0
  cat "$inbox.inflight" "$inbox" > "$inbox.tmp" && mv "$inbox.tmp" "$inbox" && rm -f "$inbox.inflight"
}

# tmux 3.8 exposes the TUI's bracketed-paste mode. A freshly resumed tab starts in a
# shell: never paste there before the agent enables input. Stop may already have consumed
# the queued message during startup, which also counts as delivery.
# Budget: WS_READY_TIMEOUT seconds (default 60): resumed agents with MCPs start slowly.
ws_bus_wait_ready() {
  local id="$1" win="$2" ready pane deadline
  zmodload zsh/datetime
  deadline=$(( EPOCHREALTIME + ${WS_READY_TIMEOUT:-60} ))
  while :; do
    [[ -s "$WS_SESSIONS/$id/inbox" ]] || return 0
    pane="$(ws_bus_pane "$win")" || return 1
    ready="$(tmux_ display -p -t "$pane" '#{bracket_paste_flag}' 2>/dev/null)" || return 1
    [[ "$ready" != 1 ]] || return 0
    (( EPOCHREALTIME < deadline )) || return 1
    sleep 0.1
  done
}

# Continuation Stops (stop_hook_active) block too, but only for messages queued since the
# last reply: an empty inbox allows stopping. Inbox lock -t 1 + hook lock -t 2 stay under
# the 5s hook timeout.
ws_bus_stop() {
  local id="$1" inbox="$WS_SESSIONS/$1/inbox"
  [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" ]] || { print '{}'; return 0; }
  : >> "$inbox.lock"
  (zsystem flock -t 1 -i 0.01 "$inbox.lock" 2>/dev/null || exit 1; ws_bus_stop_locked "$id")
}

# Drained messages stay in inbox.inflight.<hook pid> until the hook has written the reply
# and acked (at-least-once): a hook killed before that leaves an orphan for the next drain.
ws_bus_stop_locked() {
  local inbox="$WS_SESSIONS/$1/inbox" inflight="$WS_SESSIONS/$1/inbox.inflight.$$"
  ws_bus_reclaim_orphans "$1" || return 1
  [[ -s "$inbox" ]] || { print '{}'; return 0; }
  mv "$inbox" "$inflight" && : > "$inbox" || return 1
  jq -cs '{decision:"block",reason:(map(.message) | join("\n\n"))}' "$inflight"
}

# cmd_hook calls this once the Stop reply reached stdout. Own file only, so no lock:
# reclaim never touches a live owner's file.
ws_bus_ack() {
  [[ "$1" == <-> ]] || return 0
  rm -f "$WS_SESSIONS/$1/inbox.inflight.$$"
}

# Under the inbox lock: requeues the legacy in-flight file and those of dead Stop hooks.
# A live owner's reply may still land, so its file stays; our own pid's is a reused one.
ws_bus_reclaim_orphans() {
  local inbox="$WS_SESSIONS/$1/inbox" f
  ws_bus_reclaim "$1" || return 1
  for f in "$inbox".inflight.<->(N); do
    [[ "${f##*.}" != $$ ]] && kill -0 "${f##*.}" 2>/dev/null && continue
    cat "$f" "$inbox" > "$inbox.tmp" && mv "$inbox.tmp" "$inbox" && rm -f "$f" || return 1
  done
}

# Fallback for cancellation or legacy hooks (never a Stop: its TUI is blocked in the hook).
# Only a verified idle tab may receive pane input; callers hold its hook lock while
# checking/draining the inbox.
ws_bus_idle() {
  local id="$1" win="$2"
  local -a orphans
  [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" ]] || return 1
  orphans=("$WS_SESSIONS/$id"/inbox.inflight*(N))
  [[ -s "$WS_SESSIONS/$id/inbox" ]] || (( $#orphans )) || return 1
  with_lock "$WS_SESSIONS/$id/inbox" ws_bus_idle_locked "$id" "$win"
}

ws_bus_idle_locked() {
  local id="$1" win="$2" inbox="$WS_SESSIONS/$1/inbox" message
  ws_bus_reclaim_orphans "$id" || return 1
  [[ -s "$inbox" ]] || return 1
  message="$(jq -rjs 'map(.message) | join("\n\n")' "$inbox" && print -n .)" || return 1
  message="${message%.}"
  ws_bus_paste "$id" "$win" "$message" || return 1
  : > "$inbox"
}

# Named buffers isolate concurrent senders; raw paste preserves LF inside bracketed input.
ws_bus_paste() {
  local id="$1" win="$2" message="$3" buffer="ws-$1-$$" pane
  pane="$(ws_bus_pane "$win")" || return 1
  print -rn -- "$message" | tmux_ load-buffer -b "$buffer" - || return 1
  if ! tmux_ paste-buffer -d -p -r -b "$buffer" -t "$pane"; then
    tmux_ delete-buffer -b "$buffer" 2>/dev/null
    return 1
  fi
  sleep 0.3
  tmux_ send-keys -t "$pane" Enter
}

# The agent's own pane (@ws_pane), never the active one: a user may split the tab with a
# shell. Fails if it left the window; untracked legacy tabs qualify only when unsplit.
ws_bus_pane() {
  local win="$1" pane panes
  pane="$(tmux_ show -wqv -t "$win" @ws_pane)"
  panes="$(tmux_ list-panes -t "$win" -F '#{pane_id}' 2>/dev/null)" || return 1
  if [[ -z "$pane" ]]; then
    [[ "$panes" != *$'\n'* ]] || return 1
    pane="$panes"
  fi
  [[ -n "$pane" && $'\n'"$panes"$'\n' == *$'\n'"$pane"$'\n'* ]] || return 1
  print -r -- "$pane"
}

ws_bus_name_locked() {
  local id="$1" label="$2" existing
  existing="$(ws_bus_resolve "$label")"
  [[ -z "$existing" || "$existing" == "$id" ]] || { print -u2 "ws: name already in use: $label"; return 1; }
  ws_bus_update "$id" --arg name "$label" '.name = $name'
}

ws_bus_resolve() {
  local target="$1" meta id
  [[ "$target" != me ]] || target="${WS_SESSION:-}"
  target="${target#\#}"
  if [[ "$target" == <-> ]]; then
    [[ -f "$WS_SESSIONS/$target/meta" ]] || return 1
    print -r -- "$target"
    return 0
  fi
  [[ -n "$target" ]] || return 1
  for meta in "$WS_SESSIONS"/<->/meta(N); do
    id="$(jq -r --arg name "$target" 'select(.name == $name) | .id' "$meta")"
    [[ -z "$id" ]] || { print -r -- "$id"; return 0; }
  done
  return 1
}

ws_bus_window() {
  local id="$1" win
  win="$(jq -r '.window // empty' "$WS_SESSIONS/$id/meta")"
  [[ -n "$win" && "$(tmux_ display -p -t "$win" '#{@ws_session}' 2>/dev/null)" == "$id" ]] || return 1
  print -r -- "$win"
}

ws_bus_open() {
  local id="$1" detach="$2" win wt slot agent resume
  win="$(ws_bus_window "$id")"
  if [[ -z "$win" ]]; then
    wt="$(jq -r '.path' "$WS_SESSIONS/$id/meta")"
    wt="$(resolve_workspace "$wt")" || die "session workspace no longer exists: $id"
    slot="$(ws_bus_restore_slot "$wt" "$id")" || return 1
    agent="$(jq -r '.agent' "$WS_SESSIONS/$id/meta")"
    resume="$(jq -r '.resume_id' "$WS_SESSIONS/$id/meta")"
    win="$(open_session "$wt" "$slot" "$agent" "$resume")" || return 1
  fi
  (( detach )) || focus "$win"
  return 0
}

# Reattach a retained identity to a free workspace slot; the old slot may have been reused.
ws_bus_restore_slot() {
  local f
  f="$(sessions_file "$1")" || return 1
  with_lock "$f" ws_bus_restore_locked "$f" "$2"
}

ws_bus_restore_locked() {
  local f="$1" id="$2" slot existing agent resume
  touch "$f" || return 1
  existing="$(awk -F '\t' -v id="$id" '$4 == id {print $1; exit}' "$f")"
  [[ -z "$existing" ]] || { print -r -- "$existing"; return 0; }
  slot="$(jq -r '.slot' "$WS_SESSIONS/$id/meta")"
  if [[ "$slot" != <-> || -n "$(awk -F '\t' -v s="$slot" '$1 == s {print $1; exit}' "$f")" ]]; then
    slot="$( { cut -f1 "$f"; print 0; } | sort -n | tail -1)"
    (( slot += 1 ))
  fi
  agent="$(jq -r '.agent' "$WS_SESSIONS/$id/meta")"
  resume="$(jq -r '.resume_id' "$WS_SESSIONS/$id/meta")"
  printf '%s\t%s\t%s\t%s\n' "$slot" "$agent" "$resume" "$id" >> "$f" || return 1
  ws_bus_update "$id" --arg slot "$slot" '.slot = $slot' || return 1
  print -r -- "$slot"
}

# Called under the workspace sessions-file lock, then the global counter lock.
ws_bus_allocate() {
  mkdir -p -m 700 "$WS_SESSIONS" || return 1
  with_lock "$WS_SESSIONS/counter" ws_bus_allocate_locked "$@"
}

ws_bus_allocate_locked() {
  local wt="$1" slot="$2" agent="$3" resume="${4:-}" id=0 dir
  [[ ! -f "$WS_SESSIONS/counter" ]] || read -r id < "$WS_SESSIONS/counter"
  [[ "$id" == <-> ]] || { print -u2 'ws: invalid session counter'; return 1; }
  (( id += 1 ))
  print -r -- "$id" > "$WS_SESSIONS/counter.tmp" && mv "$WS_SESSIONS/counter.tmp" "$WS_SESSIONS/counter" || return 1
  dir="$WS_SESSIONS/$id"
  mkdir -m 700 "$dir" || return 1
  jq -n --arg id "$id" --arg wt "$wt" --arg workspace "$(workspace_name "$wt")" \
    --arg agent "$agent" --arg slot "$slot" --arg resume "$resume" \
    '{id:$id, name:"", workspace:$workspace, path:$wt, slot:$slot, agent:$agent,
      resume_id:$resume, window:"", status:"active", spawned:false}' > "$dir/meta" || return 1
  touch "$dir/inbox" "$dir/events"
  print -r -- "$id"
}

# Upgrade a legacy three-column workspace row only once, before launching its tab.
ws_bus_identity() {
  local f
  f="$(sessions_file "$1")" || return 1
  with_lock "$f" ws_bus_identity_locked "$f" "$@"
}

ws_bus_identity_locked() {
  local f="$1" wt="$2" slot="$3" agent="$4" resume="$5" id
  id="$(awk -F '\t' -v s="$slot" '$1 == s {print $4}' "$f")"
  if [[ "$id" != <-> || ! -f "$WS_SESSIONS/$id/meta" ]]; then
    id="$(ws_bus_allocate "$wt" "$slot" "$agent" "$resume")" || return 1
    rewrite "$f" -F '\t' -v OFS='\t' -v s="$slot" -v id="$id" '$1 == s {$4 = id} {print}' || return 1
  fi
  print -r -- "$id"
}

# Serialize atomic JSON updates independently of slot bookkeeping and hook locks.
ws_bus_update() {
  local id="$1"
  shift
  [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" ]] || return 1
  with_lock "$WS_SESSIONS/$id/meta" ws_bus_update_locked "$WS_SESSIONS/$id/meta" "$@"
}

ws_bus_update_locked() {
  local meta="$1"
  shift
  jq "$@" "$meta" > "$meta.tmp" && mv "$meta.tmp" "$meta"
}

ws_bus_close() {
  ws_bus_update "$1" '.window = "" | if .status == "active" then .status = "closed" else . end'
}
