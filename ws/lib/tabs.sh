#!/bin/zsh
# Agent tabs: open/resume sessions, per-agent command building, slot registry and tab lookup. Sourced by ws.sh.

# Opens one agent tab in the worktree's tmux session (creating the session if needed) and
# prints its window id. The tab closes with the agent; a clean exit (0) forgets the session,
# anything else (crash, SIGHUP) keeps it for resume.
open_session() {
  local wt="$1" slot="$2" agent="$3" resume="$4"
  local id
  id="$(ws_bus_identity "$wt" "$slot" "$agent" "$resume")" || return 1
  with_lock "$WS_SESSIONS/$id/open" open_session_locked "$wt" "$slot" "$agent" "$resume" "$id" "${5:-}"
}

open_session_locked() {
  local wt="$1" slot="$2" agent="$3" resume="$4" id="$5"
  local name tab cmd run sess win
  if win="$(ws_bus_window "$id")"; then
    print -r -- "$win"
    return 0
  fi
  name="$(workspace_name "$wt")"
  tab="$agent"
  (( slot > 1 )) && tab+="#$slot"
  cmd="$(agent_command "$agent" "$resume" "" "$id")"
  # Prompt via file: tmux rejects commands over ~16KB. `_agent` consumes it once.
  if [[ -n "${6:-}" ]]; then
    (umask 077 && print -rn -- "$6" > "$WS_SESSIONS/$id/prompt") || return 1
  else
    rm -f "$WS_SESSIONS/$id/prompt"
  fi
  # Non-interactive agent shells (codex/grok `zsh -lc`) lack the `ws` alias: put a `ws` on PATH.
  [[ "$WS_HOME/bin/ws" -ef "$WS_BIN" ]] || { mkdir -p "$WS_HOME/bin" && ln -sfn "$WS_BIN" "$WS_HOME/bin/ws"; } 2>/dev/null
  run="PATH=${(q)WS_HOME}/bin:\$PATH; ${(q)WS_BIN} _agent $id ${(q)cmd}; s=\$?; ${(q)WS_BIN} _dismiss \"\$TMUX_PANE\"; if (( s == 0 )); then ${(q)WS_BIN} _end ${(q)wt} $slot $id \"\$TMUX_PANE\"; else print \"agent exited (\$s) — enter to close\"; read; fi"
  local -a spawn=(-d -P -F '#{window_id}' -n "$tab" -c "$wt" -e "WS_WORKSPACE=$name" -e "WS_SESSION=$id"
    -e "WS_HOME=$WS_HOME" -e "WS_ROOT=$WS_ROOT" -e "WS_TMUX_SOCKET=${WS_TMUX_SOCKET:-}" -e "WS_BIN=$WS_BIN"
    -e "WS_AGENT_SHELL=$WS_AGENT_SHELL" ${(z)WS_AGENT_SHELL} "$run")
  sess="$(sessions_for "$wt" | head -1)"
  if [[ -n "$sess" ]]; then
    win="$(tmux_ new-window -t "$sess:" "${spawn[@]}")" || return 1
  else
    win="$(tmux_ new-session -s "${name//[.:]/_}" "${spawn[@]}")" || return 1
    configure_bindings
    sess="$(tmux_ display -p -t "$win" '#{session_id}')"
    tmux_ set -t "$sess" @ws_path "$wt"
    tmux_ set -t "$sess" detach-on-destroy off
    tmux_ set -t "$sess" status-left-length 60
    tmux_ set -t "$sess" status-left "#[bold] $name #[default]"
  fi
  tag_window "$win" "$wt" "$slot" "$agent" || return 1
  print -r -- "$win"
}

cmd_add() {
  local target="" agent="claude" detach=0
  while [[ $# -gt 0 ]]; do
    case "$1" in
    --agent) agent="$2"; shift 2 ;;
    --detach) detach=1; shift ;;
    *) target="$1"; shift ;;
    esac
  done
  check_agent "$agent"
  [[ -n "$target" ]] || target="$(current_workspace)" || die "add: not inside a workspace (pass <repo/branch>)"
  local wt win slot
  wt="$(resolve_workspace "$target")" || die "no such workspace: $target"
  slot="$(session_add "$wt" "$agent")" || die "cannot register session"
  win="$(open_session "$wt" "$slot" "$agent" "")" || die "cannot open session"
  (( detach )) || focus "$win"
}

cmd_open() {
  local target="" detach=0
  while [[ $# -gt 0 ]]; do
    case "$1" in
    --detach) detach=1; shift ;;
    *) target="$1"; shift ;;
    esac
  done
  local wt id row
  if id="$(ws_bus_resolve "$target")"; then
    ws_bus_open "$id" "$detach"
    return $?
  fi
  wt="$(resolve_workspace "$target")" || die "no such workspace: $target"
  if [[ -z "$(windows_for "$wt")" ]]; then
    [[ -n "$(sessions_read "$wt")" ]] || session_add "$wt" claude >/dev/null
    sessions_read "$wt" | while IFS= read -r row; do
      local -a fields=("${(@ps:\t:)row}")
      open_session "$wt" "${fields[1]}" "${fields[2]}" "${fields[3]:-}" >/dev/null
    done
  fi
  (( detach )) || focus "$(active_window_for "$wt")"
}

# Runs an agent in the current tab (one opened by hand, e.g. prefix+c) as a tracked session:
# ws hooks, state, resume. The tab goes back to a plain shell when the agent exits.
cmd_run() {
  local agent="$1" wt win slot cmd s id
  shift
  [[ -n "${TMUX_PANE:-}" ]] || die "$agent: run it inside a workspace's tmux tab"
  wt="$(current_workspace)" && wt="$(resolve_workspace "$wt")" || die "$agent: not inside a workspace"
  win="$(tmux_ display -p -t "$TMUX_PANE" '#{window_id}')"
  [[ -z "$(tmux_ show -wqv -t "$win" @ws_slot)" ]] || die "$agent: this tab already runs a ws session"
  slot="$(session_add "$wt" "$agent")" || die "cannot register session"
  id="$(ws_bus_identity "$wt" "$slot" "$agent" "")" || die "cannot register session"
  tag_window "$win" "$wt" "$slot" "$agent"
  cmd="$(agent_command "$agent" "" "" "$id")"
  (( $# )) && cmd+=" ${(j: :)${(q)@}}"
  WS_SESSION="$id" WS_WORKSPACE="$(workspace_name "$wt")" ws_bus_run_agent "$id" "$cmd"
  s=$?
  cmd_dismiss "$TMUX_PANE"
  # Generation-scoped (as `_end`): a resumed ID's new window keeps its row and registry window.
  (( s == 0 )) && session_remove "$wt" "$slot" "$id" "$win" # else kept for resume, as for a killed tab
  untag_window "$win"
  return $s
}

# Prompt goes after `--` so option-like or subcommand-named prompts stay positional.
agent_command() {
  local agent="$1" resume="$2" prompt="$3" id="$4"
  local preamble="You are ws session #$id. Use the ws-sessions skill to coordinate sessions."
  # done is permanent: only spawned sessions own their completion; user tabs outlive tasks.
  [[ "$(jq -r '.spawned' "$WS_SESSIONS/$id/meta" 2>/dev/null)" != true ]] ||
    preamble="You are ws session #$id; run \`ws done\` when the user's task is complete. Use the ws-sessions skill to coordinate sessions."
  case "$agent" in
  claude)
    write_claude_settings
    local cmd="claude --dangerously-skip-permissions --settings ${(q)WS_HOME}/claude-settings.json"
    cmd+=" --append-system-prompt ${(q)preamble}"
    [[ -n "$resume" ]] && cmd+=" --resume ${(q)resume}"
    [[ -z "$prompt" ]] || cmd+=" -- ${(q)prompt}"
    print -r -- "$cmd"
    ;;
  codex)
    # Hook-trust bypass: ws hooks are passed per launch, so they'd otherwise need re-trusting.
    local cmd="codex" ev st
    [[ -n "$resume" ]] && cmd+=" resume ${(q)resume}"
    cmd+=" --dangerously-bypass-approvals-and-sandbox --dangerously-bypass-hook-trust"
    local instructions="developer_instructions=$(jq -rn --arg text "$preamble" '$text | @json')"
    cmd+=" -c ${(q)instructions}"
    local hook
    for ev st in UserPromptSubmit working PreToolUse working PostToolUse working PermissionRequest waiting Stop stop Interrupt cancel; do
      hook="hooks.$ev=[{hooks=[{type=\"command\",command=$(jq -rn --arg c "${(q)WS_BIN} hook $st" '$c | @json')}]}]"
      cmd+=" -c ${(q)hook}"
    done
    [[ -z "$prompt" ]] || cmd+=" -- ${(q)prompt}"
    print -r -- "$cmd"
    ;;
  grok)
    write_grok_hooks
    local cmd="grok --always-approve"
    cmd+=" --rules ${(q)preamble}"
    [[ -n "$resume" ]] && cmd+=" --resume ${(q)resume}"
    [[ -z "$prompt" ]] || cmd+=" -- ${(q)prompt}"
    print -r -- "$cmd"
    ;;
  esac
}

sessions_file() { print -r -- "$(git -C "$1" rev-parse --absolute-git-dir)/ws-sessions"; }

sessions_read() {
  local f
  f="$(sessions_file "$1")"
  [[ -f "$f" ]] && cat "$f"
  return 0
}

# Appends a session on the next free slot and prints the slot.
session_add() {
  local wt="$1" agent="$2" f
  f="$(sessions_file "$wt")"
  with_lock "$f" session_append "$f" "$agent" "$wt"
}

session_append() {
  local f="$1" agent="$2" wt="$3" slot id
  slot="$( { [[ -f "$f" ]] && cut -f1 "$f"; print 0; } | sort -n | tail -1)"
  slot=$((slot + 1))
  id="$(ws_bus_allocate "$wt" "$slot" "$agent")" || return 1
  printf '%s\t%s\t\t%s\n' "$slot" "$agent" "$id" >> "$f"
  print -r -- "$slot"
}

# session_remove <wt> <slot> [<id> <window|pane>] — with an exiting generation (launch
# wrapper `_end`), a slot re-pointed meanwhile (`ws open` resumed the ID elsewhere, or
# the slot was reused) is left to its new owner.
session_remove() {
  local wt="$1" slot="$2" gid="${3:-}" win="${4:-}" f
  f="$(sessions_file "$wt")" || return 0
  [[ -f "$f" ]] || return 0
  [[ "$win" != %* ]] || win="$(tmux_ display -p -t "$win" '#{window_id}' 2>/dev/null)"
  with_lock "$f" session_remove_locked "$f" "$slot" "$gid" "$win"
}

session_remove_locked() {
  local f="$1" slot="$2" gid="$3" win="$4" id
  id="$(awk -F '\t' -v s="$slot" '$1 == s {print $4}' "$f")"
  if [[ -n "$gid" ]]; then
    [[ -z "$id" || "$id" == "$gid" ]] || return 0
    if [[ "$gid" == <-> && -n "$win" ]]; then
      ws_bus_close "$gid" "$win"
      (( $? != 3 )) || return 0
    fi
  fi
  rewrite "$f" -F '\t' -v s="$slot" '$1 != s' || return 1
  [[ "$id" != <-> || ( -n "$win" && "$id" == "$gid" ) ]] || ws_bus_close "$id"
  return 0
}

# Stores the tab's resume id, re-adding its row if it went missing (a session must stay
# resumable after a reboot). Skips the write when nothing changed — hooks fire on every tool.
# slot/id are read before the lock: a stale hook after slot reuse must never take over the
# row of the slot's new owner, so only an unowned row or our own is updated.
record_resume_id() {
  local sid wt slot agent f id meta t="${2:-3}" # t: lock wait (s); contention skips the update
  sid="$(print -r -- "$1" | jq -r '.session_id // empty' 2>/dev/null)"
  [[ -n "$sid" ]] || sid="${GROK_SESSION_ID:-}" # grok hooks: id in env if not in the payload
  [[ -n "$sid" ]] || return 0
  wt="$(tmux_ display -p -t "$TMUX_PANE" '#{@ws_path}' 2>/dev/null)"
  slot="$(tmux_ display -p -t "$TMUX_PANE" '#{@ws_slot}' 2>/dev/null)"
  agent="$(tmux_ display -p -t "$TMUX_PANE" '#{@ws_agent}' 2>/dev/null)"
  id="$(tmux_ display -p -t "$TMUX_PANE" '#{@ws_session}' 2>/dev/null)"
  [[ -n "$wt" && -n "$slot" ]] || return 0
  f="$(sessions_file "$wt")" || return 0
  [[ -f "$f" ]] || return 0
  meta="$WS_SESSIONS/$id/meta"
  if [[ "$id" == <-> && -f "$meta" && "$(jq -r '.resume_id' "$meta")" != "$sid" ]]; then
    : >> "$meta.lock"
    (zsystem flock -t "$t" -i 0.01 "$meta.lock" 2>/dev/null &&
      ws_bus_update_locked "$meta" --arg resume "$sid" '.resume_id = $resume') || return 0
  fi
  grep -qxF -- "$slot	${agent:-claude}	$sid	$id" "$f" && return 0
  : >> "$f.lock"
  (zsystem flock -t "$t" -i 0.01 "$f.lock" 2>/dev/null || exit 0
    rewrite "$f" -F '\t' -v OFS='\t' -v s="$slot" -v a="${agent:-claude}" -v id="$sid" -v g="$id" \
      '$1 == s { found = 1; if ($4 == "" || $4 == g) { $3 = id; if (g != "") $4 = g } } { print } END { if (!found) print s, a, id, g }')
}

# Agent tabs of a worktree, in any session (so manually moved tabs are still found).
windows_for() {
  tmux_ list-windows -a -F '#{window_id}	#{@ws_path}	#{@ws_slot}' 2>/dev/null \
    | awk -F '\t' -v p="$1" '$2 == p && $3 != "" { print $1 }'
}

# The tab you last used in a worktree: active in its tmux session, else the first.
active_window_for() {
  tmux_ list-windows -a -F '#{window_id}	#{@ws_path}	#{@ws_slot}	#{window_active}' 2>/dev/null \
    | awk -F '\t' -v p="$1" '$2 == p && $3 != "" { if (!first) first = $1; if ($4 == 1) { print $1; found = 1; exit } }
      END { if (!found && first) print first }'
}

sessions_for() {
  tmux_ list-sessions -F '#{session_id}	#{@ws_path}' 2>/dev/null \
    | awk -F '\t' -v p="$1" '$2 == p { print $1 }'
}
