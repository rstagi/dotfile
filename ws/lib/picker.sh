#!/bin/zsh
# fzf UI and tmux bindings. Rust embeds this script without the legacy data module.

cmd_pick() {
  command -v fzf >/dev/null || die "fzf not installed"
  local ws_command="${(q)WS_BIN}"
  local out key row wt win rows # never `path`: zsh ties it to $PATH
  rows="$("$WS_BIN" _rows)" || return
  out="$(print -r -- "$rows" | fzf --ansi --delimiter '\t' --with-nth 1 --no-sort --layout reverse --sync \
    --header-first --bind "load:transform-header($ws_command _summary)" \
    --bind "start:transform-header($ws_command _summary)+pos($(current_row "$rows"))" \
    --preview "$ws_command _preview {2} {3}" --preview-window 'right,50%,follow' \
    --expect ctrl-n,ctrl-o,ctrl-r,ctrl-a,ctrl-t,ctrl-v \
    --bind "ctrl-p:execute-silent($ws_command pin {2})+reload($ws_command _rows)" \
    --bind "ctrl-s:execute-silent($ws_command _toggle_sessions)+reload($ws_command _rows)" \
    --bind "ctrl-u:execute-silent($ws_command _unread {3})+reload($ws_command _rows)" \
    --bind "ctrl-x:execute($ws_command rm --interactive {2})+reload($ws_command _rows)")" || return 0
  key="${out%%$'\n'*}"
  row=""
  [[ "$out" == *$'\n'* ]] && row="${out#*$'\n'}"
  wt="$(print -r -- "$row" | cut -f2)"
  win="$(print -r -- "$row" | cut -f3)"
  case "$key" in
  ctrl-n) "$WS_BIN" new ;;
  ctrl-o) "$WS_BIN" new --agent codex ;;
  ctrl-r) "$WS_BIN" new --agent grok ;;
  ctrl-a) [[ -n "$wt" ]] && "$WS_BIN" add "$wt" ;;
  ctrl-t) [[ -n "$wt" ]] && "$WS_BIN" add "$wt" --agent codex ;;
  ctrl-v) [[ -n "$wt" ]] && "$WS_BIN" add "$wt" --agent grok ;;
  *)
    if [[ -n "$win" ]]; then
      if [[ "${WS_PICKER_RUST:-}" == 1 ]]; then
        "$WS_BIN" _pick_focus "$win"
      else
        focus "$win"
      fi
    elif [[ -n "$wt" ]]; then
      "$WS_BIN" open "$wt"
    fi
    ;;
  esac
}

# Configure ws keys. Other tmux sessions keep the default workspace selector.
configure_bindings() {
  tmux_ has-session 2>/dev/null || return 0
  local in_ws="#{!=:#{@ws_path},}" ws_command="${(q)WS_BIN}" key value
  if [[ "${WS_PICKER_RUST:-}" == 1 ]]; then
    for key in WS_HOME WS_STATE WS_SOCK WS_TMUX_SOCKET WS_ROOT WS_NOTIFY; do
      if (( ${+parameters[$key]} )); then
        value="${(P)key}"
        ws_command="$key=${(q)value} $ws_command"
      fi
    done
    ws_command="env $ws_command"
  fi
  local pick_cmd="$ws_command pick" new_cmd="$ws_command new" merge_cmd="$ws_command _merge_popup"
  local add_cmd="$ws_command add #{q:@ws_path}"
  tmux_ set-hook -g 'session-window-changed[77]' 'set -wu @ws_unread'
  tmux_ set-hook -g 'client-session-changed[77]' 'set -wu @ws_unread'
  # The tmux parser reads an unquoted "#" at the start of a word as a comment. Quote each format.
  # display-popup expands the -d format. The path needs no other quotes.
  local start_dir="-d '#{pane_current_path}'"
  tmux_ bind-key w if-shell -F "$in_ws" \
    "display-popup -E -w 90% -h 85% $start_dir ${(q)pick_cmd}" "choose-tree -Zw"
  tmux_ bind-key N if-shell -F "$in_ws" \
    "display-popup -E -w 60% -h 50% $start_dir ${(q)new_cmd}"
  tmux_ bind-key M if-shell -F "$in_ws" \
    "display-popup -E -w 70% -h 50% $start_dir ${(q)merge_cmd}"
  tmux_ bind-key a if-shell -F "$in_ws" "run-shell ${(q)add_cmd}"
  add_cmd="$ws_command add #{q:@ws_path} --agent codex"
  tmux_ bind-key A if-shell -F "$in_ws" "run-shell ${(q)add_cmd}"
  add_cmd="$ws_command add #{q:@ws_path} --agent grok"
  tmux_ bind-key G if-shell -F "$in_ws" "run-shell ${(q)add_cmd}"
  # Array indices 77 and 78 preserve user hooks at index 0.
  local dismiss_cmd="$ws_command _dismiss '#{window_id}'"
  local run_cmd="run-shell -b ${(q)dismiss_cmd}"
  local dismiss="if-shell -F '#{==:#{@ws_notify_open},1}' ${(q)run_cmd}"
  tmux_ set-hook -g 'session-window-changed[78]' "$dismiss"
  tmux_ set-hook -g 'client-session-changed[78]' "$dismiss"
}

# current_row <picker rows> — 1-based index of the row for the tab we were opened from (its
# window, else its worktree), so the picker starts on it; 1 outside tmux or ws. In the
# prefix+w popup TMUX_PANE is unset and display -p resolves to the client's current window.
current_row() {
  local cur ws_path
  local -a target=(${TMUX_PANE:+-t} ${TMUX_PANE:-})
  cur="$(tmux_ display -p "${target[@]}" $'#{window_id}\t#{@ws_path}' 2>/dev/null)"
  ws_path="${cur#*$'\t'}" cur="${cur%%$'\t'*}"
  print -r -- "$1" | awk -F '\t' -v w="$cur" -v p="$ws_path" '
    w != "" && $3 == w { print NR; found = 1; exit }
    p != "" && $2 == p && !byp { byp = NR }
    END { if (!found) print (byp ? byp : 1) }'
}

if [[ "$ZSH_EVAL_CONTEXT" == *:file ]]; then
  source "${${(%):-%x}:A:h}/picker-legacy.sh"
else
  set -u -o pipefail
  tmux_() {
    case "${WS_TMUX_SOCKET:-}" in
      */*) command tmux -S "$WS_TMUX_SOCKET" "$@" ;;
      ?*) command tmux -L "$WS_TMUX_SOCKET" "$@" ;;
      *) command tmux "$@" ;;
    esac
  }
  die() { print -u2 "ws: $1"; exit 1; }
  case "${1:-pick}" in
    pick) cmd_pick ;;
    bindings) configure_bindings ;;
    *) die "Unknown picker action: $1" ;;
  esac
fi
