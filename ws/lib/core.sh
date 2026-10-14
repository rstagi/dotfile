#!/bin/zsh
# Core helpers: errors, tmux wrappers, locks, atomic rewrites, focus and window tagging. Sourced by ws.sh.

die() { print -u2 "ws: $1"; exit "${2:-1}"; }

tmux_() {
  if [[ -n "${WS_TMUX_SOCKET:-}" ]]; then
    command tmux -L "$WS_TMUX_SOCKET" "$@"
  else
    command tmux "$@"
  fi
}

# Replaces the process (attach from a plain terminal); `exec` can't target the tmux_ function.
exec_tmux() {
  if [[ -n "${WS_TMUX_SOCKET:-}" ]]; then
    exec tmux -L "$WS_TMUX_SOCKET" "$@"
  fi
  exec tmux "$@"
}

# with_lock <file> <cmd...> — runs cmd holding <file>.lock: parallel tool calls fire hooks
# concurrently, and unserialized rewrites of the sessions file wipe its rows.
with_lock() {
  local f="$1"
  shift
  : >> "$f.lock" # flock won't create it
  (zsystem flock -t 3 -i 0.01 "$f.lock" 2>/dev/null || { print -u2 "ws: timed out waiting for lock: $f.lock"; exit 1; }; "$@")
}

# rewrite <file> <awk args...> — replaces the file with awk's output, atomically.
rewrite() {
  local f="$1"
  shift
  awk "$@" "$f" > "$f.tmp" && mv "$f.tmp" "$f"
}

# confirm <question> — y/N from stdin; the prompt is printed explicitly (zsh's `read "?p"`
# stays silent when stdin isn't a terminal).
confirm() {
  local ans
  print -rn -- "$1 [y/N] "
  read -r ans || ans=""
  [[ "$ans" == (y|Y) ]]
}

check_agent() { (( ${AGENTS[(Ie)$1]} )) || die "agent must be one of: ${(j:, :)AGENTS}"; }

# realtime_us <$EPOCHREALTIME value> — integer µs since epoch from one snapshot (two reads
# could straddle a second boundary and jump ~1s ahead).
realtime_us() {
  local frac="${1#*.}"
  print -r -- "${1%.*}${(r:6::0:)frac}"
}

focus() {
  local win="$1" sess
  [[ -n "$win" ]] || return 1
  sess="$(tmux_ display -p -t "$win" '#{session_id}')" || return 1
  tmux_ select-window -t "$win"
  tmux_ set -wu -t "$win" @ws_unread # already current: no hook fires
  if [[ -n "${TMUX:-}" ]]; then
    tmux_ switch-client -t "$sess"
  else
    exec_tmux attach-session -t "$sess"
  fi
}

# Marks a tab as an agent session: what windows_for, cmd_list and hooks key on.
tag_window() {
  local win="$1" wt="$2" slot="$3" agent="$4" tab="$4" id pane
  id="$(ws_bus_identity "$wt" "$slot" "$agent" "")" || return 1
  (( slot > 1 )) && tab+="#$slot"
  tmux_ rename-window -t "$win" "$tab"
  tmux_ set -w -t "$win" @ws_path "$wt"
  tmux_ set -w -t "$win" @ws_slot "$slot"
  tmux_ set -w -t "$win" @ws_session "$id"
  tmux_ set -w -t "$win" @ws_agent "$agent"
  tmux_ set -w -t "$win" @ws_state idle
  # Agent pane: the caller's when it runs here (ws run), else the new tab's only pane.
  pane="${TMUX_PANE:-}"
  [[ -n "$pane" && "$(tmux_ display -p -t "$pane" '#{window_id}' 2>/dev/null)" == "$win" ]] ||
    pane="$(tmux_ display -p -t "$win" '#{pane_id}')"
  tmux_ set -w -t "$win" @ws_pane "$pane"
  ws_bus_update "$id" --arg win "$win" '.window = $win | if .status == "closed" then .status = "active" else . end'
  tmux_ set -w -t "$win" automatic-rename off
  tmux_ set -w -t "$win" window-status-format "#I $(state_format) #W"
  tmux_ set -w -t "$win" window-status-current-format "#[bold]#I $(state_format) #W#[default]"
}

untag_window() {
  local opt
  for opt in @ws_path @ws_slot @ws_session @ws_agent @ws_state @ws_main @ws_turn @ws_unread automatic-rename \
    window-status-format window-status-current-format; do
    tmux_ set -wu -t "$1" "$opt" 2>/dev/null
  done
  tmux_ show -w -t "$1" 2>/dev/null | awk '/^@ws_sub_/ { print $1 }' | while read -r opt; do
    tmux_ set -wu -t "$1" "$opt"
  done
}

state_icon() {
  case "$1" in
  waiting) print -n $'\e[31m●\e[0m' ;;
  working) print -n $'\e[33m◐\e[0m' ;;
  idle) print -n $'\e[32m○\e[0m' ;;
  *) print -n $'\e[2m·\e[0m' ;;
  esac
}

state_format() {
  print -rn -- '#{?#{==:#{@ws_state},waiting},#[fg=red]●#[fg=default],#{?#{==:#{@ws_state},working},#[fg=yellow]◐#[fg=default],#[fg=green]○#[fg=default]}}'
}
