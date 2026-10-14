#!/bin/zsh
# Session bus: global registry/identity, send, inbox, delivery and retry, agent run/exit lifecycle. Sourced by ws/ws.sh.
# Workspace slots remain local; ids never reuse. Live state comes from tmux; metadata keeps durable identity and lifecycle status.
WS_SESSIONS="$WS_HOME/sessions"

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
  local id message="$2" sender=terminal nl=$'\n'
  id="$(ws_bus_resolve "$1")" || die "no such session: $1"
  if [[ -n "${WS_SESSION:-}" ]]; then
    sender="#$(cmd_whoami)" || return 1
  fi
  if [[ "$message" == - ]]; then
    message="$(cat; print -n .)"
    message="${message%.}"
  fi
  [[ -n "$message" ]] || die "send: empty message"
  # Body lines must not forge a sender header (Stop joins batches): indent `[ws ` lines.
  [[ "$message" != '[ws '* ]] || message=" $message"
  message="${message//"${nl}[ws "/"${nl} [ws "}"
  message="[ws $sender]"$'\n'"$message"
  # 75 (EX_TEMPFAIL): durably queued, not yet delivered. A later Stop/idle or the bounded
  # background retry delivers it: never re-send.
  local win lock lockfd rc attempt queued=75
  # Durable first: a slow TUI or busy hook lock can delay, never drop, the message.
  with_lock "$WS_SESSIONS/$id/inbox" ws_bus_send_enqueue "$id" "$message" "$force" || return 1
  # The window may close while we wait for its hook lock: re-resolve (and resume) once.
  for attempt in 1 2; do
    win="$(ws_bus_window "$id")"
    if [[ -z "$win" ]]; then
      if ! (ws_bus_open "$id" 1); then
        # A vanished workspace can never resume: a genuine failure, not a pending delivery.
        resolve_workspace "$(jq -r '.path' "$WS_SESSIONS/$id/meta")" >/dev/null ||
          die "send: message queued; session #$id cannot resume: workspace no longer exists"
        ws_bus_queued_die "$id" "send: message queued; cannot resume session #$id"
      fi
      win="$(ws_bus_window "$id")" || ws_bus_queued_die "$id" "send: message queued; session #$id did not open"
      ws_bus_wait_ready "$id" "$win" || ws_bus_queued_die "$id" "send: message queued; session #$id input not ready"
    elif [[ "$(tmux_ show -wqv -t "$win" @ws_state)" == idle && -z "$(tmux_ show -wqv -t "$win" @ws_main)" ]]; then
      ws_bus_wait_ready "$id" "$win" || ws_bus_queued_die "$id" "send: message queued; session #$id input not ready"
    fi
    lock="${TMPDIR:-/tmp}/ws-hook-${WS_TMUX_SOCKET:-default}-$win.lock"
    { : >> "$lock" && zsystem flock -t 3 -i 0.01 -f lockfd "$lock"; } 2>/dev/null ||
      ws_bus_queued_die "$id" "send: message queued; session #$id busy"
    with_lock "$WS_SESSIONS/$id/inbox" ws_bus_send_locked "$id" "$win" "$message" 1
    rc=$?
    exec {lockfd}>&-
    if (( rc == 4 )); then
      # Readiness changed after the unlocked check: release both locks before waiting.
      ws_bus_wait_ready "$id" "$win" || ws_bus_queued_die "$id" "send: message queued; session #$id input not ready"
    elif (( rc != 3 )); then
      (( rc == 0 )) || { ws_bus_deliver_later "$id"; return "$queued"; } # pane unavailable or inbox lock timeout: still queued
      return 0
    fi
  done
  ws_bus_queued_die "$id" "send: message queued; session #$id not delivered"
}

# 75 after enqueue: a bounded detached retry covers an idle target that fires no hook.
ws_bus_queued_die() {
  local id="$1"
  shift
  ws_bus_deliver_later "$id"
  die "$1" 75
}

ws_bus_deliver_later() {
  ws_bus_detach "$WS_BIN" _deliver-retry "$1"
}

# Retry delivery of a queued inbox while the target sits idle. Working targets drain through
# their Stop hook; a gone window or empty inbox ends it. Bounded: retries x interval.
ws_bus_deliver_retry() {
  local id="$1" tries="${WS_DELIVER_RETRIES:-12}" i win lock lockfd rc
  for (( i = 0; i < tries; i++ )); do
    sleep "${WS_DELIVER_RETRY_INTERVAL:-5}"
    [[ -s "$WS_SESSIONS/$id/inbox" ]] || return 0
    win="$(ws_bus_window "$id")"
    [[ -n "$win" ]] || return 0
    [[ "$(tmux_ show -wqv -t "$win" @ws_state)" == idle ]] || continue
    lock="${TMPDIR:-/tmp}/ws-hook-${WS_TMUX_SOCKET:-default}-$win.lock"
    { : >> "$lock" && zsystem flock -t 3 -i 0.01 -f lockfd "$lock"; } 2>/dev/null || continue
    with_lock "$WS_SESSIONS/$id/inbox" ws_bus_send_locked "$id" "$win" "" 1
    rc=$?
    exec {lockfd}>&-
    (( rc == 3 || rc == 4 )) || return 0
  done
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

cmd_show() {
  [[ $# == 1 ]] || die "show: expected <id|name>"
  local id output=""
  id="$(ws_bus_resolve "$1")" || die "no such session: $1"
  [[ ! -f "$WS_SESSIONS/$id/output.md" ]] || output="$WS_SESSIONS/$id/output.md"
  jq -r --arg output "$output" '
    "id: \(.id)\nname: \(.name)\nworkspace: \(.workspace)\nagent: \(.agent)\nstatus: \(.status)\noutput: \($output)"
  ' "$WS_SESSIONS/$id/meta"
}

# Run a command in its own session: it survives the caller's tab being killed.
ws_bus_detach() {
  if command -v perl >/dev/null 2>&1; then
    perl -MPOSIX -e 'POSIX::setsid(); exec @ARGV' -- "$@" </dev/null >/dev/null 2>&1 &!
  else
    "$@" </dev/null >/dev/null 2>&1 &!
  fi
}

# A background child plus wait lets HUP interrupt supervision even if the TUI holds the
# closed tty. Explicit stdin preserves interactive input. Reset async zsh's ignored
# INT/QUIT in the child; the watcher alone ignores Ctrl-C while waiting for its exit code.
ws_bus_run_agent() {
  setopt localtraps
  local id="$1" command="$2" code agent_pid input_fd win="" prompt="$WS_SESSIONS/$1/prompt"
  local -a args
  # Capture the launch generation, never a later window resumed under this identity.
  [[ -z "${TMUX_PANE:-}" ]] || win="$(tmux_ display -p -t "$TMUX_PANE" '#{window_id}' 2>/dev/null)"
  # One-shot initial prompt (open_session writes it): literal positional $1 after `--`.
  if [[ -f "$prompt" ]]; then
    args=(ws "$(cat -- "$prompt"; print x)") # x: keep trailing newlines
    args[2]="${args[2]%x}"
    rm -f "$prompt"
    command+=' -- "$1"'
  fi
  exec {input_fd}<&0
  { trap - INT QUIT; exec ${(z)WS_AGENT_SHELL} "$command" "${args[@]}"; } <&$input_fd &
  agent_pid=$!
  exec {input_fd}<&-
  trap 'trap "" HUP; kill -HUP "$agent_pid" 2>/dev/null; ws_bus_untag_agent "$id" "$win"; ws_bus_exit "$id" 129 </dev/null >/dev/null 2>&1; exit 129' HUP
  trap '' INT
  wait "$agent_pid"
  code=$?
  # EOF may finish the child just before the tty's HUP arrives. Protect publication,
  # including its subprocesses, rather than restoring the default HUP action early.
  trap '' HUP INT
  # Exit actions may wait for slow targets: stop delivery before publishing any exit.
  # @ws_state stays for the wrapper's dismiss.
  ws_bus_untag_agent "$id" "$win"
  # tmux serializes commands: a paste admitted before the untag is already in the tty.
  # Discard it so the tab's shell never reads it; non-canonical so partial lines go too.
  # Bounded, so a continuous stream cannot stall the exit. Foreground only: an interactive
  # agent shell may keep the tty's process group, and touching it would stop this wrapper.
  if [[ -t 0 ]]; then
    local tty_mode junk drained=0 tpgid pgid
    read -r tpgid pgid <<<"$(ps -o tpgid= -o pgid= -p $$ 2>/dev/null)"
    if [[ -n "$tpgid" && "$tpgid" == "$pgid" ]] && tty_mode="$(stty -g 2>/dev/null)" && stty -icanon min 0 time 0 2>/dev/null; then
      while (( drained++ < 65536 )) && read -r -k 1 -u 0 -t 0.05 junk 2>/dev/null; do :; done
      stty "$tty_mode" 2>/dev/null
    fi
  fi
  ws_bus_exit "$id" "$code" </dev/null

  return "$code"
}

# Only clear this wrapper's window while it still belongs to its session. Evaluate and
# untag together in tmux so a retagged window cannot be cleared between separate calls.
ws_bus_untag_agent() {
  local id="$1" win="$2"
  [[ -n "$win" ]] || return 0
  tmux_ if-shell -F -t "$win" "#{==:#{@ws_session},$id}" \
    "set -wu -t $win @ws_session; set -wu -t $win @ws_main" 2>/dev/null
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
  local id="$1" win="$2" message="$3" queued="${4:-0}" state pane
  (( queued )) || ws_bus_enqueue "$id" "$message" || return 1
  # Revalidate under the hook lock: a closed/retagged window or unknown state is not a live
  # busy target. 3 = queued, caller re-resolves (resume) instead of reporting delivery.
  [[ "$(ws_bus_window "$id")" == "$win" ]] || return 3
  state="$(tmux_ show -wqv -t "$win" @ws_state)"
  [[ -n "$state" ]] || return 3
  if [[ "$state" == idle && -s "$WS_SESSIONS/$id/inbox" ]]; then
    if [[ -z "$(tmux_ show -wqv -t "$win" @ws_main)" ]]; then
      # One nonblocking check under lock; 4 asks cmd_send to wait unlocked and retry.
      pane="$(ws_bus_pane "$win")" || return 4
      [[ "$(tmux_ display -p -t "$pane" '#{bracket_paste_flag}' 2>/dev/null)" == 1 ]] || return 4
    fi
    [[ ! -e "$WS_SESSIONS/$id/inbox.inflight.paste" ]] || return 0 # awaiting prompt acknowledgement
    ws_bus_idle_locked "$id" "$win" || { print -u2 "ws: send: message queued; session #$id agent pane unavailable"; return 1; }
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
  ws_bus_paste_expire "$1" || return 1
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
  (zsystem flock -t 1 -i 0.01 "$inbox.lock" 2>/dev/null || { print -u2 "ws: timed out waiting for lock: $inbox.lock"; exit 1; }; ws_bus_stop_locked "$id")
}

# Drained messages stay in inbox.inflight.<hook pid> until the hook has written the reply
# and acked (at-least-once): a hook killed before that leaves an orphan for the next drain.
ws_bus_stop_locked() {
  local inbox="$WS_SESSIONS/$1/inbox" inflight="$WS_SESSIONS/$1/inbox.inflight.$$"
  ws_bus_reclaim_orphans "$1" || return 1
  [[ -s "$inbox" ]] || { print '{}'; return 0; }
  # Fresh mtime: the orphan expiry measures this drain, not the last enqueue.
  mv "$inbox" "$inflight" && touch "$inflight" && : > "$inbox" || return 1
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
# Past 2x the 5s hook timeout no owner is alive: a live pid there is a reused one.
ws_bus_reclaim_orphans() {
  local inbox="$WS_SESSIONS/$1/inbox" f
  local -a expired=("$inbox".inflight.<->(Nms+10))
  ws_bus_reclaim "$1" || return 1
  ws_bus_paste_expire "$1" || return 1
  for f in "$inbox".inflight.<->(N); do
    [[ "${f##*.}" != $$ ]] && (( ! ${expired[(Ie)$f]} )) && kill -0 "${f##*.}" 2>/dev/null && continue
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

# Under the inbox lock: one pasted batch awaits a newer UserPromptSubmit, not just Enter.
ws_bus_idle_locked() {
  local id="$1" win="$2" inbox="$WS_SESSIONS/$1/inbox" message seq timeout="${WS_PASTE_ACK_TIMEOUT:-10}"
  ws_bus_reclaim_orphans "$id" || return 1
  [[ ! -e "$inbox.inflight.paste" && -s "$inbox" ]] || return 1
  message="$(jq -rjs 'map(.message) | join("\n\n")' "$inbox" && print -n .)" || return 1
  message="${message%.}"
  zmodload zsh/datetime
  seq="$(realtime_us "$EPOCHREALTIME")"
  jq -nc --arg seq "$seq" --argjson deadline "$(( EPOCHREALTIME + timeout ))" \
    '{seq:$seq,deadline:$deadline}' > "$inbox.paste" || return 1
  mv "$inbox" "$inbox.inflight.paste" && : > "$inbox" || return 1
  # Publish before input so delayed older hooks cannot confirm or settle this paste.
  tmux_ set -w -t "$win" @ws_seq "$seq" \; set -wu -t "$win" @ws_turn 2>/dev/null
  # Separate process: inherits neither lock; timeout requeues even if no later hook fires.
  "$WS_BIN" _paste-timeout "$id" "$seq" "$timeout" </dev/null >/dev/null 2>&1 &!
  ws_bus_paste "$id" "$win" "$message" || { ws_bus_paste_requeue "$id"; return 1; }
}

# Only a prompt hook newer than the paste watermark acknowledges this batch.
ws_bus_paste_ack() {
  local id="$1" seq="$2" inbox="$WS_SESSIONS/$1/inbox" pasted
  [[ -f "$inbox.paste" ]] || return 0
  pasted="$(jq -r '.seq' "$inbox.paste")" || return 1
  [[ "$pasted" == <-> ]] && (( seq > pasted )) || return 0
  rm -f "$inbox.inflight.paste" "$inbox.paste"
}

# Under the inbox lock; also checked on drains in case the timeout worker was interrupted.
ws_bus_paste_expire() {
  local id="$1" seq="${2:-}" inbox="$WS_SESSIONS/$1/inbox" pasted deadline
  [[ -f "$inbox.paste" ]] || return 0
  IFS='|' read -r pasted deadline <<<"$(jq -r '[.seq,.deadline] | join("|")' "$inbox.paste")"
  [[ -z "$seq" || "$seq" == "$pasted" ]] || return 0 # an old worker cannot reclaim a new batch
  zmodload zsh/datetime
  (( EPOCHREALTIME >= deadline )) || return 0
  ws_bus_paste_requeue "$id"
}

ws_bus_paste_requeue() {
  local inbox="$WS_SESSIONS/$1/inbox"
  if [[ -f "$inbox.inflight.paste" ]]; then
    cat "$inbox.inflight.paste" "$inbox" > "$inbox.tmp" && mv "$inbox.tmp" "$inbox" || return 1
  fi
  rm -f "$inbox.inflight.paste" "$inbox.paste"
}

# Invoked by the detached timer; no waiting while an inbox or hook lock is held.
ws_bus_paste_timeout() {
  sleep "$3"
  with_lock "$WS_SESSIONS/$1/inbox" ws_bus_paste_expire "$1" "$2"
}

# Named buffers isolate concurrent senders; raw paste preserves LF inside bracketed input.
# Ownership is rechecked inside tmux at paste and at Enter: an agent that exited mid-delivery
# (untagged) must never have its pane's shell receive the text. Lost ownership fails: requeue.
ws_bus_paste() {
  local id="$1" win="$2" message="$3" buffer="ws-$1-$$" pane owned
  pane="$(ws_bus_pane "$win")" || return 1
  owned="#{==:#{@ws_session},$id}"
  print -rn -- "$message" | tmux_ load-buffer -b "$buffer" - || return 1
  tmux_ if-shell -F -t "$pane" "$owned" "paste-buffer -d -p -r -b $buffer -t $pane" 2>/dev/null
  if tmux_ show-buffer -b "$buffer" >/dev/null 2>&1; then # not consumed: not pasted
    tmux_ delete-buffer -b "$buffer" 2>/dev/null
    return 1
  fi
  sleep 0.3
  [[ "$(tmux_ display -p -t "$pane" "$owned" 2>/dev/null)" == 1 ]] || return 1
  tmux_ if-shell -F -t "$pane" "$owned" "send-keys -t $pane Enter"
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
  local target="$1" id
  [[ "$target" != me ]] || target="${WS_SESSION:-}"
  target="${target#\#}"
  if [[ "$target" == <-> ]]; then
    [[ -f "$WS_SESSIONS/$target/meta" ]] || return 1
    print -r -- "$target"
    return 0
  fi
  [[ -n "$target" ]] || return 1
  local -a records=("$WS_SESSIONS"/<->/meta(N))
  (( ${#records} )) || return 1
  id="$(ws_bus_jq_records -r --arg name "$target" 'select(.name == $name) | .id' -- "${records[@]}")"
  id="${id%%$'\n'*}"
  [[ -n "$id" ]] || return 1
  print -r -- "$id"
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

# ws_bus_close <id> [<window>] — with a window, closes only that generation (check-and-clear
# under the meta lock): 3 = meta re-pointed to another window (resumed), left untouched.
ws_bus_close() {
  local id="$1" win="${2:-}"
  [[ -n "$win" ]] || { ws_bus_update "$id" '.window = "" | if .status == "active" then .status = "closed" else . end'; return; }
  [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" ]] || return 1
  with_lock "$WS_SESSIONS/$id/meta" ws_bus_close_owned_locked "$WS_SESSIONS/$id/meta" "$win"
}

ws_bus_close_owned_locked() {
  local meta="$1" win="$2" cur
  cur="$(jq -r '.window // ""' "$meta")" || return 1
  [[ -z "$cur" || "$cur" == "$win" ]] || return 3
  ws_bus_update_locked "$meta" '.window = "" | if .status == "active" then .status = "closed" else . end'
}
