#!/bin/zsh
# Agent hooks: `ws hook` ingest, ownership checks and the claude/grok hook config writers. Sourced by ws.sh.

# Agent hooks call this from inside a session window: sets the window's state and records
# the agent's session id (for resume). Best effort, always exits 0. Stop may request a
# continuation with queued messages; all other hook output stays silent.
# seq (µs since epoch, taken at entry) orders hooks per tab: one older than the tab's @ws_seq
# is dropped, and a per-window lock keeps overlapping hooks from both applying/emitting
# (no lock → dropped: an unordered write could clobber a newer state).
# Cancellation also requires the recorded turn id: late reports have newer entry times.
cmd_hook() {
  zmodload zsh/datetime zsh/system 2>/dev/null
  local seq="$(realtime_us "$EPOCHREALTIME")" event="${1:-}" payload=""
  [[ -t 0 ]] || payload="$(cat)"
  local hook_event="$(print -r -- "$payload" | jq -r '.hook_event_name // .hookEventName // empty' 2>/dev/null)"
  local turn_id="$(print -r -- "$payload" | jq -r '.promptId // .prompt_id // .turn_id // empty | select(type == "string")' 2>/dev/null)"
  # Accept older launches that wired Stop to idle, as well as the explicit Stop command.
  if [[ "$event" == idle ]]; then
    case "$hook_event" in
    Stop|stop) event=stop ;;
    StopCancelled|stop_cancelled|Interrupt) event=cancel ;;
    esac
  fi
  # Grok parses but ignores Stop decisions when its channel closes: preserve queued work.
  if [[ "$event" == stop && "$(print -r -- "$payload" | jq -r '.reason // empty' 2>/dev/null)" == (channel_closed|shutdown) ]]; then
    print '{}'
    return 0
  fi
  local reply=""
  [[ "$event" != stop ]] || reply='{}'
  [[ -n "${WS_WORKSPACE:-}" && -n "${TMUX_PANE:-}" ]] || { [[ -z "$reply" ]] || print -r -- "$reply"; return 0; }
  [[ "$event" == (working|idle|stop|cancel|waiting|subagent-start|subagent-stop) ]] || return 0
  # A reused pane now hosts another session: a delayed hook from the old launch must not
  # touch the new one's resume id, state or inbox.
  if ! hook_owns_pane || nested_agent_hook; then
    [[ -z "$reply" ]] || print -r -- "$reply"
    return 0
  fi
  # Stop shares a 5s hook timeout with the hook and inbox locks: never wait on resume ids.
  record_resume_id "$payload" "$([[ "$event" == stop ]] && print 0 || print 3)"
  local win lock lockfd prev last state agent_id="" applied=0
  [[ -n "$payload" ]] && agent_id="$(print -r -- "$payload" | jq -r '.agent_id // empty' 2>/dev/null)"
  agent_id="${agent_id//[^A-Za-z0-9_-]/_}"
  win="$(tmux_ display -p -t "$TMUX_PANE" '#{window_id}' 2>/dev/null)" || { [[ -z "$reply" ]] || print -r -- "$reply"; return 0; }
  lock="${TMPDIR:-/tmp}/ws-hook-${WS_TMUX_SOCKET:-default}-$win.lock"
  # Stop waits longest (3s hook lock + 1s inbox lock stay under the 5s hook timeout) and, if
  # starved, hands settling the tab to a detached retry rather than dropping the event.
  local lock_wait=2
  [[ "$event" != stop ]] || lock_wait=3
  { : >>"$lock" && zsystem flock -t "$lock_wait" -i 0.01 -f lockfd "$lock"; } 2>/dev/null || {
    [[ "$event" != stop ]] || ws_bus_detach "$WS_BIN" _hook-retry "$(tmux_ show -wqv -t "$TMUX_PANE" @ws_seq 2>/dev/null)"
    [[ -z "$reply" ]] || print -r -- "$reply"
    return 0
  }
  IFS='|' read -r last prev <<<"$(tmux_ display -p -t "$TMUX_PANE" '#{@ws_seq}|#{@ws_state}' 2>/dev/null)"
  # A tool hook may overtake a delayed prompt hook's state update. Delivery ack only
  # compares against the paste watermark; stale state must not discard that confirmation.
  if [[ "$event" == working && -z "$agent_id" && "$hook_event" == (UserPromptSubmit|user_prompt_submit) ]]; then
    local id="$(tmux_ display -p -t "$win" '#{@ws_session}')"
    id="${id:-${WS_SESSION:-}}"
    [[ "$id" != <-> ]] || with_lock "$WS_SESSIONS/$id/inbox" ws_bus_paste_ack "$id" "$seq"
  fi
  if [[ "$last" != <-> ]] || (( last < seq )); then
    # Cancel reports can start after the next prompt: the entry-time watermark alone
    # cannot order them. Without a matching turn id, neither state nor input is safe.
    if [[ "$event" == cancel ]]; then
      if [[ -z "$turn_id" || "$turn_id" != "$(tmux_ show -wqv -t "$win" @ws_turn)" ]]; then
        exec {lockfd}>&-
        return 0
      fi
      event=idle
    elif [[ "$event" == working && -z "$agent_id" ]]; then
      # Tool hooks may omit ids; a new prompt without one invalidates the old turn.
      if [[ -n "$turn_id" || "$hook_event" == (UserPromptSubmit|user_prompt_submit) ]]; then
        tmux_ set -w -t "$win" @ws_turn "$turn_id" 2>/dev/null
      fi
    fi
    if [[ "$event" == stop ]] && ! hook_owns_pane; then # exit cleanup/resume raced the lock
      exec {lockfd}>&-
      print -r -- '{}'
      return 0
    elif [[ "$event" == stop ]]; then
      local id="$(tmux_ display -p -t "$win" '#{@ws_session}')"
      id="${id:-${WS_SESSION:-}}" # early hooks fire before new-window tags @ws_session
      reply="$(ws_bus_stop "$id")" || reply='{}'
      if [[ "$reply" == *'"decision":"block"'* ]]; then
        state="$(hook_state working "" "$prev")"
      else
        state="$(hook_state idle "" "$prev")"
      fi
    else
      state="$(hook_state "$event" "$agent_id" "$prev")"
    fi
    # Never paste for a Stop: its TUI is blocked in this hook. Later sends see the idle tab.
    if [[ "$event" != stop && "$state" == idle ]]; then
      local id="$(tmux_ display -p -t "$win" '#{@ws_session}')"
      id="${id:-${WS_SESSION:-}}" # early hooks fire before new-window tags @ws_session
      if ws_bus_idle "$id" "$win"; then
        seq="$(tmux_ show -wqv -t "$win" @ws_seq)" # paste watermark; state awaits UserPromptSubmit
      fi
    fi
    tmux_ set -w -t "$TMUX_PANE" @ws_seq "$seq" \; set -w -t "$TMUX_PANE" @ws_state "$state" 2>/dev/null
    applied=1
  elif [[ "$event" == subagent-* ]]; then
    hook_state "$event" "$agent_id" "$prev" >/dev/null # stale for state, but keep subagent bookkeeping
  fi
  exec {lockfd}>&-
  if [[ -n "$reply" ]] && print -r -- "$reply" && [[ "$reply" == *'"decision":"block"'* ]]; then
    ws_bus_ack "${id:-}" # reply delivered: drop its in-flight copy
  fi
  (( applied )) || return 0
  # finished while you weren't looking → unread until you visit the tab (or ws opens it)
  [[ "$prev" == working && "$state" == idle ]] && ! pane_focused && tmux_ set -w -t "$TMUX_PANE" @ws_unread 1 2>/dev/null
  [[ "$state" != "$prev" ]] && emit_notify_event "$state" "$prev" "$payload" "$seq"
  return 0
}

# Detached retry for a Stop that never got the hook lock: the TUI is no longer blocked, so
# settle the tab as idle (draining its inbox by paste) unless a newer hook already moved it.
hook_retry() {
  local seq0="${1:-}" i
  for i in {1..15}; do
    sleep 1
    [[ "$(tmux_ show -wqv -t "$TMUX_PANE" @ws_seq 2>/dev/null)" == "$seq0" &&
      "$(tmux_ show -wqv -t "$TMUX_PANE" @ws_state 2>/dev/null)" == working ]] || return 0
    print -r -- '{"hook_event_name":"idle"}' | cmd_hook idle >/dev/null 2>&1
  done
}

# A pane whose @ws_session moved on (reused tab), or was untagged by exit cleanup while the
# registry points elsewhere (resumed/closed), no longer belongs to this launch's WS_SESSION.
# Only a pre-tag hook whose registry window is this very pane may fall back to WS_SESSION.
hook_owns_pane() {
  [[ -n "${WS_SESSION:-}" ]] || return 0
  local pane_session="$(tmux_ display -p -t "$TMUX_PANE" '#{@ws_session}' 2>/dev/null)" win agent_pane
  if [[ -n "$pane_session" ]]; then
    [[ "$pane_session" == "$WS_SESSION" ]] || return 1
    # WS_SESSION leaks into split panes via the tmux session env: only the tab's agent pane
    # (and, by sharing its TMUX_PANE, that agent's children) acts for the tab.
    agent_pane="$(tmux_ show -wqv -t "$TMUX_PANE" @ws_pane 2>/dev/null)"
    [[ -z "$agent_pane" || "$agent_pane" == "$TMUX_PANE" ]]
    return
  fi
  win="$(tmux_ display -p -t "$TMUX_PANE" '#{window_id}' 2>/dev/null)" || return 1
  [[ -n "$win" && "$(jq -r '.window // ""' "$WS_SESSIONS/$WS_SESSION/meta" 2>/dev/null)" == "$win" ]]
}

# Nested agents (e.g. headless grok run from the tab agent's tool shell) inherit TMUX_PANE/
# WS_* and global hooks: their hooks must not act for the tab. Walking the hook's ancestry
# up to the pane, a run of same-named agent processes is one agent (node → native codex);
# two agent runs mean a nested agent. Hooks from outside the pane's tree are not judged.
nested_agent_hook() {
  local pane_pid pid=$$ name prev="" runs=0 line
  local -A parent args
  pane_pid="$(tmux_ display -p -t "$TMUX_PANE" '#{pane_pid}' 2>/dev/null)"
  [[ "$pane_pid" == <-> ]] || return 1
  while read -r line; do
    line="${line## #}"
    local -a f=(${=line})
    (( ${#f} >= 3 )) || continue
    parent[$f[1]]="$f[2]"
    args[$f[1]]="${(j: :)f[3,-1]}"
  done < <(ps -Ao pid=,ppid=,args= 2>/dev/null)
  while [[ -n "${parent[$pid]:-}" && "$pid" != "$pane_pid" && "$pid" -gt 1 ]]; do
    pid="${parent[$pid]}"
    name="$(agent_process_name "${args[$pid]:-}")"
    if [[ -n "$name" && "$name" != "$prev" ]]; then
      (( ++runs ))
    fi
    prev="$name"
  done
  [[ "$pid" == "$pane_pid" ]] && (( runs > 1 ))
}

# agent_process_name <args> — claude|codex|grok when the process is that agent (also via an
# interpreter: `node …/codex`, `sh …/claude`), else empty.
agent_process_name() {
  local -a w=(${=1})
  local exe="${w[1]:-}" name="${${w[1]:-}:t}"
  if [[ "$name" == (node|bun|deno|sh|bash|zsh|python3) && -n "${w[2]:-}" && "${w[2]}" != -* ]]; then
    exe="${w[2]}"; name="${w[2]:t}"
  fi
  if [[ "$name" == (claude|codex|grok)(|-*) ]]; then
    print -r -- "${name%%-*}"
  elif [[ "$exe" == *(claude-code|@openai/codex)/* ]]; then
    [[ "$exe" == *claude-code/* ]] && print claude || print codex
  fi
}

# hook_state <event> <agent_id> <prev> — the tab's new state. The main agent's turn can end
# (Stop) while background subagents still run: the tab stays working until the last one
# stops. Each running subagent is its own window option (@ws_sub_<id>), so parallel hooks
# never race on a shared list.
hook_state() {
  local event="$1" id="$2" prev="$3"
  case "$event" in
  subagent-start)
    [[ -n "$id" ]] && tmux_ set -w -t "$TMUX_PANE" "@ws_sub_$id" 1 2>/dev/null
    print working
    ;;
  subagent-stop)
    [[ -n "$id" ]] && tmux_ set -wu -t "$TMUX_PANE" "@ws_sub_$id" 2>/dev/null
    if ! subagents_running && [[ "$(tmux_ show -wqv -t "$TMUX_PANE" @ws_main)" == idle ]]; then
      print idle
    else
      print -r -- "${prev:-working}"
    fi
    ;;
  idle)
    tmux_ set -wu -t "$TMUX_PANE" @ws_turn 2>/dev/null # ended before any fallback starts a new turn
    tmux_ set -w -t "$TMUX_PANE" @ws_main idle 2>/dev/null
    if subagents_running; then print working; else print idle; fi
    ;;
  working)
    [[ -z "$id" ]] && tmux_ set -w -t "$TMUX_PANE" @ws_main working 2>/dev/null # not a subagent's tool
    print working
    ;;
  *) print -r -- "$event" ;;
  esac
}

subagents_running() { tmux_ show -w -t "$TMUX_PANE" 2>/dev/null | grep -q '^@ws_sub_'; }

# Per-launch settings (claude --settings) so ws hooks never leak into non-ws sessions.
# No SessionStart: its id has no conversation yet, and `claude --resume <id>` fails on it.
write_claude_settings() {
  mkdir -p "$WS_HOME"
  jq -n --arg h "${(q)WS_BIN} hook" '
    def run($s): [{hooks: [{type: "command", command: "\($h) \($s)", timeout: 5}]}];
    {hooks: {
      UserPromptSubmit: run("working"),
      PreToolUse: run("working"),
      PostToolUse: run("working"),
      Stop: run("stop"),
      SubagentStart: run("subagent-start"),
      SubagentStop: run("subagent-stop"),
      PermissionRequest: run("waiting"),
      Notification: [{matcher: "permission_prompt|elicitation_dialog",
                      hooks: [{type: "command", command: "\($h) waiting", timeout: 5}]}]
    }}' > "$WS_HOME/claude-settings.json"
}

# Grok has no per-launch hooks flag, so ws hooks live in its global hooks dir (always
# trusted there). Harmless outside ws: without WS_WORKSPACE, cmd_hook only answers Stop with {}.
write_grok_hooks() {
  # Global hooks fire in every Grok session: never wire them to a missing ws.
  [[ -f "$WS_BIN" && -x "$WS_BIN" ]] || { print -u2 "ws: grok hooks: WS_BIN not executable: $WS_BIN"; return 1; }
  mkdir -p "${GROK_HOOKS:h}"
  jq -n --arg h "${(q)WS_BIN} hook" '
    def cmd($s): {type: "command", command: "\($h) \($s)", timeout: 5};
    def run($s): [{hooks: [cmd($s)]}];
    {hooks: {
      UserPromptSubmit: run("working"),
      PreToolUse: run("working"),
      PostToolUse: run("working"),
      Stop: run("stop"),
      StopCancelled: run("cancel"),
      PermissionRequest: run("waiting"),
      Notification: [{matcher: "permission_prompt", hooks: [cmd("waiting")]}]
    }}' > "$GROK_HOOKS"
}

# Re-points the Claude settings file and the global Grok hooks at the current WS_BIN after ws
# moved (both embed its absolute path). Touches only files that already exist, and writes
# nothing when they already name WS_BIN, so it is cheap enough to run on every startup.
hooks_migrate() {
  local needle
  needle="$(jq -rn --arg h "${(q)WS_BIN} hook " '$h | @json')"
  needle="${needle[1,-2]}" # opening quote kept: a longer path ending in WS_BIN must not match
  [[ ! -f "$WS_HOME/claude-settings.json" ]] || grep -qF -- "$needle" "$WS_HOME/claude-settings.json" || write_claude_settings
  [[ ! -f "$GROK_HOOKS" ]] || grep -qF -- "$needle" "$GROK_HOOKS" || write_grok_hooks
  return 0
}
