#!/bin/zsh
set -u -o pipefail

# Loop Engineering — merge a lane branch into the integration branch, in the
# integration worktree (the only place merges and pushes happen).
#
# Usage:
#   loop-merge.sh --worktree <int-wt> --lane-branch <b> --verify-cmd <c> [--no-push]
#   loop-merge.sh --worktree <int-wt> --finish --verify-cmd <c> [--no-push]
#   loop-merge.sh --worktree <int-wt> --abort
#
# A push without a verify gate needs an explicit --no-verify — never silently.
#
# Exit codes:
#   0  merged (verified, pushed)   — also for an already-merged lane ({"already":true})
#   2  textual conflict — merge left IN PROGRESS, {"conflicts":[...]} on stdout
#   3  lane branch missing
#   4  merge committed but verify failed (semantic conflict) — caller decides revert
#   1  usage / unexpected git failure

WT="" LANE="" VERIFY="" MODE="merge" PUSH=1 NOVERIFY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
  --worktree) WT="$2"; shift 2 ;;
  --lane-branch) LANE="$2"; shift 2 ;;
  --verify-cmd) VERIFY="$2"; shift 2 ;;
  --finish) MODE="finish"; shift ;;
  --abort) MODE="abort"; shift ;;
  --no-push) PUSH=0; shift ;;
  --no-verify) NOVERIFY=1; shift ;;
  *) echo "loop-merge: unknown arg $1" >&2; exit 1 ;;
  esac
done

[[ -d "$WT" ]] || { echo "loop-merge: worktree '$WT' not found" >&2; exit 1; }
cd "$WT" || exit 1
if [[ "$MODE" != "abort" && -z "$VERIFY" && $NOVERIFY -eq 0 ]]; then
  echo "loop-merge: refusing to merge without --verify-cmd (pass --no-verify to override)" >&2
  exit 1
fi

verify_and_push() {
  if [[ -n "$VERIFY" ]]; then
    if ! eval "$VERIFY"; then
      return 4
    fi
  fi
  if [[ "$PUSH" -eq 1 ]]; then
    git push origin HEAD || return 1
  fi
  return 0
}

case "$MODE" in
abort)
  git merge --abort 2>/dev/null
  exit 0
  ;;
finish)
  [[ -f "$(git rev-parse --git-dir)/MERGE_HEAD" ]] || { echo "loop-merge: no merge in progress" >&2; exit 1; }
  if [[ -n "$(git diff --name-only --diff-filter=U)" ]]; then
    echo "loop-merge: unresolved conflicts remain" >&2
    git diff --name-only --diff-filter=U >&2
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
  git diff --name-only --diff-filter=U | jq -Rn '{"conflicts": [inputs]}'
  exit 2
  ;;
esac
