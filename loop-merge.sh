#!/bin/zsh
set -u -o pipefail

# Loop Engineering — merge a lane branch into the integration branch, in the
# integration worktree (the only place merges and pushes happen).
#
# Usage:
#   loop-merge.sh --worktree <int-wt> --lane-branch <b> --verify-cmd <c> [--verified-record <f>] [--no-push]
#   loop-merge.sh --worktree <int-wt> --finish --verify-cmd <c> [--no-push]
#   loop-merge.sh --worktree <int-wt> --abort
#   loop-merge.sh --worktree <int-wt> --undo  (requires WS_LOOP_ID)
#
# A push without a verify gate needs an explicit --no-verify — never silently.
# --verified-record names the runner's verified.json ({tree, verify}). When the merged tree and
# the Verify command equal the record, the Verify already passed on this exact tree and does not
# run again. Any other record, or no record, runs the Verify.
# With WS_LOOP_ID set, an unpushed ancestor merge runs Verify and pushes on retry.
# --no-push records no successful merge event. --undo removes only a clean, unpushed
# merge whose subject is "merge: <lane>". It preserves the lane branch.
#
# Exit codes:
#   0  merged (verified, pushed)   — also for an already-merged lane ({"already":true})
#   2  textual conflict — merge left IN PROGRESS, {"conflicts":[...]} on stdout
#   3  lane branch missing
#   4  merge committed but verify failed (semantic conflict) — caller decides revert
#   1  usage / unexpected git failure

# Capture SCRIPT_DIR before any cd (loop-merge cd's into the worktree below).
# LOOP_EMIT_SH is a test seam like loop-orchestrator.sh.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
EMIT_SH="${LOOP_EMIT_SH:-$SCRIPT_DIR/loop-emit.sh}"
if [[ -r "$EMIT_SH" ]]; then
  source "$EMIT_SH"
else
  loop_emit() { :; }
fi

WT="" LANE="" VERIFY="" MODE="merge" PUSH=1 NOVERIFY=0 VERIFIED_RECORD=""
RUN_ID="" PHASE="" REPOSITORY=""
VERIFY_LOG=""   # a copy of the Verify output, outside the worktree; the EXIT trap removes it
VERIFY_DETAIL_BYTES=2048
VERIFY_DETAIL_LINES=20

while [[ $# -gt 0 ]]; do
  case "$1" in
  --worktree) WT="$2"; shift 2 ;;
  --lane-branch) LANE="$2"; shift 2 ;;
  --verify-cmd) VERIFY="$2"; shift 2 ;;
  --verified-record) VERIFIED_RECORD="$2"; shift 2 ;;
  --finish) MODE="finish"; shift ;;
  --abort) MODE="abort"; shift ;;
  --undo)
    [[ -n "${WS_LOOP_ID:-}" ]] || { echo "loop-merge: --undo requires WS_LOOP_ID" >&2; exit 1; }
    MODE="undo"; shift ;;
  --no-push) PUSH=0; shift ;;
  --no-verify) NOVERIFY=1; shift ;;
  --run-id) RUN_ID="$2"; shift 2 ;;
  --phase) PHASE="$2"; shift 2 ;;
  --repository) REPOSITORY="$2"; shift 2 ;;
  *) echo "loop-merge: unknown arg $1" >&2; exit 1 ;;
  esac
done

# A clean merge/finish (rc 0, not abort) is a phase.merged; conflicts emit merge.conflict;
# a failed Verify (exit 4) emits merge.verify-fail.
loop_merge_finish() {
  local rc="$1"
  [[ -n "$VERIFY_LOG" ]] && rm -f "$VERIFY_LOG"
  [[ -n "${RUN_ID:-}" && -n "${PHASE:-}" && "$MODE" != "abort" && "$MODE" != "undo" && "$rc" -eq 0 ]] || return 0
  [[ -z "${WS_LOOP_ID:-}" || "$PUSH" -eq 1 ]] || return 0
  jq -cn --arg event phase.merged --arg phase "$PHASE" --arg repository "$REPOSITORY" \
    --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{event:$event, phase:$phase, repository:(if ($repository|length)>0 then $repository else null end), ts:$ts}' | loop_emit "$RUN_ID" event
}
emit_conflict() { # $1 = conflicting files (newline-separated)
  [[ -n "${RUN_ID:-}" && -n "${PHASE:-}" ]] || return 0
  jq -cn --arg event merge.conflict --arg phase "$PHASE" --arg repository "$REPOSITORY" --arg detail "$1" \
    --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{event:$event, phase:$phase, repository:(if ($repository|length)>0 then $repository else null end),
      detail:$detail, ts:$ts}' | loop_emit "$RUN_ID" event
}
# $1 = Verify exit, $2 = file with the Verify output. The detail is "verify exit <n>" and the
# last output lines, VERIFY_DETAIL_BYTES at most; the cut keeps whole characters.
emit_verify_fail() {
  [[ -n "${RUN_ID:-}" && -n "${PHASE:-}" ]] || return 0
  local tail_text
  tail_text="$(tail -n "$VERIFY_DETAIL_LINES" "$2" 2>/dev/null | LC_ALL=C tail -c "$VERIFY_DETAIL_BYTES")"
  jq -cn --arg event merge.verify-fail --arg phase "$PHASE" --arg repository "$REPOSITORY" \
    --arg head "verify exit $1" --arg tail "$tail_text" --argjson max "$VERIFY_DETAIL_BYTES" \
    --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    'def fit($n): if utf8bytelength <= $n then . else .[1:] | fit($n) end;
     {event:$event, phase:$phase, repository:(if ($repository|length)>0 then $repository else null end),
      detail:(if $tail == "" then $head else $head + "\n" + ($tail | fit($max - ($head|utf8bytelength) - 1)) end),
      ts:$ts}' | loop_emit "$RUN_ID" event
}
# $1 = pass|fail|none. Appends loop.merge (keyed, so a retry adds nothing) to the loop session when WS_LOOP_ID is set. A failed
# append prints one warning and never changes the exit code. Verify none means --no-verify.
ws_loop_merge_event() {
  [[ -n "${WS_LOOP_ID:-}" ]] || return 0
  local ws_bin body head
  ws_bin="$(whence -p ws)"
  head="$(git rev-parse HEAD 2>/dev/null)"
  body="$(jq -cn --arg phase "$PHASE" --arg repo "$REPOSITORY" --arg head "$head" \
    --arg verify "$1" '{n:($phase|tonumber), repo:$repo, head:$head, verify:$verify}' 2>/dev/null)"
  # The key names the phase, the repository, the merged head and the Verify result. A retry of
  # the same merge has the same key. A failed Verify and a later passed Verify are two facts.
  if [[ -z "$ws_bin" || -z "$body" ]] ||
      ! "$ws_bin" event loop.merge --body "$body" --source tool --target "$WS_LOOP_ID" \
        --key "loop.merge:$PHASE:$REPOSITORY:$head:$1" > /dev/null 2>&1; then
    echo "loop-merge: warning: cannot append loop.merge to loop session $WS_LOOP_ID" >&2
  fi
  return 0
}
trap 'loop_merge_finish $?' EXIT

[[ -d "$WT" ]] || { echo "loop-merge: worktree '$WT' not found" >&2; exit 1; }
cd "$WT" || exit 1
if [[ "$MODE" != "abort" && "$MODE" != "undo" && -z "$VERIFY" && $NOVERIFY -eq 0 ]]; then
  echo "loop-merge: refusing to merge without --verify-cmd (pass --no-verify to override)" >&2
  exit 1
fi

# Verify output still reaches stdout and stderr; tee keeps a copy for the merge.verify-fail detail.
# The copy keeps the order inside each stream, not between stdout and stderr.
verify_and_push() {
  local rc
  if [[ -n "$VERIFY" ]] && tree_already_verified; then
    echo "loop-merge: tree $(git rev-parse 'HEAD^{tree}') passed this Verify in the runner; Verify skipped" >&2
  elif [[ -n "$VERIFY" ]]; then
    VERIFY_LOG="$(mktemp "${TMPDIR:-/tmp}/loop-merge-verify.XXXXXX")" || VERIFY_LOG=""
    # The braces make zsh wait for both tee processes before the next command.
    { eval "$VERIFY" } > >(tee -a "${VERIFY_LOG:-/dev/null}") 2> >(tee -a "${VERIFY_LOG:-/dev/null}" >&2)
    rc=$?
    if (( rc != 0 )); then
      emit_verify_fail "$rc" "${VERIFY_LOG:-/dev/null}"
      ws_loop_merge_event fail
      return 4
    fi
  fi
  if [[ "$PUSH" -eq 1 ]]; then
    git push origin HEAD || return 1
    # Record success only after the push. A local-only merge is not published.
    ws_loop_merge_event "$([[ -n "$VERIFY" ]] && print pass || print none)"
  fi
  return 0
}

# Return 0 when the remote branch contains HEAD, 1 when it does not, or 2 on a read error.
# Read the remote branch directly. A local tracking ref can be stale after a failed push.
remote_contains_head() {
  local branch remote_line remote_head rc
  branch="$(git symbolic-ref -q HEAD)" || return 2
  remote_line="$(git ls-remote --exit-code origin "$branch")"
  rc=$?
  (( rc == 2 )) && return 1
  (( rc == 0 )) || return 2
  remote_head="${remote_line%%$'\t'*}"
  [[ -n "$remote_head" ]] || return 2
  if ! git cat-file -e "$remote_head^{commit}" 2>/dev/null; then
    git fetch --no-tags --no-write-fetch-head origin "$remote_head" || return 2
  fi
  git merge-base --is-ancestor HEAD "$remote_head"
  rc=$?
  (( rc <= 1 )) && return "$rc"
  return 2
}

# Undo only a clean, unpublished merge made by this command. Preserve the lane ref.
undo_merge() {
  local dirty subject
  local -a parents
  dirty="$(git status --porcelain --untracked-files=all)" || return 1
  if [[ -n "$dirty" || -f "$(git rev-parse --git-path MERGE_HEAD)" ]]; then
    echo "loop-merge: undo requires a clean worktree with no merge in progress" >&2
    return 1
  fi
  subject="$(git show -s --format=%s HEAD)" || return 1
  parents=(${(s: :)$(git show -s --format=%P HEAD)})
  if [[ ${#parents} -ne 2 || "$subject" != 'merge: '?* ]]; then
    echo "loop-merge: HEAD is not a lane merge made by this command" >&2
    return 1
  fi
  remote_contains_head
  if [[ $? -ne 1 ]]; then
    echo "loop-merge: cannot undo a published merge or an unreadable remote branch" >&2
    return 1
  fi
  git reset --hard "$parents[1]" || return 1
}

# True when the runner record names the merged tree and this Verify command.
tree_already_verified() {
  [[ -n "$VERIFIED_RECORD" && -r "$VERIFIED_RECORD" ]] || return 1
  local tree
  tree="$(git rev-parse 'HEAD^{tree}')" || return 1
  jq -e --arg tree "$tree" --arg verify "$VERIFY" '.tree == $tree and .verify == $verify' \
    "$VERIFIED_RECORD" >/dev/null 2>&1
}

case "$MODE" in
undo)
  undo_merge; exit $?
  ;;
abort)
  git merge --abort 2>/dev/null
  exit 0
  ;;
finish)
  [[ -f "$(git rev-parse --git-dir)/MERGE_HEAD" ]] || { echo "loop-merge: no merge in progress" >&2; exit 1; }
  unresolved="$(git diff --name-only --diff-filter=U)"
  if [[ -n "$unresolved" ]]; then
    echo "loop-merge: unresolved conflicts remain" >&2
    printf '%s\n' "$unresolved" >&2
    emit_conflict "$unresolved"
    exit 2
  fi
  git add -u   # tracked files only — never sweep verify artifacts into the merge commit
  git -c rerere.enabled=true commit --no-edit || exit 1
  verify_and_push; exit $?
  ;;
merge)
  [[ -n "$LANE" ]] || { echo "loop-merge: --lane-branch required" >&2; exit 1; }
  git rev-parse --verify --quiet "$LANE" >/dev/null || { echo "loop-merge: branch '$LANE' not found" >&2; exit 3; }
  if git merge-base --is-ancestor "$LANE" HEAD; then
    if [[ -n "${WS_LOOP_ID:-}" ]]; then
      remote_contains_head
      remote_rc=$?
      case "$remote_rc" in
      0) ;;
      1) verify_and_push; exit $? ;;
      *) echo "loop-merge: cannot check the remote integration branch" >&2; exit 1 ;;
      esac
    fi
    echo '{"already":true}'
    exit 0
  fi
  if git -c rerere.enabled=true merge --no-ff -m "merge: $LANE" "$LANE"; then
    verify_and_push; exit $?
  fi
  # a merge that failed to START (dirty tree, untracked collision) is not a conflict
  if [[ ! -f "$(git rev-parse --git-dir)/MERGE_HEAD" ]]; then
    echo "loop-merge: merge failed to start (dirty worktree?)" >&2
    git status --short >&2
    exit 1
  fi
  # conflict: leave in progress, report the files
  conflicts="$(git diff --name-only --diff-filter=U)"
  printf '%s\n' "$conflicts" | jq -Rn '{"conflicts": [inputs]}'
  emit_conflict "$conflicts"
  exit 2
  ;;
esac
