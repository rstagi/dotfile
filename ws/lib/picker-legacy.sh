#!/bin/zsh
# Legacy picker data. Sourced only by the zsh backend.

cmd_attach() {
  configure_bindings
  if [[ -z "$(cmd_list)" ]]; then
    cmd_new
  else
    cmd_pick
  fi
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

unpin() {
  [[ -f "$PINS" ]] || return 0
  grep -vxF -- "$1" "$PINS" > "$PINS.tmp"
  mv "$PINS.tmp" "$PINS"
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

unread_mark() { [[ "$1" == 1 ]] && print -n $'\e[1;36m•\e[0m' || print -n ' '; }

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
