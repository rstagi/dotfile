#!/bin/zsh
# Global session registry, sourced by ws.sh. Workspace slots remain local; ids never reuse.
WS_SESSIONS="$WS_HOME/sessions"

cmd_whoami() {
  local id="${WS_SESSION:-}"
  [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" ]] || die "whoami: not inside a ws session"
  print -r -- "$id"
}

# TSV: id, name, workspace, agent, state, status. A missing tab is stopped, not done.
cmd_sessions() {
  local meta id win state
  for meta in "$WS_SESSIONS"/<->/meta(N); do
    id="${meta:h:t}"
    win="$(jq -r '.window // empty' "$meta")"
    state=stopped
    if [[ -n "$win" && "$(tmux_ display -p -t "$win" '#{@ws_session}' 2>/dev/null)" == "$id" ]]; then
      state="$(tmux_ show -wqv -t "$win" @ws_state 2>/dev/null)"
    fi
    jq -r --arg state "$state" '[.id, .name, .workspace, .agent, $state, .status] | @tsv' "$meta"
  done
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
      resume_id:$resume, window:"", state:"stopped", status:"active", spawned:false}' > "$dir/meta" || return 1
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
