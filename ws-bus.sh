#!/bin/zsh
# Global session registry, sourced by ws.sh. Workspace slots remain local; ids never reuse.
# Live state comes from tmux; metadata keeps durable identity and lifecycle status.
WS_SESSIONS="$WS_HOME/sessions"

cmd_send() {
  [[ $# == 2 ]] || die "send: expected <id|name> <message|->"
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
  local win lock lockfd
  win="$(ws_bus_window "$id")" || die "send: session is stopped: $id"
  lock="${TMPDIR:-/tmp}/ws-hook-${WS_TMUX_SOCKET:-default}-$win.lock"
  { : >> "$lock" && zsystem flock -t 3 -i 0.01 -f lockfd "$lock"; } 2>/dev/null || return 1
  with_lock "$WS_SESSIONS/$id/inbox" ws_bus_send_locked "$id" "$win" "[ws $sender]"$'\n'"$message"
  local rc=$?
  exec {lockfd}>&-
  return "$rc"
}

# Both send and Stop hold the window's hook lock before the inbox lock: no idle/queue race.
ws_bus_send_locked() {
  local id="$1" win="$2" message="$3" state
  state="$(tmux_ show -wqv -t "$win" @ws_state)"
  jq -nc --arg message "$message" '{message:$message}' >> "$WS_SESSIONS/$id/inbox" || return 1
  if [[ "$state" == idle ]]; then
    ws_bus_idle_locked "$id" "$win" || return 1
    tmux_ set -w -t "$win" @ws_main working \; set -w -t "$win" @ws_state working
  fi
}

# A Stop continuation is one-shot. The next Stop with an empty inbox must allow stopping.
ws_bus_stop() {
  local id="$1" payload="$2"
  [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" ]] || { print '{}'; return 0; }
  if print -r -- "$payload" | jq -e '.stop_hook_active == true or .stopHookActive == true' >/dev/null 2>&1; then
    print '{}'
    return 0
  fi
  with_lock "$WS_SESSIONS/$id/inbox" ws_bus_stop_locked "$id"
}

ws_bus_stop_locked() {
  local inbox="$WS_SESSIONS/$1/inbox" reply
  [[ -s "$inbox" ]] || { print '{}'; return 0; }
  reply="$(jq -cs '{decision:"block",reason:(map(.message) | join("\n\n"))}' "$inbox")" || return 1
  : > "$inbox" || return 1
  print -r -- "$reply"
}

# Fallback for cancellation, legacy hooks or an active Stop guard. Only a verified idle
# tab may receive pane input; callers hold its hook lock while checking/draining the inbox.
ws_bus_idle() {
  local id="$1" win="$2"
  [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" && -s "$WS_SESSIONS/$id/inbox" ]] || return 1
  with_lock "$WS_SESSIONS/$id/inbox" ws_bus_idle_locked "$id" "$win"
}

ws_bus_idle_locked() {
  local id="$1" win="$2" inbox="$WS_SESSIONS/$1/inbox" message
  [[ -s "$inbox" ]] || return 1
  message="$(jq -rjs 'map(.message) | join("\n\n")' "$inbox" && print -n .)" || return 1
  message="${message%.}"
  ws_bus_paste "$id" "$win" "$message" || return 1
  : > "$inbox"
}

# Named buffers isolate concurrent senders; raw paste preserves LF inside bracketed input.
ws_bus_paste() {
  local id="$1" win="$2" message="$3" buffer="ws-$1-$$" pane
  pane="$(tmux_ display -p -t "$win" '#{pane_id}')" || return 1
  print -rn -- "$message" | tmux_ load-buffer -b "$buffer" - || return 1
  if ! tmux_ paste-buffer -d -p -r -b "$buffer" -t "$pane"; then
    tmux_ delete-buffer -b "$buffer" 2>/dev/null
    return 1
  fi
  sleep 0.3
  tmux_ send-keys -t "$pane" Enter
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

ws_bus_name_locked() {
  local id="$1" label="$2" existing
  existing="$(ws_bus_resolve "$label")"
  [[ -z "$existing" || "$existing" == "$id" ]] || { print -u2 "ws: name already in use: $label"; return 1; }
  ws_bus_update "$id" --arg name "$label" '.name = $name'
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
  print -r -- "$id" > "$WS_SESSIONS/counter" || return 1
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
