#!/bin/zsh
set -u -o pipefail
zmodload zsh/system # zsystem flock (sessions file)

# ws — bare-minimum terminal Conductor: one git worktree + one tmux session per workspace,
# one tab (window) per agent session inside it. Status and resume ids come from Claude Code /
# Codex / Grok Build hooks — no screen scraping.
# Every launched/resumed agent gets its global identity (spawned ones: a reminder to run
# `ws done` when the task is complete); ws-sessions teaches messaging and dependent tasks.
#
#   ws                         picker (attaches when run outside tmux)
#   ws new [--repo P] [--branch B] [--agent claude|codex|grok] [--detach]
#   ws add [name|path] [--agent claude|codex|grok] [--detach]   another agent tab (default: current worktree)
#   ws open <workspace|path|id|name> [--detach]   focus or resume a workspace/session
#   ws pick                    grouped picker: enter open · ctrl-n new · ctrl-o new codex ·
#                              ctrl-r new grok · ctrl-a add claude tab · ctrl-t add codex tab ·
#                              ctrl-v add grok tab · ctrl-p pin · ctrl-x rm ·
#                              ctrl-s show/hide sessions · ctrl-u toggle unread (• = finished, not yet seen)
#   ws pin <name|path>         toggle pin (pinned workspaces are listed first)
#   ws list                    TSV: state name path agent window pinned active unread
#   ws whoami                  own global session id (inside an agent tab)
#   ws sessions [id|name]      TSV: id name workspace agent state status
#   ws name <label>            name your session (unique; usable in place of its id)
#   ws send [--force] <id|name> <msg|->   sender-prefixed message (or stdin): paste idle, queue busy
#                              done needs --force; stopped sessions resume before delivery
#   ws done [--output <file>|-]   declare own session done; replace output.md (default empty)
#                              --output - also reads stdin; repeat appends another done event
#                              pending inbox/in-flight messages refuse done; let Stop deliver first
#                              spawned tabs close after publication; user-opened tabs stay
#   ws show <id|name>           session metadata, lifecycle status and output path
#   ws on <id[,id…]> <done|exit> (--send-to <id|me> <msg> | --spawn <prompt> [--agent A])
#                              one-shot fan-in; spawn defaults to claude in current worktree
#                              done counts past events; exit counts only events after registration
#   ws triggers                TSV: pending trigger id, sources, event, action, target, template
#   ws on --rm <trigger-id>     cancel a pending trigger; cycles are rejected on registration
#   ws rm <name|path> [--force] [--delete-branch]
#   ws merge                   in a workspace: confirm, merge its PR (gh, $WS_MERGE_METHOD,
#                              default squash), fast-forward the main checkout, then on confirm
#                              remove worktree + tabs + local/remote branch (prefix+M); if
#                              dirty, lists the files and asks to force delete
#   ws claude|codex|grok [args]  run the agent in this tab as a tracked session (e.g. a prefix+c tab)
#   ws hook <event>            called by agent hooks inside a session window
#                              (working|idle|stop|cancel|waiting|subagent-start|subagent-stop)
#                              cancel requires the current turn id; unknown/stale ids are ignored
#   ws _dismiss <window|pane>  tab seen or agent exited: emit `closed` if a notification is open
#
# Workspace slots live in <worktree git dir>/ws-sessions (slot, agent, resume id, global id).
# Global identities live in $WS_HOME/sessions/<id>/. A killed tab resumes on `ws open`;
# a clean exit forgets its workspace slot while retaining its global record.
# Exited tabs stop accepting messages before exit triggers run; sends resume the session.

WS_BIN="${0:A}"
WS_ROOT="${WS_ROOT:-$HOME/.ws/worktrees}"
WS_HOME="${WS_HOME:-$HOME/.ws}"
WS_NOTIFY="${WS_NOTIFY:-1}"
WS_NOTIFIER="${WS_NOTIFIER:-auto}" # auto | osascript | terminal-notifier | boringnotch | executable path
WS_BORINGNOTCH_SOCKET="${WS_BORINGNOTCH_SOCKET:-$HOME/Library/Application Support/boringNotch/notify.sock}"
# Notification sound (file path or macOS sound name) + icon per state.
WS_NOTIFY_SOUND_DONE="${WS_NOTIFY_SOUND_DONE:-${WS_BIN:h}/assets/ws/sounds/done.wav}"
WS_NOTIFY_SOUND_WAITING="${WS_NOTIFY_SOUND_WAITING:-${WS_BIN:h}/assets/ws/sounds/waiting.wav}"
WS_NOTIFY_ICON_DONE="${WS_NOTIFY_ICON_DONE:-${WS_BIN:h}/assets/ws/done.png}"
WS_NOTIFY_ICON_WAITING="${WS_NOTIFY_ICON_WAITING:-${WS_BIN:h}/assets/ws/waiting.png}"
# Branded copy of terminal-notifier (macOS takes a notification's icon from the sending app).
WS_NOTIFIER_APP="$WS_HOME/ws.app"
WS_NOTIFIER_SOURCE="${WS_NOTIFIER_SOURCE:-/opt/homebrew/opt/terminal-notifier/terminal-notifier.app}"
WS_APP_ICON="${WS_BIN:h}/assets/ws/app.png"
WS_AGENT_SHELL="${WS_AGENT_SHELL:-zsh -ic}" # interactive: agents need .zshrc (secrets, PATH)
REGISTRY="${LOOP_REPO_REGISTRY:-$HOME/.loop/repos.json}"
WS_REPO_ROOTS="${WS_REPO_ROOTS:-$HOME/Dev $HOME/dotfile}" # scanned for repos by `ws new`
PINS="$WS_HOME/pins"
SHOW_SESSIONS="$WS_HOME/show-sessions" # exists → picker lists per-session rows (ctrl-s)
DEFAULT_INCLUDE_GLOBS=(".env*")
AGENTS=(claude codex grok)
GROK_HOOKS="${GROK_HOME:-$HOME/.grok}/hooks/ws.json"
source "${WS_BIN:h}/ws-bus.sh"

main() {
  local cmd="${1:-}"
  [[ $# -gt 0 ]] && shift
  case "$cmd" in
  "") cmd_attach ;;
  new) cmd_new "$@" ;;
  add) cmd_add "$@" ;;
  open) cmd_open "$@" ;;
  pick) cmd_pick ;;
  pin) cmd_pin "$@" ;;
  list | ls) cmd_list ;;
  whoami) cmd_whoami ;;
  sessions) cmd_sessions "$@" ;;
  name) cmd_name "$@" ;;
  send) cmd_send "$@" ;;
  done) cmd_done "$@" ;;
  on) cmd_on "$@" ;;
  triggers) cmd_triggers "$@" ;;
  show) cmd_show "$@" ;;
  rm) cmd_rm "$@" ;;
  merge) cmd_merge ;;
  _merge_popup) (cmd_merge); print -n "\npress enter to close"; read -r _ ;; # subshell: die must not skip the pause
  claude | codex | grok) cmd_run "$cmd" "$@" ;;
  hook) cmd_hook "$@" ;;
  _dismiss) cmd_dismiss "$@" ;;
  _rows) picker_rows ;;
  _summary) picker_summary ;;
  _toggle_sessions) toggle_sessions ;;
  _unread) toggle_unread "$@" ;;
  _preview) preview "$@" ;;
  _end) session_remove "$@" ;;
  _agent) ws_bus_run_agent "$@" ;;
  _build-notifier) build_notifier ;;
  -h | --help | help) sed -n '/^# ws —/,/^# Workspace slots/{ /^# Workspace slots/d; p; }' "$WS_BIN" | sed 's/^# \{0,1\}//' ;;
  *) die "unknown command: $cmd (try: ws help)" ;;
  esac
}

cmd_attach() {
  configure_bindings
  if [[ -z "$(cmd_list)" ]]; then
    cmd_new
  else
    cmd_pick
  fi
}

cmd_new() {
  local repo="" branch="" agent="claude" detach=0
  while [[ $# -gt 0 ]]; do
    case "$1" in
    --repo) repo="$2"; shift 2 ;;
    --branch) branch="$2"; shift 2 ;;
    --agent) agent="$2"; shift 2 ;;
    --detach) detach=1; shift ;;
    *) die "new: unknown arg $1" ;;
    esac
  done
  check_agent "$agent"
  [[ -n "$repo" ]] || repo="$(pick_repo)" || exit 1
  repo="$(main_checkout "$repo")" || die "not a git repo: $repo"
  if [[ -z "$branch" ]]; then
    read -r "branch?branch (${repo:t}): " </dev/tty || exit 1
  fi
  [[ -n "$branch" ]] || die "new: branch required"
  git check-ref-format --branch "$branch" >/dev/null 2>&1 || die "invalid branch: $branch"

  local wt="$WS_ROOT/${repo:t}/${branch//\//-}"
  [[ -e "$wt" ]] && die "workspace exists: $wt (use: ws open ${repo:t}/${wt:t})"
  add_worktree "$repo" "$branch" "$wt" || die "git worktree add failed"
  copy_included_files "$repo" "$wt"
  local win slot
  slot="$(session_add "$wt" "$agent")" || die "cannot register session"
  win="$(open_session "$wt" "$slot" "$agent" "")" || die "cannot open session"
  run_setup "$wt" "$(setup_script "$repo")"
  (( detach )) || focus "$win"
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
  (( s == 0 )) && session_remove "$wt" "$slot" # else kept for resume, as for a killed tab
  untag_window "$win"
  return $s
}

cmd_pick() {
  command -v fzf >/dev/null || die "fzf not installed"
  local out key row wt win rows # never `path`: zsh ties it to $PATH
  rows="$(picker_rows)"
  out="$(print -r -- "$rows" | fzf --ansi --delimiter '\t' --with-nth 1 --no-sort --layout reverse --sync \
    --header-first --bind "load:transform-header($WS_BIN _summary)" \
    --bind "start:transform-header($WS_BIN _summary)+pos($(current_row "$rows"))" \
    --preview "$WS_BIN _preview {2} {3}" --preview-window 'right,50%,follow' \
    --expect ctrl-n,ctrl-o,ctrl-r,ctrl-a,ctrl-t,ctrl-v \
    --bind "ctrl-p:execute-silent($WS_BIN pin {2})+reload($WS_BIN _rows)" \
    --bind "ctrl-s:execute-silent($WS_BIN _toggle_sessions)+reload($WS_BIN _rows)" \
    --bind "ctrl-u:execute-silent($WS_BIN _unread {3})+reload($WS_BIN _rows)" \
    --bind "ctrl-x:execute($WS_BIN rm --interactive {2})+reload($WS_BIN _rows)")" || return 0
  key="${out%%$'\n'*}"
  row=""
  [[ "$out" == *$'\n'* ]] && row="${out#*$'\n'}"
  wt="$(print -r -- "$row" | cut -f2)"
  win="$(print -r -- "$row" | cut -f3)"
  case "$key" in
  ctrl-n) cmd_new ;;
  ctrl-o) cmd_new --agent codex ;;
  ctrl-r) cmd_new --agent grok ;;
  ctrl-a) [[ -n "$wt" ]] && cmd_add "$wt" ;;
  ctrl-t) [[ -n "$wt" ]] && cmd_add "$wt" --agent codex ;;
  ctrl-v) [[ -n "$wt" ]] && cmd_add "$wt" --agent grok ;;
  *)
    if [[ -n "$win" ]]; then
      focus "$win"
    elif [[ -n "$wt" ]]; then
      cmd_open "$wt"
    fi
    ;;
  esac
}

cmd_pin() {
  local wt name
  wt="$(resolve_workspace "${1:-}")" || die "no such workspace: ${1:-}"
  name="$(workspace_name "$wt")"
  mkdir -p "$WS_HOME"
  touch "$PINS"
  if grep -qxF -- "$name" "$PINS"; then
    unpin "$name"
  else
    print -r -- "$name" >> "$PINS"
  fi
}

# One row per session window; a workspace with no window gets one "stopped" row.
cmd_list() {
  local wt name pinned win found
  for wt in "$WS_ROOT"/*/*(N/); do
    name="$(workspace_name "$wt")"
    pinned=0
    [[ -f "$PINS" ]] && grep -qxF -- "$name" "$PINS" && pinned=1
    found=0
    for win in ${(f)"$(windows_for "$wt")"}; do
      found=1
      local -a o=("${(@ps:\t:)$(tmux_ display -p -t "$win" '#{@ws_state}	#{@ws_slot}	#{@ws_agent}	#{window_active}	#{?@ws_unread,1,0}')}")
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "${o[1]}" "$name${${o[2]:#1}:+#${o[2]}}" "$wt" "${o[3]}" "$win" "$pinned" "${o[4]}" "${o[5]}"
    done
    (( found )) || printf 'stopped\t%s\t%s\t-\t\t%s\t0\t0\n' "$name" "$wt" "$pinned"
  done
}

cmd_rm() {
  local target="" force=0 delete_branch=0 interactive=0
  while [[ $# -gt 0 ]]; do
    case "$1" in
    --force) force=1; shift ;;
    --delete-branch) delete_branch=1; shift ;;
    --force-delete-branch) delete_branch=2; shift ;; # e.g. squash-merged: not an ancestor of main
    --interactive) interactive=1; shift ;;
    *) target="$1"; shift ;;
    esac
  done
  local wt repo branch
  wt="$(resolve_workspace "$target")" || die "no such workspace: $target"
  repo="$(main_checkout "$wt")" || die "cannot find main checkout for $wt"
  branch="$(git -C "$wt" branch --show-current)"
  if (( interactive )); then
    local ans
    read -r "ans?remove $(workspace_name "$wt") and all its sessions? [y/N/b=also delete branch] " </dev/tty
    case "$ans" in
    y | Y) ;;
    b | B) delete_branch=1 ;;
    *) return 0 ;;
    esac
  fi
  if (( ! force )) && [[ -n "$(git -C "$wt" status --porcelain 2>/dev/null)" ]]; then
    die "workspace has uncommitted changes: $wt (use --force)"
  fi
  # Kill windows last: rm may run from inside one of them (picker popup).
  local -a sessions=(${(f)"$(sessions_for "$wt")"}) wins=(${(f)"$(windows_for "$wt")"})
  local -a rm_flags=()
  (( force )) && rm_flags=(--force --force)
  git -C "$repo" worktree remove "${rm_flags[@]}" "$wt" || die "git worktree remove failed: $wt"
  rmdir "${wt:h}" 2>/dev/null
  unpin "$(workspace_name "$wt")"
  if (( delete_branch == 2 )) && [[ -n "$branch" ]]; then
    git -C "$repo" branch -D "$branch" >/dev/null
  elif (( delete_branch )) && [[ -n "$branch" ]]; then
    git -C "$repo" branch -d "$branch" >/dev/null || print -u2 "ws: kept unmerged branch $branch (git branch -D to force)"
  fi
  local target
  for target in "${wins[@]}"; do tmux_ kill-window -t "$target" 2>/dev/null; done
  for target in "${sessions[@]}"; do tmux_ kill-session -t "$target" 2>/dev/null; done
  return 0
}

cmd_merge() {
  local wt repo branch default
  wt="$(current_workspace)" && wt="$(resolve_workspace "$wt")" || die "merge: not inside a workspace"
  repo="$(main_checkout "$wt")" || die "cannot find main checkout for $wt"
  branch="$(git -C "$wt" branch --show-current)"
  [[ -n "$branch" ]] || die "merge: detached HEAD in $wt"

  local pr
  pr="$(cd "$wt" && gh pr view "$branch" --json number,title,url \
    --jq '"#\(.number) \(.title) (\(.url))"' 2>/dev/null)"
  [[ -n "$pr" ]] || die "merge: no PR found for $branch"
  confirm "merge PR $pr (${WS_MERGE_METHOD:-squash})?" || { print "not merged"; return 0; }

  print "→ merging PR for $branch (${WS_MERGE_METHOD:-squash})"
  if ! (cd "$wt" && gh pr merge "$branch" "--${WS_MERGE_METHOD:-squash}"); then
    [[ "$(cd "$wt" && gh pr view "$branch" --json state -q .state 2>/dev/null)" == MERGED ]] \
      || die "merge: gh pr merge failed"
    print "  (already merged)"
  fi

  default="$(git -C "$repo" symbolic-ref -q --short refs/remotes/origin/HEAD)"
  default="${default#origin/}"
  [[ -n "$default" ]] || default=main
  print "→ updating $default in ${repo/#$HOME/~}"
  git -C "$repo" fetch -q origin || die "merge: fetch failed"
  if [[ "$(git -C "$repo" branch --show-current)" == "$default" ]]; then
    git -C "$repo" merge -q --ff-only "origin/$default" || die "merge: cannot fast-forward $default"
  else
    git -C "$repo" fetch -q origin "$default:$default" || die "merge: cannot fast-forward $default"
  fi

  confirm "delete $(workspace_name "$wt") (worktree, tabs, local + remote $branch)?" \
    || { print "kept $(workspace_name "$wt")"; return 0; }
  local dirty
  local -a rm_flags=(--force-delete-branch)
  dirty="$(git -C "$wt" status --short 2>/dev/null)"
  if [[ -n "$dirty" ]]; then
    print "can't delete $(workspace_name "$wt"): uncommitted/untracked files:"
    print -r -- "$dirty" | sed 's/^/  /'
    confirm "force delete anyway (these are lost)?" || { print "kept $(workspace_name "$wt")"; return 0; }
    rm_flags+=(--force)
  fi
  git -C "$repo" push -q origin --delete "$branch" 2>/dev/null # GitHub may have auto-deleted it
  cmd_rm "$wt" "${rm_flags[@]}"
}

# confirm <question> — y/N from stdin; the prompt is printed explicitly (zsh's `read "?p"`
# stays silent when stdin isn't a terminal).
confirm() {
  local ans
  print -rn -- "$1 [y/N] "
  read -r ans || ans=""
  [[ "$ans" == (y|Y) ]]
}

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
  local reply=""
  [[ "$event" != stop ]] || reply='{}'
  [[ -n "${WS_WORKSPACE:-}" && -n "${TMUX_PANE:-}" ]] || { [[ -z "$reply" ]] || print -r -- "$reply"; return 0; }
  [[ "$event" == (working|idle|stop|cancel|waiting|subagent-start|subagent-stop) ]] || return 0
  record_resume_id "$payload"
  local win lock lockfd prev last state agent_id="" applied=0
  [[ -n "$payload" ]] && agent_id="$(print -r -- "$payload" | jq -r '.agent_id // empty' 2>/dev/null)"
  agent_id="${agent_id//[^A-Za-z0-9_-]/_}"
  win="$(tmux_ display -p -t "$TMUX_PANE" '#{window_id}' 2>/dev/null)" || { [[ -z "$reply" ]] || print -r -- "$reply"; return 0; }
  lock="${TMPDIR:-/tmp}/ws-hook-${WS_TMUX_SOCKET:-default}-$win.lock"
  { : >>"$lock" && zsystem flock -t 2 -i 0.01 -f lockfd "$lock"; } 2>/dev/null || { [[ -z "$reply" ]] || print -r -- "$reply"; return 0; }
  IFS='|' read -r last prev <<<"$(tmux_ display -p -t "$TMUX_PANE" '#{@ws_seq}|#{@ws_state}' 2>/dev/null)"
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
    if [[ "$event" == stop ]]; then
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
        state="$(hook_state working "" "$prev")"
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

# Tab selected (tmux hook) or agent exited: emits one `closed` event when the tab's last
# notification is still open, so receivers drop it. The tab state itself is untouched.
cmd_dismiss() {
  zmodload zsh/datetime 2>/dev/null
  local seq="$(realtime_us "$EPOCHREALTIME")" target="${1:-}" open state
  (( WS_NOTIFY )) && [[ -n "$target" ]] || return 0
  IFS='|' read -r open state <<<"$(tmux_ display -p -t "$target" '#{@ws_notify_open}|#{@ws_state}' 2>/dev/null)"
  [[ "$open" == 1 ]] || return 0
  emit_notify_event closed "$state" "" "$seq" "$target"
  return 0
}

# realtime_us <$EPOCHREALTIME value> — integer µs since epoch from one snapshot (two reads
# could straddle a second boundary and jump ~1s ahead).
realtime_us() {
  local frac="${1#*.}"
  print -r -- "${1%.*}${(r:6::0:)frac}"
}

# prefix+w → ws picker, prefix+N → new workspace, prefix+a / prefix+A / prefix+G → claude /
# codex / grok tab in the current worktree, prefix+M → ws merge. Only inside ws sessions (they carry the @ws_path session option);
# other sessions keep tmux's default choose-tree on prefix+w. Landing on a tab, however you
# get there, marks it read (hooks at a high index, so user hooks at [0] are kept).
configure_bindings() {
  tmux_ has-session 2>/dev/null || return 0 # no server yet: bound when the first session is created
  local in_ws="#{!=:#{@ws_path},}"
  tmux_ set-hook -g 'session-window-changed[77]' 'set -wu @ws_unread'
  tmux_ set-hook -g 'client-session-changed[77]' 'set -wu @ws_unread'
  tmux_ bind-key w if-shell -F "$in_ws" \
    "display-popup -E -w 90% -h 85% -d '#{pane_current_path}' '$WS_BIN pick'" "choose-tree -Zw"
  tmux_ bind-key N if-shell -F "$in_ws" \
    "display-popup -E -w 60% -h 50% -d '#{pane_current_path}' '$WS_BIN new'"
  tmux_ bind-key M if-shell -F "$in_ws" \
    "display-popup -E -w 70% -h 50% -d '#{pane_current_path}' '$WS_BIN _merge_popup'"
  tmux_ bind-key a if-shell -F "$in_ws" "run-shell \"'$WS_BIN' add '#{@ws_path}'\""
  tmux_ bind-key A if-shell -F "$in_ws" "run-shell \"'$WS_BIN' add '#{@ws_path}' --agent codex\""
  tmux_ bind-key G if-shell -F "$in_ws" "run-shell \"'$WS_BIN' add '#{@ws_path}' --agent grok\""
  # Selecting a tab (in any session, or by switching session) dismisses its open notification.
  # Fixed array index (77 = unread): re-running stays idempotent and leaves the user's own hooks alone.
  local dismiss="if-shell -F '#{==:#{@ws_notify_open},1}' \"run-shell -b \\\"'$WS_BIN' _dismiss '#{window_id}'\\\"\""
  tmux_ set-hook -g 'session-window-changed[78]' "$dismiss"
  tmux_ set-hook -g 'client-session-changed[78]' "$dismiss"
}

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
  run="PATH=${(q)WS_HOME}/bin:\$PATH; ${(q)WS_BIN} _agent $id ${(q)cmd}; s=\$?; ${(q)WS_BIN} _dismiss \"\$TMUX_PANE\"; if (( s == 0 )); then ${(q)WS_BIN} _end ${(q)wt} $slot; else print \"agent exited (\$s) — enter to close\"; read; fi"
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

# Per-launch settings (claude --settings) so ws hooks never leak into non-ws sessions.
# No SessionStart: its id has no conversation yet, and `claude --resume <id>` fails on it.
write_claude_settings() {
  mkdir -p "$WS_HOME"
  jq -n --arg h "$WS_BIN hook" '
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
  mkdir -p "${GROK_HOOKS:h}"
  jq -n --arg h "$WS_BIN hook" '
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

# Picker rows (TSV: display, path, window): pinned first, then grouped by repo. No header
# rows — every row must be actionable (the cursor starts on the first one). The repo name
# shows on a group's first row and dims on the rest, so grouping stays visible and searchable.
# A worktree with several sessions gets one indented row per session (enter → that tab).
picker_rows() {
  local list
  list="$(cmd_list)"
  local -a rows=("${(@f)$(print -r -- "$list" | workspace_rows | sort -t $'\t' -k2,2)}") # by repo/branch
  rows=(${rows:#})
  local -a pinned=(${(M)rows:#*$'\t'1}) unpinned=(${(M)rows:#*$'\t'0})
  local row repo prev=""
  for row in "${pinned[@]}"; do
    picker_row "$row" pinned
    [[ -e "$SHOW_SESSIONS" ]] && session_rows "$list" "$row"
  done
  for row in "${unpinned[@]}"; do
    repo="${${row#*$'\t'}%%/*}"
    picker_row "$row" "${${repo:#$prev}:+first}"
    [[ -e "$SHOW_SESSIONS" ]] && session_rows "$list" "$row"
    prev="$repo"
  done
}

# Picker header: session totals across all workspaces + key help.
picker_summary() {
  local -a states=("${(@f)$(cmd_list | cut -f1)}")
  local st n
  local -a parts=()
  for st in waiting working idle stopped; do
    n=${#${(M)states:#$st}}
    (( n )) && parts+=("$(state_icon "$st") $n $st")
  done
  n=${#${(M)${(f)"$(cmd_list | cut -f8)"}:#1}}
  (( n )) && parts+=($'\e[36m•\e[0m'" $n unread")
  print -r -- "${(j: · :)parts:-no workspaces}"
  print -r -- $'\e[2m↵ open · ^s sessions · ^p pin · ^u unread · ^x rm\e[0m'
  print -r -- $'\e[2m^n/^o/^r new claude/codex/grok · ^a/^t/^v add claude/codex/grok tab\e[0m'
}

toggle_sessions() {
  mkdir -p "$WS_HOME"
  if [[ -e "$SHOW_SESSIONS" ]]; then rm -f "$SHOW_SESSIONS"; else touch "$SHOW_SESSIONS"; fi
}

# toggle_unread <window> — picker ctrl-u: mark a tab unread again (or read). No-op when stopped.
toggle_unread() {
  local win="${1:-}"
  [[ -n "$win" ]] || return 0
  if [[ -n "$(tmux_ show -wqv -t "$win" @ws_unread)" ]]; then
    tmux_ set -wu -t "$win" @ws_unread
  else
    tmux_ set -w -t "$win" @ws_unread 1
  fi
}

# cmd_list rows (stdin) collapsed to one per worktree: most urgent state, agent (or "N
# sessions"), target window (first unread tab, else last-used: active in its tmux session,
# else the first), unread (any tab), pinned.
workspace_rows() {
  awk -F '\t' -v OFS='\t' '
    function rank(s) { return s == "waiting" ? 3 : s == "working" ? 2 : s == "idle" ? 1 : 0 }
    !($3 in seen) { seen[$3] = 1; order[++n] = $3; st[$3] = $1; nm[$3] = $2; ag[$3] = $4; w[$3] = $5; pin[$3] = $6; act[$3] = ($7 == 1); un[$3] = ($8 == 1); cnt[$3] = 1; sub(/#.*/, "", nm[$3]); next }
    { if (rank($1) > rank(st[$3])) st[$3] = $1; if (!un[$3] && ($8 == 1 || ($7 == 1 && !act[$3]++))) w[$3] = $5; if ($8 == 1) un[$3] = 1; cnt[$3]++ }
    END { for (i = 1; i <= n; i++) { p = order[i]; print st[p], nm[p], p, (cnt[p] > 1 ? cnt[p] " sessions" : ag[p]), w[p], un[p], pin[p] } }'
}

# session_rows <cmd_list output> <workspace row> — for a multi-session worktree, one
# indented row per tab with its own state.
session_rows() {
  local list="$1"
  local -a f=("${(@ps:\t:)2}")
  [[ "${f[4]}" == *" sessions" ]] || return 0
  local state name ws_path agent win pinned active unread tab
  print -r -- "$list" | while IFS=$'\t' read -r state name ws_path agent win pinned active unread; do
    [[ "$ws_path" == "${f[3]}" ]] || continue
    tab="$agent"
    [[ "$name" == *"#"* ]] && tab+="#${name##*#}"
    printf '%17s%s └ %s %-22s \e[2m%s\e[0m\t%s\t%s\n' "" "$(unread_mark "$unread")" "$(state_icon "$state")" "$tab" "$state" "$ws_path" "$win"
  done
}

# picker_row <workspace row> <pinned|first|""> — one display line
picker_row() {
  local -a f=("${(@ps:\t:)1}")
  local repo="${f[2]%%/*}" branch="${f[2]#*/}" detail="${f[4]}" mark="  " repo_style=$'\e[2m'
  [[ "$detail" == "-" ]] && detail="${f[1]}"
  case "$2" in
  pinned) mark="📌" repo_style=$'\e[1m' ;;
  first) repo_style=$'\e[1m' ;;
  esac
  printf '%s %s%-14s\e[0m %s %-24s %s \e[2m%s\e[0m\t%s\t%s\n' \
    "$mark" "$repo_style" "$repo" "$(unread_mark "${f[6]}")" "$branch" "$(state_icon "${f[1]}")" "$detail" "${f[3]}" "${f[5]}"
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

unread_mark() { [[ "$1" == 1 ]] && print -n $'\e[1;36m•\e[0m' || print -n ' '; }

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

# Candidates: current repo, ~/.loop/repos.json, then repos found under WS_REPO_ROOTS. Enter
# with no match takes the typed text as a path (~ expanded); cmd_new validates it.
pick_repo() {
  command -v fzf >/dev/null || die "fzf not installed (or pass --repo)"
  local here sel
  here="$(main_checkout "$PWD" 2>/dev/null)"
  sel="$({
    [[ -n "$here" ]] && print -r -- "$here"
    [[ -f "$REGISTRY" ]] && jq -r '.[]' "$REGISTRY"
    scan_repos
  } | awk 'NF && !seen[$0]++' | while read -r p; do [[ -d "$p" ]] && print -r -- "$p"; done \
    | fzf --prompt 'repo (or type a path)> ' --no-sort --print-query --bind 'enter:accept-or-print-query')" \
    || [[ -n "$sel" ]] || return 1
  local -a lines=("${(@f)sel}") # --print-query: query line, then the match (if any)
  sel="${lines[-1]}"
  print -r -- "${sel/#\~/$HOME}"
}

# Git repos (real .git dirs, so not worktrees) at or under each root, skipping dependency dirs.
scan_repos() {
  local root
  for root in ${(z)WS_REPO_ROOTS}; do
    root="${~root}"
    [[ -d "$root" ]] || continue
    [[ -d "$root/.git" ]] && print -r -- "$root"
    find "$root" -mindepth 2 -maxdepth 5 \( -name node_modules -o -name .venv -o -name vendor \) -prune \
      -o -name .git -type d -print -prune 2>/dev/null | sed 's|/\.git$||' | sort
  done
}

check_agent() { (( ${AGENTS[(Ie)$1]} )) || die "agent must be one of: ${(j:, :)AGENTS}"; }

# Main checkout of a repo (or of any of its worktrees): parent of the common git dir.
main_checkout() {
  local common
  common="$(git -C "$1" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || return 1
  [[ "${common:t}" == ".git" ]] || return 1
  print -r -- "${common:h}"
}

# Existing local branch → reuse; remote-only → track it; else branch off the default branch.
add_worktree() {
  local repo="$1" branch="$2" wt="$3" base
  mkdir -p "${wt:h}"
  git -C "$repo" fetch -q origin 2>/dev/null
  if git -C "$repo" show-ref -q --verify "refs/heads/$branch"; then
    git -C "$repo" worktree add -q "$wt" "$branch"
  elif git -C "$repo" show-ref -q --verify "refs/remotes/origin/$branch"; then
    git -C "$repo" worktree add -q --track -b "$branch" "$wt" "origin/$branch"
  else
    base="$(git -C "$repo" symbolic-ref -q --short refs/remotes/origin/HEAD 2>/dev/null)" || base=HEAD
    git -C "$repo" worktree add -q -b "$branch" "$wt" "$base"
  fi
}

# Copy gitignored files (default .env*) from the main checkout. Honors Conductor's
# .conductor/settings.toml file_include_globs so migrated repos behave the same.
copy_included_files() {
  local repo="$1" wt="$2" f pat
  local -a globs=("${(@f)$(conductor_setting "$repo" file_include_globs)}")
  globs=(${globs:#})
  (( ${#globs} )) || globs=("${DEFAULT_INCLUDE_GLOBS[@]}")
  git -C "$repo" ls-files -z --others --ignored --exclude-standard --directory \
    | while IFS= read -r -d '' f; do
      [[ "$f" == */ ]] && continue
      for pat in "${globs[@]}"; do
        if [[ "$pat" == */* && "$f" == ${~pat} ]] || [[ "$pat" != */* && "${f:t}" == ${~pat} ]]; then
          mkdir -p "$wt/${f:h}" && cp -p "$repo/$f" "$wt/$f"
          break
        fi
      done
    done
}

# Setup command: .conductor/settings.toml [scripts] setup, else an executable .ws-setup.
setup_script() {
  local repo="$1" setup
  setup="$(conductor_setting "$repo" setup)"
  if [[ -n "$setup" ]]; then
    print -r -- "$setup"
  elif [[ -x "$repo/.ws-setup" ]]; then
    print -r -- "${(q)repo}/.ws-setup"
  fi
}

# Setup runs in its own background window next to the agent: closes on success, stays
# open on failure so the error is visible.
run_setup() {
  local wt="$1" setup="$2"
  [[ -n "$setup" ]] || return 0
  tmux_ new-window -d -t "$(sessions_for "$wt" | head -1):" -n setup -c "$wt" \
    ${(z)WS_AGENT_SHELL} "$setup || { print \"setup failed (\$?) — enter to close\"; read; }"
}

# Minimal reader for the two Conductor keys ws uses (string or """multiline""" value).
conductor_setting() {
  local file="$1/.conductor/settings.toml" key="$2"
  [[ -f "$file" ]] || return 0
  awk -v key="$key" '
    multi { if ($0 ~ /"""/) { sub(/""".*/, ""); if ($0 != "") print; exit } print; next }
    $0 ~ "^[ \t]*" key "[ \t]*=" {
      sub(/^[^=]*=[ \t]*/, "")
      if ($0 ~ /^"""/) { sub(/^"""/, ""); if ($0 ~ /"""/) { sub(/""".*/, ""); print; exit } if ($0 != "") print; multi = 1; next }
      sub(/^"/, ""); sub(/"[ \t]*(#.*)?$/, ""); print; exit
    }' "$file"
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

session_remove() {
  local wt="$1" slot="$2" f
  f="$(sessions_file "$wt")" || return 0
  [[ -f "$f" ]] || return 0
  with_lock "$f" session_remove_locked "$f" "$slot"
}

session_remove_locked() {
  local f="$1" slot="$2" id
  id="$(awk -F '\t' -v s="$slot" '$1 == s {print $4}' "$f")"
  rewrite "$f" -F '\t' -v s="$slot" '$1 != s' || return 1
  [[ "$id" != <-> ]] || ws_bus_close "$id"
  return 0
}

# Stores the tab's resume id, re-adding its row if it went missing (a session must stay
# resumable after a reboot). Skips the write when nothing changed — hooks fire on every tool.
# slot/id are read before the lock: a stale hook after slot reuse must never take over the
# row of the slot's new owner, so only an unowned row or our own is updated.
record_resume_id() {
  local sid wt slot agent f id
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
  if [[ "$id" == <-> && -f "$WS_SESSIONS/$id/meta" && "$(jq -r '.resume_id' "$WS_SESSIONS/$id/meta")" != "$sid" ]]; then
    ws_bus_update "$id" --arg resume "$sid" '.resume_id = $resume' || return 0
  fi
  grep -qxF -- "$slot	${agent:-claude}	$sid	$id" "$f" && return 0
  with_lock "$f" rewrite "$f" -F '\t' -v OFS='\t' -v s="$slot" -v a="${agent:-claude}" -v id="$sid" -v g="$id" \
    '$1 == s { found = 1; if ($4 == "" || $4 == g) { $3 = id; if (g != "") $4 = g } } { print } END { if (!found) print s, a, id, g }'
}

# with_lock <file> <cmd...> — runs cmd holding <file>.lock: parallel tool calls fire hooks
# concurrently, and unserialized rewrites of the sessions file wipe its rows.
with_lock() {
  local f="$1"
  shift
  : >> "$f.lock" # flock won't create it
  (zsystem flock -t 3 -i 0.01 "$f.lock" 2>/dev/null || exit 1; "$@")
}

# rewrite <file> <awk args...> — replaces the file with awk's output, atomically.
rewrite() {
  local f="$1"
  shift
  awk "$@" "$f" > "$f.tmp" && mv "$f.tmp" "$f"
}

unpin() {
  [[ -f "$PINS" ]] || return 0
  grep -vxF -- "$1" "$PINS" > "$PINS.tmp"
  mv "$PINS.tmp" "$PINS"
}

# The workspace you're in: the current dir's worktree, else the current tmux session's.
current_workspace() {
  local top
  top="$(git rev-parse --show-toplevel 2>/dev/null)"
  if [[ -n "$top" && "${top:A}" == "${WS_ROOT:A}"/*/* ]]; then
    print -r -- "${top:A}"
  elif [[ -n "${TMUX:-}" ]]; then
    top="$(tmux_ display -p '#{@ws_path}' 2>/dev/null)"
    [[ -n "$top" ]] && print -r -- "$top"
  else
    return 1
  fi
}

resolve_workspace() {
  local t="${1:-}"
  [[ -n "$t" ]] || return 1
  [[ "$t" == /* ]] || t="$WS_ROOT/$t"
  [[ -d "$t" && "$t" == "$WS_ROOT"/*/* ]] || return 1
  print -r -- "${t:A}"
}

workspace_name() { print -r -- "${1:h:t}/${1:t}"; }

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

# emit_notify_event <state> <prev> <payload> <seq> [target] — send one v1 JSON event on stdin
# to the configured adapter. target (window/pane) defaults to the hook's pane. `closed` marks
# the tab's notification dismissed; any other state marks it open (see cmd_dismiss).
emit_notify_event() {
  (( WS_NOTIFY )) || return 0
  local target="${5:-$TMUX_PANE}" focused
  focused="$(tmux_ display -p -t "$target" '#{&&:#{window_active},#{session_attached}}' 2>/dev/null)"
  [[ "$focused" == 1 ]] && focused=true || focused=false
  local win sess click wt agent branch detail event sound="" name
  win="$(tmux_ display -p -t "$target" '#{window_id}')"
  sess="$(tmux_ display -p -t "$target" '#{session_id}')"
  wt="$(tmux_ display -p -t "$target" '#{@ws_path}')"
  agent="$(tmux_ display -p -t "$target" '#{@ws_agent}')"
  name="${WS_WORKSPACE:-$(workspace_name "$wt")}"
  local msg="$name is $1"
  [[ "$1" == waiting ]] && msg="$name needs input"
  [[ "$1" == closed ]] && msg="$name dismissed"
  tmux_ set -w -t "$win" @ws_notify_open "$([[ "$1" == closed ]] && print 0 || print 1)" 2>/dev/null
  branch="$(git -C "$wt" branch --show-current 2>/dev/null)"
  detail="$(hook_detail_json "${3:-}")"
  [[ -n "$detail" ]] || detail='""'
  click="${(q)$(command -v tmux)}${WS_TMUX_SOCKET:+ -L ${(q)WS_TMUX_SOCKET}} select-window -t ${(q)win} \\; switch-client -t ${(q)sess}"
  local bundle="${WS_TERMINAL_BUNDLE:-$(tmux_ show-environment -g __CFBundleIdentifier 2>/dev/null | cut -d= -f2)}"
  [[ -n "$bundle" ]] && click+=" ; open -b ${(q)bundle}"
  [[ "$1" == idle ]] && sound=done
  [[ "$1" == waiting ]] && sound=waiting
  event="$(jq -nc --arg id "ws:$win" --arg state "$1" --arg prev "$2" \
    --arg message "$msg" --arg agent "$agent" --arg repo "${name%%/*}" \
    --arg branch "$branch" --argjson detail "$detail" --arg sound "$sound" --arg command "$click" \
    --argjson focused "$focused" --argjson seq "${4:-0}" --argjson ts "$(date +%s)" \
    '{v:1, id:$id, source:"ws", state:$state, prev:$prev, title:"ws", message:$message,
      detail:$detail, agent:$agent, repo:$repo, branch:$branch, focused:$focused, sound:$sound,
      actions:[{id:"focus", label:"Focus tab", command:$command}], seq:$seq, ts:$ts}')" || return 0
  { print -r -- "$event" | notify_adapter; } >/dev/null 2>&1 &!
}

# Keep detail JSON-encoded so command substitution preserves trailing newlines in the text.
hook_detail_json() {
  print -r -- "$1" | jq -cs '
    (if length == 1 then .[0] else {} end)
    | if type == "object" then
        [.message, .last_assistant_message, .prompt]
        | map(select(type == "string") | select(length > 0))
        | (first // "") | .[:1024]
      else "" end
  ' 2>/dev/null
}

notify_adapter() {
  case "$WS_NOTIFIER" in
  boringnotch)
    local event="$(cat)"
    # The notch app owns sound (its Settings); only the fallback plays ws cues.
    print -r -- "$event" | nc -U -w 1 "$WS_BORINGNOTCH_SOCKET" >/dev/null 2>&1 \
      || print -r -- "$event" | WS_NOTIFIER=auto notify_adapter
    ;;
  auto | terminal-notifier | osascript)
    local event="$(cat)" lockfd
    notify_claim "$event" || return 0
    if [[ "$(print -r -- "$event" | jq -r '.state')" == closed ]]; then
      [[ "$WS_NOTIFIER" == osascript ]] || print -r -- "$event" | terminal_notifier_remove
      return 0
    fi
    print -r -- "$event" | jq -e '
      (.state == "idle" or .state == "waiting") and .focused == false
    ' >/dev/null 2>&1 || return 0
    notify_sound "$(print -r -- "$event" | jq -r '.sound')"
    tmux_ display-message "ws: $(print -r -- "$event" | jq -r '.message')" 2>/dev/null
    if [[ "$WS_NOTIFIER" == osascript ]]; then
      print -r -- "$event" | osascript_notify
    else
      print -r -- "$event" | terminal_notifier_notify
    fi
    ;;
  *)
    if [[ "$WS_NOTIFIER" == */* && -x "$WS_NOTIFIER" ]]; then
      "$WS_NOTIFIER"
    else
      tmux_ display-message "ws: unknown WS_NOTIFIER '$WS_NOTIFIER', using auto" 2>/dev/null
      WS_NOTIFIER=auto notify_adapter
    fi
    ;;
  esac
}

# notify_claim <event> — per-tab delivery watermark for the built-in notifiers: under the tab's
# notify lock (left open in the caller's lockfd, so posts serialize until the adapter exits),
# fail if a same-or-newer seq was already claimed, else record this one — every state, so a
# newer `working` still supersedes an older banner. No seq (custom/legacy) counts as newest.
notify_claim() {
  local id seq win lock last
  print -r -- "$1" | jq -r '.id // "", .seq // 0' | { read -r id; read -r seq; }
  win="${id#ws:}"
  [[ "$id" == ws:?* && "$seq" == <1-> ]] || return 0
  zmodload zsh/system 2>/dev/null
  lock="${TMPDIR:-/tmp}/ws-notify-${WS_TMUX_SOCKET:-default}-$win.lock"
  { : >>"$lock" && zsystem flock -t 2 -i 0.01 -f lockfd "$lock"; } 2>/dev/null || return 1
  last="$(tmux_ display -p -t "$win" '#{@ws_notify_seq}' 2>/dev/null)"
  [[ "$last" == <-> ]] && (( seq <= last )) && return 1
  tmux_ set -w -t "$win" @ws_notify_seq "$seq" 2>/dev/null
  return 0
}

# Play once outside notification transport so a fallback cannot duplicate the sound.
notify_sound() {
  local sound
  case "$1" in
  done) sound="$WS_NOTIFY_SOUND_DONE" ;;
  waiting) sound="$WS_NOTIFY_SOUND_WAITING" ;;
  *) return 0 ;;
  esac
  if [[ ! -f "$sound" && "$sound" != */* ]]; then
    local -a found=({$HOME/Library/Sounds,/Library/Sounds,/System/Library/Sounds}/$sound.*(N.))
    sound="${found[1]:-}"
  fi
  # Detached: a long cue must not delay (and reorder) the notification.
  [[ -n "$sound" && -f "$sound" ]] && afplay "$sound" >/dev/null 2>&1 &!
  return 0
}

# Preserve macOS icon/group/click behavior, preferring the branded ws.app.
# Fall back to osascript if terminal-notifier is missing or macOS rejects it.
terminal_notifier_notify() {
  local event="$(cat)"
  local notifier="$WS_NOTIFIER_APP/Contents/MacOS/terminal-notifier"
  [[ -x "$notifier" ]] || notifier="$(command -v terminal-notifier)"
  if [[ -n "$notifier" ]]; then
    local msg title id bundle click image="$WS_NOTIFY_ICON_DONE"
    local -a icon=()
    [[ "$(print -r -- "$event" | jq -r '.sound')" == waiting ]] \
      && image="$WS_NOTIFY_ICON_WAITING"
    [[ -f "$image" ]] && icon=(-contentImage "$image") # app icon can't be overridden on modern macOS
    msg="$(print -r -- "$event" | jq -r '.message')"
    title="$(print -r -- "$event" | jq -r '.title')"
    id="$(print -r -- "$event" | jq -r '.id')"
    click="$(print -r -- "$event" | jq -r '.actions[] | select(.id == "focus") | .command')"
    bundle="${WS_TERMINAL_BUNDLE:-$(tmux_ show-environment -g __CFBundleIdentifier 2>/dev/null | cut -d= -f2)}"
    "$notifier" -title "$title" -message "$msg" -group "${id/ws:/ws-}" "${icon[@]}" \
      ${bundle:+-activate} ${bundle:+$bundle} -execute "$click" >/dev/null 2>&1 && return 0
  fi
  print -r -- "$event" | osascript_notify
}

# Remove the tab's banner (osascript notifications can't be withdrawn).
terminal_notifier_remove() {
  local notifier="$WS_NOTIFIER_APP/Contents/MacOS/terminal-notifier" id
  [[ -x "$notifier" ]] || notifier="$(command -v terminal-notifier)"
  [[ -n "$notifier" ]] || return 0
  id="$(jq -r '.id')"
  "$notifier" -remove "${id/ws:/ws-}" >/dev/null 2>&1
  return 0
}

osascript_notify() {
  local event="$(cat)"
  local msg="$(print -r -- "$event" | jq -r '.message')" title="$(print -r -- "$event" | jq -r '.title')"
  osascript -e 'on run argv' -e 'display notification (item 1 of argv) with title (item 2 of argv)' \
    -e 'end run' "$msg" "$title" >/dev/null 2>&1
}

# The hook's tab is on screen: current in its session, and that session attached.
pane_focused() {
  [[ "$(tmux_ display -p -t "$TMUX_PANE" '#{&&:#{window_active},#{session_attached}}' 2>/dev/null)" == 1 ]]
}

# Copies terminal-notifier.app to $WS_HOME/ws.app with its own name, bundle id and icon, so
# notifications come from "ws" (own entry in System Settings → Notifications: allow it there,
# pick Alerts to keep them on screen). Re-run after `brew upgrade terminal-notifier`.
build_notifier() {
  [[ -d "$WS_NOTIFIER_SOURCE" ]] || die "terminal-notifier.app not found: $WS_NOTIFIER_SOURCE (brew install terminal-notifier)"
  local app="$WS_NOTIFIER_APP" plist="$WS_NOTIFIER_APP/Contents/Info.plist" iconset size
  rm -rf "$app"
  mkdir -p "$WS_HOME"
  cp -R "$WS_NOTIFIER_SOURCE" "$app" || die "copy failed"
  chmod -R u+w "$app"
  /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier sh.ratel.ws.notifier" \
    -c "Set :CFBundleName ws" "$plist" || die "cannot edit $plist"
  iconset="$(mktemp -d)/ws.iconset"
  mkdir -p "$iconset"
  for size in 16 32 128 256 512; do
    sips -z $size $size "$WS_APP_ICON" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) "$WS_APP_ICON" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$iconset" -o "$app/Contents/Resources/Terminal.icns" || die "iconutil failed"
  rm -rf "${iconset:h}"
  codesign --force --deep --sign - "$app" >/dev/null 2>&1 || print -u2 "ws: codesign failed (notifications may be blocked)"
  touch "$app" # nudge LaunchServices to pick up the new icon
  print -r -- "built $app"
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

preview() {
  local wt="${1:-}" win="${2:-}"
  if [[ -n "$win" ]]; then
    tmux_ capture-pane -e -p -t "$win"
  elif [[ -d "$wt" ]]; then
    print -r -- "stopped — enter restores its sessions"
    git -C "$wt" log --oneline -10 --color=always
    git -C "$wt" status -s
  fi
}

# Replaces the process (attach from a plain terminal); `exec` can't target the tmux_ function.
exec_tmux() {
  if [[ -n "${WS_TMUX_SOCKET:-}" ]]; then
    exec tmux -L "$WS_TMUX_SOCKET" "$@"
  fi
  exec tmux "$@"
}

tmux_() {
  if [[ -n "${WS_TMUX_SOCKET:-}" ]]; then
    command tmux -L "$WS_TMUX_SOCKET" "$@"
  else
    command tmux "$@"
  fi
}

die() { print -u2 "ws: $1"; exit "${2:-1}"; }

[[ "$ZSH_EVAL_CONTEXT" == *:file ]] || main "$@" # sourced (tests): functions only
