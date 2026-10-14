#!/bin/zsh
# Session triggers: done/exit events, one-shot fan-in `ws on` registry and evaluation. Sourced by ws/ws.sh.
WS_TRIGGERS="$WS_HOME/triggers"

# Trigger ids are independent of session ids. Terminal records are archived for diagnostics.
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
  local required_text="${template//\{output\}/}"
  [[ -n "${required_text//[[:space:]]/}" ]] \
    || die "on: message or prompt may be empty; include text or {id} alongside {output}"
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
  local -a records=("$WS_TRIGGERS"/<->.json(Nn))
  (( ${#records} )) || return 0
  ws_bus_jq_records -r 'select(.status == "pending") | [.id, (.sources | join(",")), .event, .action, .target, .template] | @tsv' \
    -- "${records[@]}"
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
  # Mark a spawned tab's HUP as cleanup first, not a crash: ws_bus_exit skips its exit event.
  # Publish before killing our own process group. Triggers evaluate in a detached worker so a
  # killed tool call can neither stall sibling fan-in nor leave this tab undone.
  if jq -e '.spawned == true' "$WS_SESSIONS/$id/meta" >/dev/null; then
    win="$(ws_bus_window "$id")"
    [[ -z "$win" ]] || ws_bus_update "$id" --arg win "$win" '.done_window = $win'
  fi
  ws_bus_event "$id" done "$(jq -nc --arg output "$WS_SESSIONS/$id/output.md" '{output:$output}')" defer || return 1
  if [[ -n "${win:-}" ]]; then
    # The kill skips the launch wrapper's `ws _end`: free the slot row here.
    session_remove "$(jq -r '.path' "$WS_SESSIONS/$id/meta")" "$(jq -r '.slot' "$WS_SESSIONS/$id/meta")"
    cmd_dismiss "$win"
    tmux_ kill-window -t "$win"
  fi
}

ws_bus_register_trigger() {
  # Serialize graph validation with registration/removal; concurrent reverse edges lose.
  local cycle_rc=0
  if [[ "$3" == send ]]; then
    ws_bus_trigger_cycle "$1" "$4" "$2" || cycle_rc=$?
    if (( cycle_rc != 1 )); then
      if (( cycle_rc == 0 )); then
        print -u2 "ws: on: dependency cycle rejected"
      else
        print -u2 "ws: on: cannot validate dependency cycles; registration rejected"
      fi
      return 1
    fi
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

# 0 = cycle, 1 = acyclic, else unvalidated. Corrupt records are warned and skipped. An exit
# self-edge is a one-shot crash handler (its session has already exited), never a cycle.
ws_bus_trigger_cycle() {
  local edges=""
  local -a records=("$WS_TRIGGERS"/<->.json(N))
  (( ! ${#records} )) || edges="$(ws_bus_jq_records -c 'select(.status == "pending" and .action == "send") |
    if (.sources | type) == "array" and all(.sources[]; type == "string") and (.target | type) == "string"
    then {sources: (if .event == "exit" then .sources - [.target] else .sources end), target}
    else error("bad trigger schema") end' -- "${records[@]}")" || return 2
  print -rn -- "$edges" | jq -se --argjson sources "$1" --arg target "$2" --arg event "${3:-}" '
    . as $edges |
    (if $event == "exit" then $sources - [$target] else $sources end) as $sources |
    def reaches($node; $seen):
      if ($sources | index($node)) != null then true
      elif ($seen | index($node)) != null then false
      else any($edges[];
        if (.sources | index($node)) != null
        then reaches(.target; $seen + [$node]) else false end)
      end;
    ($sources | length) > 0 and reaches($target; [])' >/dev/null
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

# List only subscribers to this source/event; ws_bus_evaluate_run fires them lock-free.
# Legacy fired/failed records retire only if unlocked: a held lock is a live action.
ws_bus_evaluate_list() {
  local -a records=("$WS_TRIGGERS"/<->.json(Nn))
  (( ${#records} )) || return 0
  ws_bus_jq_records -r --arg id "$1" --arg event "$2" '
    if .status == "fired" or .status == "failed" then "retire\t\(.id)"
    elif .status == "pending" and .event == $event and (.sources | index($id)) then "fire\t\(.id)"
    else empty end
  ' -- "${records[@]}"
}

ws_bus_evaluate_run() {
  local line
  [[ -n "$1" ]] || return 0
  for line in "${(@f)1}"; do
    case "$line" in # failures are recorded; keep going
      fire$'\t'*) ws_bus_evaluate_trigger "${line#*$'\t'}" ;;
      retire$'\t'*) ws_bus_retire_trigger "${line#*$'\t'}" ;;
    esac
  done
  return 0
}

# Parse many JSON records with one jq; if any is unparseable or schema-bad, fall back to
# one jq per file so a bad record is warned about and skipped, never fatal to the rest.
# Usage: ws_bus_jq_records <jq args...> -- <files...>. Missing files are skipped.
ws_bus_jq_records() {
  local -a args=()
  local out f
  while (( $# )) && [[ "$1" != -- ]]; do args+=("$1"); shift; done
  shift
  if out="$( { cat "$@" 2>/dev/null || true; } | jq "${args[@]}" 2>/dev/null)"; then
    [[ -z "$out" ]] || print -r -- "$out"
    return 0
  fi
  for f in "$@"; do
    [[ -f "$f" ]] || continue
    if out="$(jq "${args[@]}" "$f" 2>/dev/null)"; then
      [[ -z "$out" ]] || print -r -- "$out"
    elif [[ -f "$f" ]]; then
      print -u2 "ws: skipping unreadable record: $f"
    fi
  done
  return 0
}

ws_bus_evaluate_trigger() {
  with_lock "$WS_TRIGGERS/$1" ws_bus_fire_trigger "$1"
}

# Non-blocking: the firing event holds this lock for its whole action; retry next event.
ws_bus_retire_trigger() {
  : >> "$WS_TRIGGERS/$1.lock"
  (zsystem flock -t 0 "$WS_TRIGGERS/$1.lock" 2>/dev/null || exit 0; ws_bus_fire_trigger "$1")
}

ws_bus_fire_trigger() {
  local record="$WS_TRIGGERS/$1.json" id event offset target sender template output="" piece message action rc errfile
  [[ -f "$record" ]] || return 0
  case "$(jq -r '.status' "$record")" in
    pending) ;;
    fired|failed) ws_bus_archive_trigger "$1"; return $? ;; # retire legacy records
    *) return 0 ;;
  esac
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
  # An empty expansion is not an action: fail and archive rather than re-erroring forever.
  if [[ -z "${message//[[:space:]]/}" ]]; then
    jq '.status = "failed" | .error = "empty message or prompt after expansion"' "$record" > "$record.tmp" &&
      mv "$record.tmp" "$record" || return 1
    print -u2 "ws: trigger #$1: empty message or prompt after expansion; see $WS_TRIGGERS/archive/$1.json"
    ws_bus_archive_trigger "$1"
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
  if [[ "$action" != spawn ]] && (( rc == 75 )); then
    # Durably queued, delivered by a later Stop/idle: not a failure, and must not be re-sent.
    jq --rawfile warning "$errfile" '.status = "queued" | .warning = $warning' \
      "$record" > "$record.tmp" && mv "$record.tmp" "$record"
    print -u2 "ws: trigger #$1 message queued, not yet delivered: $(tail -1 "$errfile"); see $WS_TRIGGERS/archive/$1.json"
    rc=0
  elif (( rc )); then
    jq --argjson rc "$rc" --rawfile error "$errfile" '.status = "failed" | .exit_code = $rc | .error = $error' \
      "$record" > "$record.tmp" && mv "$record.tmp" "$record"
    print -u2 "ws: trigger #$1 action failed: $(tail -1 "$errfile"); see $WS_TRIGGERS/archive/$1.json"
  fi
  rm -f "$errfile"
  ws_bus_archive_trigger "$1" || return 1
  return "$rc"
}

# Called under the trigger lock, after the action has finished updating its record.
ws_bus_archive_trigger() {
  mkdir -p -m 700 "$WS_TRIGGERS/archive" &&
    mv "$WS_TRIGGERS/$1.json" "$WS_TRIGGERS/archive/$1.json"
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
  ws_bus_reclaim_orphans "$id" || return 1 # dead/expired Stop copies must not block forever
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
  local id="$1" event="$2" detail="${3:-}" defer="${4:-}" record tids
  [[ -n "$detail" ]] || detail='{}'
  mkdir -p -m 700 "$WS_TRIGGERS" 2>/dev/null # before `ws on`, the registry lock needs its directory
  [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" ]] || return 1
  record="$(jq -nc --arg id "$id" --arg event "$event" --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --argjson detail "$detail" '$detail + {id:$id,event:$event,at:$at}')" || return 1
  # Registry then events lock: a concurrent registration either snapshots an offset past
  # this event or is written before we list subscribers. Actions run after both release.
  if ! tids="$(with_lock "$WS_TRIGGERS/registry" ws_bus_event_registry_locked "$id" "$record" "$event")"; then
    # Registry lock unavailable: never lose the event itself.
    with_lock "$WS_SESSIONS/$id/events" ws_bus_event_locked "$id" "$record" || return 1
    tids="$(ws_bus_evaluate_list "$id" "$event")"
  fi
  if [[ "$defer" == defer ]]; then # the worker re-lists: records are idempotent while pending
    [[ -z "$tids" ]] || ws_bus_detach "$WS_BIN" _evaluate "$id" "$event"
    return 0
  fi
  # The event is published: subscriber failures are reported, never the event's status.
  ws_bus_evaluate_run "$tids" || print -u2 "ws: event #$id $event: trigger evaluation failed"
  return 0
}

# Detached worker for a deferred event: fire whatever subscribers are pending now.
ws_bus_evaluate_event() {
  ws_bus_evaluate_run "$(ws_bus_evaluate_list "$1" "$2")"
}

# Fails only if the append failed; the subscriber listing is best-effort.
ws_bus_event_registry_locked() {
  with_lock "$WS_SESSIONS/$1/events" ws_bus_event_locked "$1" "$2" || return 1
  ws_bus_evaluate_list "$1" "$3"
  return 0
}

ws_bus_event_locked() {
  print -r -- "$2" >> "$WS_SESSIONS/$1/events"
}
