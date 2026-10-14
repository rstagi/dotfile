#!/bin/zsh
# Workspaces: git worktree + tmux session create/rm/list/merge, repo setup and name resolution. Sourced by ws.sh.

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
