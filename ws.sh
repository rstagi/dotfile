#!/bin/zsh
set -u -o pipefail

# ws — bare-minimum terminal Conductor: one git worktree + one tmux session per workspace,
# one tab (window) per agent session inside it. Status and resume ids come from Claude Code /
# Codex hooks — no screen scraping.
#
#   ws                         picker (attaches when run outside tmux)
#   ws new [--repo P] [--branch B] [--agent claude|codex] [--detach]
#   ws add [name|path] [--agent claude|codex] [--detach]   another agent tab (default: current worktree)
#   ws open <name|path> [--detach]   focus, or restore every recorded session (resumed by id)
#   ws pick                    grouped picker: enter open · ctrl-n new · ctrl-o new codex ·
#                              ctrl-a add claude tab · ctrl-t add codex tab · ctrl-p pin · ctrl-x rm ·
#                              ctrl-s show/hide sessions
#   ws pin <name|path>         toggle pin (pinned workspaces are listed first)
#   ws list                    TSV: state name path agent window pinned
#   ws rm <name|path> [--force] [--delete-branch]
#   ws merge                   in a workspace: merge its PR (gh, $WS_MERGE_METHOD, default
#                              squash), fast-forward the main checkout, then on confirm
#                              remove worktree + tabs + local/remote branch (prefix+M)
#   ws hook <working|idle|waiting>   called by agent hooks inside a session window
#
# Sessions live in <worktree git dir>/ws-sessions (slot, agent, resume id). Quitting an agent
# cleanly forgets its session; a killed one (reboot, kill-window) is resumed on `ws open`.

WS_BIN="${0:A}"
WS_ROOT="${WS_ROOT:-$HOME/.ws/worktrees}"
WS_HOME="${WS_HOME:-$HOME/.ws}"
WS_NOTIFY="${WS_NOTIFY:-1}"
# Notification sound (/System/Library/Sounds or ~/Library/Sounds) + icon per state.
WS_NOTIFY_SOUND_DONE="${WS_NOTIFY_SOUND_DONE:-Glass}"
WS_NOTIFY_SOUND_WAITING="${WS_NOTIFY_SOUND_WAITING:-Ping}"
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
  rm) cmd_rm "$@" ;;
  merge) cmd_merge ;;
  _merge_popup) cmd_merge; print -n "\npress enter to close"; read -r _ ;;
  hook) cmd_hook "$@" ;;
  _rows) picker_rows ;;
  _summary) picker_summary ;;
  _toggle_sessions) toggle_sessions ;;
  _preview) preview "$@" ;;
  _end) session_remove "$@" ;;
  _build-notifier) build_notifier ;;
  -h | --help | help) sed -n '4,20p' "$WS_BIN" | sed 's/^# \{0,1\}//' ;;
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
  local win
  win="$(open_session "$wt" "$(session_add "$wt" "$agent")" "$agent" "")"
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
  local wt win
  wt="$(resolve_workspace "$target")" || die "no such workspace: $target"
  win="$(open_session "$wt" "$(session_add "$wt" "$agent")" "$agent" "")"
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
  local wt slot agent resume
  wt="$(resolve_workspace "$target")" || die "no such workspace: $target"
  if [[ -z "$(windows_for "$wt")" ]]; then
    [[ -n "$(sessions_read "$wt")" ]] || session_add "$wt" claude >/dev/null
    sessions_read "$wt" | while IFS=$'\t' read -r slot agent resume; do
      open_session "$wt" "$slot" "$agent" "$resume" >/dev/null
    done
  fi
  (( detach )) || focus "$(windows_for "$wt" | head -1)"
}

cmd_pick() {
  command -v fzf >/dev/null || die "fzf not installed"
  local out key row wt win # never `path`: zsh ties it to $PATH
  out="$(picker_rows | fzf --ansi --delimiter '\t' --with-nth 1 --no-sort --layout reverse \
    --header-first --bind "start,load:transform-header($WS_BIN _summary)" \
    --preview "$WS_BIN _preview {2} {3}" --preview-window 'right,50%,follow' \
    --expect ctrl-n,ctrl-o,ctrl-a,ctrl-t \
    --bind "ctrl-p:execute-silent($WS_BIN pin {2})+reload($WS_BIN _rows)" \
    --bind "ctrl-s:execute-silent($WS_BIN _toggle_sessions)+reload($WS_BIN _rows)" \
    --bind "ctrl-x:execute($WS_BIN rm --interactive {2})+reload($WS_BIN _rows)")" || return 0
  key="${out%%$'\n'*}"
  row=""
  [[ "$out" == *$'\n'* ]] && row="${out#*$'\n'}"
  wt="$(print -r -- "$row" | cut -f2)"
  win="$(print -r -- "$row" | cut -f3)"
  case "$key" in
  ctrl-n) cmd_new ;;
  ctrl-o) cmd_new --agent codex ;;
  ctrl-a) [[ -n "$wt" ]] && cmd_add "$wt" ;;
  ctrl-t) [[ -n "$wt" ]] && cmd_add "$wt" --agent codex ;;
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
      local -a o=("${(@ps:\t:)$(tmux_ display -p -t "$win" '#{@ws_state}	#{@ws_slot}	#{@ws_agent}')}")
      printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
        "${o[1]}" "$name${${o[2]:#1}:+#${o[2]}}" "$wt" "${o[3]}" "$win" "$pinned"
    done
    (( found )) || printf 'stopped\t%s\t%s\t-\t\t%s\n' "$name" "$wt" "$pinned"
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
  local wt repo branch default ans
  wt="$(current_workspace)" && wt="$(resolve_workspace "$wt")" || die "merge: not inside a workspace"
  repo="$(main_checkout "$wt")" || die "cannot find main checkout for $wt"
  branch="$(git -C "$wt" branch --show-current)"
  [[ -n "$branch" ]] || die "merge: detached HEAD in $wt"

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

  read -r "ans?delete $(workspace_name "$wt") (worktree, tabs, local + remote $branch)? [y/N] " || ans=""
  [[ "$ans" == (y|Y) ]] || { print "kept $(workspace_name "$wt")"; return 0; }
  git -C "$repo" push -q origin --delete "$branch" 2>/dev/null # GitHub may have auto-deleted it
  cmd_rm "$wt" --force-delete-branch
}

# Agent hooks call this from inside a session window: sets the window's state and records
# the agent's session id (for resume). Best effort, always exits 0 — never blocks the agent.
cmd_hook() {
  local state="${1:-}" payload=""
  [[ -t 0 ]] || payload="$(cat)"
  [[ -n "${WS_WORKSPACE:-}" && -n "${TMUX_PANE:-}" ]] || return 0
  [[ "$state" == (working|idle|waiting) ]] || return 0
  local prev
  prev="$(tmux_ display -p -t "$TMUX_PANE" '#{@ws_state}' 2>/dev/null)"
  tmux_ set -w -t "$TMUX_PANE" @ws_state "$state" 2>/dev/null
  record_resume_id "$payload"
  [[ "$state" != "$prev" && "$state" != working ]] && notify_unless_focused "$state"
  return 0
}

# prefix+w → ws picker, prefix+N → new workspace, prefix+a / prefix+A → claude / codex tab in
# the current worktree, prefix+M → ws merge. Only inside ws sessions (they carry the @ws_path session option);
# other sessions keep tmux's default choose-tree on prefix+w.
configure_bindings() {
  tmux_ has-session 2>/dev/null || return 0 # no server yet: bound when the first session is created
  local in_ws="#{!=:#{@ws_path},}"
  tmux_ bind-key w if-shell -F "$in_ws" \
    "display-popup -E -w 90% -h 85% -d '#{pane_current_path}' '$WS_BIN pick'" "choose-tree -Zw"
  tmux_ bind-key N if-shell -F "$in_ws" \
    "display-popup -E -w 60% -h 50% -d '#{pane_current_path}' '$WS_BIN new'"
  tmux_ bind-key M if-shell -F "$in_ws" \
    "display-popup -E -w 70% -h 50% -d '#{pane_current_path}' '$WS_BIN _merge_popup'"
  tmux_ bind-key a if-shell -F "$in_ws" "run-shell \"'$WS_BIN' add '#{@ws_path}'\""
  tmux_ bind-key A if-shell -F "$in_ws" "run-shell \"'$WS_BIN' add '#{@ws_path}' --agent codex\""
}

# Opens one agent tab in the worktree's tmux session (creating the session if needed) and
# prints its window id. The tab closes with the agent; a clean exit (0) forgets the session,
# anything else (crash, SIGHUP) keeps it for resume.
open_session() {
  local wt="$1" slot="$2" agent="$3" resume="$4"
  local name tab cmd run sess win
  name="$(workspace_name "$wt")"
  tab="$agent"
  (( slot > 1 )) && tab+="#$slot"
  cmd="$(agent_command "$agent" "$resume")"
  run="$cmd; s=\$?; if (( s == 0 )); then ${(q)WS_BIN} _end ${(q)wt} $slot; else print \"agent exited (\$s) — enter to close\"; read; fi"
  local -a spawn=(-d -P -F '#{window_id}' -n "$tab" -c "$wt" -e "WS_WORKSPACE=$name" ${(z)WS_AGENT_SHELL} "$run")
  sess="$(sessions_for "$wt" | head -1)"
  if [[ -n "$sess" ]]; then
    win="$(tmux_ new-window -t "$sess:" "${spawn[@]}")"
  else
    win="$(tmux_ new-session -s "${name//[.:]/_}" "${spawn[@]}")"
    configure_bindings
    sess="$(tmux_ display -p -t "$win" '#{session_id}')"
    tmux_ set -t "$sess" @ws_path "$wt"
    tmux_ set -t "$sess" detach-on-destroy off
    tmux_ set -t "$sess" status-left-length 60
    tmux_ set -t "$sess" status-left "#[bold] $name #[default]"
  fi
  tmux_ set -w -t "$win" @ws_path "$wt"
  tmux_ set -w -t "$win" @ws_slot "$slot"
  tmux_ set -w -t "$win" @ws_agent "$agent"
  tmux_ set -w -t "$win" @ws_state idle
  tmux_ set -w -t "$win" automatic-rename off
  tmux_ set -w -t "$win" window-status-format "#I $(state_format) #W"
  tmux_ set -w -t "$win" window-status-current-format "#[bold]#I $(state_format) #W#[default]"
  print -r -- "$win"
}

agent_command() {
  local agent="$1" resume="$2"
  case "$agent" in
  claude)
    write_claude_settings
    local cmd="claude --dangerously-skip-permissions --settings ${(q)WS_HOME}/claude-settings.json"
    [[ -n "$resume" ]] && cmd+=" --resume ${(q)resume}"
    print -r -- "$cmd"
    ;;
  codex)
    # Hook-trust bypass: ws hooks are passed per launch, so they'd otherwise need re-trusting.
    local cmd="codex" ev st
    [[ -n "$resume" ]] && cmd+=" resume ${(q)resume}"
    cmd+=" --dangerously-bypass-approvals-and-sandbox --dangerously-bypass-hook-trust"
    for ev st in UserPromptSubmit working PostToolUse working PermissionRequest waiting Stop idle Interrupt idle; do
      cmd+=" -c 'hooks.$ev=[{hooks=[{type=\"command\",command=\"$WS_BIN hook $st\"}]}]'"
    done
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
      PostToolUse: run("working"),
      Stop: run("idle"),
      PermissionRequest: run("waiting"),
      Notification: [{matcher: "permission_prompt|elicitation_dialog",
                      hooks: [{type: "command", command: "\($h) waiting", timeout: 5}]}]
    }}' > "$WS_HOME/claude-settings.json"
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
  print -r -- "${(j: · :)parts:-no workspaces}"
  print -r -- $'\e[2m↵ open · ^s sessions · ^p pin · ^x rm\e[0m'
  print -r -- $'\e[2m^n/^o new claude/codex · ^a/^t add claude/codex tab\e[0m'
}

toggle_sessions() {
  mkdir -p "$WS_HOME"
  if [[ -e "$SHOW_SESSIONS" ]]; then rm -f "$SHOW_SESSIONS"; else touch "$SHOW_SESSIONS"; fi
}

# cmd_list rows (stdin) collapsed to one per worktree: most urgent state, agent (or "N
# sessions"), first window.
workspace_rows() {
  awk -F '\t' -v OFS='\t' '
    function rank(s) { return s == "waiting" ? 3 : s == "working" ? 2 : s == "idle" ? 1 : 0 }
    !($3 in seen) { seen[$3] = 1; order[++n] = $3; st[$3] = $1; nm[$3] = $2; ag[$3] = $4; w[$3] = $5; pin[$3] = $6; cnt[$3] = 1; sub(/#.*/, "", nm[$3]); next }
    { if (rank($1) > rank(st[$3])) st[$3] = $1; cnt[$3]++ }
    END { for (i = 1; i <= n; i++) { p = order[i]; print st[p], nm[p], p, (cnt[p] > 1 ? cnt[p] " sessions" : ag[p]), w[p], pin[p] } }'
}

# session_rows <cmd_list output> <workspace row> — for a multi-session worktree, one
# indented row per tab with its own state.
session_rows() {
  local list="$1"
  local -a f=("${(@ps:\t:)2}")
  [[ "${f[4]}" == *" sessions" ]] || return 0
  local state name ws_path agent win pinned tab
  print -r -- "$list" | while IFS=$'\t' read -r state name ws_path agent win pinned; do
    [[ "$ws_path" == "${f[3]}" ]] || continue
    tab="$agent"
    [[ "$name" == *"#"* ]] && tab+="#${name##*#}"
    printf '%19s└ %s %-22s \e[2m%s\e[0m\t%s\t%s\n' "" "$(state_icon "$state")" "$tab" "$state" "$ws_path" "$win"
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
  printf '%s %s%-14s\e[0m %-24s %s \e[2m%s\e[0m\t%s\t%s\n' \
    "$mark" "$repo_style" "$repo" "$branch" "$(state_icon "${f[1]}")" "$detail" "${f[3]}" "${f[5]}"
}

focus() {
  local win="$1" sess
  [[ -n "$win" ]] || return 1
  sess="$(tmux_ display -p -t "$win" '#{session_id}')" || return 1
  tmux_ select-window -t "$win"
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

check_agent() { [[ "$1" == (claude|codex) ]] || die "agent must be claude or codex"; }

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
  local wt="$1" agent="$2" f slot
  f="$(sessions_file "$wt")"
  slot="$( { [[ -f "$f" ]] && cut -f1 "$f"; print 0; } | sort -n | tail -1)"
  slot=$((slot + 1))
  printf '%s\t%s\t\n' "$slot" "$agent" >> "$f"
  print -r -- "$slot"
}

session_remove() {
  local wt="$1" slot="$2" f
  f="$(sessions_file "$wt")" || return 0
  [[ -f "$f" ]] || return 0
  awk -F '\t' -v s="$slot" '$1 != s' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
}

record_resume_id() {
  local sid wt slot f
  sid="$(print -r -- "$1" | jq -r '.session_id // empty' 2>/dev/null)"
  [[ -n "$sid" ]] || return 0
  wt="$(tmux_ display -p -t "$TMUX_PANE" '#{@ws_path}' 2>/dev/null)"
  slot="$(tmux_ display -p -t "$TMUX_PANE" '#{@ws_slot}' 2>/dev/null)"
  [[ -n "$wt" && -n "$slot" ]] || return 0
  f="$(sessions_file "$wt")" || return 0
  [[ -f "$f" ]] || return 0
  awk -F '\t' -v OFS='\t' -v s="$slot" -v id="$sid" '$1 == s { $3 = id } { print }' "$f" > "$f.tmp" \
    && mv "$f.tmp" "$f"
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

sessions_for() {
  tmux_ list-sessions -F '#{session_id}	#{@ws_path}' 2>/dev/null \
    | awk -F '\t' -v p="$1" '$2 == p { print $1 }'
}

# terminal-notifier (sound, click → terminal app to front + tmux on the tab, one notification
# per tab); osascript fallback when it's missing or not yet allowed by macOS.
notify_unless_focused() {
  (( WS_NOTIFY )) || return 0
  local focused
  focused="$(tmux_ display -p -t "$TMUX_PANE" '#{&&:#{window_active},#{session_attached}}' 2>/dev/null)"
  [[ "$focused" == 1 ]] && return 0
  local msg="$WS_WORKSPACE is $1"
  [[ "$1" == waiting ]] && msg="$WS_WORKSPACE needs input"
  tmux_ display-message "ws: $msg" 2>/dev/null
  local notifier="$WS_NOTIFIER_APP/Contents/MacOS/terminal-notifier"
  [[ -x "$notifier" ]] || notifier="$(command -v terminal-notifier)"
  if [[ -n "$notifier" ]]; then
    local win sess bundle click sound="$WS_NOTIFY_SOUND_DONE" image="$WS_NOTIFY_ICON_DONE"
    local -a icon=()
    [[ "$1" == waiting ]] && sound="$WS_NOTIFY_SOUND_WAITING" image="$WS_NOTIFY_ICON_WAITING"
    [[ -f "$image" ]] && icon=(-contentImage "$image") # app icon can't be overridden on modern macOS
    win="$(tmux_ display -p -t "$TMUX_PANE" '#{window_id}')"
    sess="$(tmux_ display -p -t "$TMUX_PANE" '#{session_id}')"
    bundle="${WS_TERMINAL_BUNDLE:-$(tmux_ show-environment -g __CFBundleIdentifier 2>/dev/null | cut -d= -f2)}"
    click="${(q)$(command -v tmux)}${WS_TMUX_SOCKET:+ -L ${(q)WS_TMUX_SOCKET}} select-window -t ${(q)win} \\; switch-client -t ${(q)sess}"
    { "$notifier" -title ws -message "$msg" -sound "$sound" -group "ws-$win" "${icon[@]}" \
        ${bundle:+-activate} ${bundle:+$bundle} -execute "$click" >/dev/null 2>&1 \
        || osascript_notify "$msg"; } &! # fails until macOS allows its notifications
  else
    osascript_notify "$msg" &!
  fi
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

osascript_notify() {
  osascript -e 'on run argv' -e 'display notification (item 1 of argv) with title "ws"' \
    -e 'end run' "$1" >/dev/null 2>&1
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

main "$@"
