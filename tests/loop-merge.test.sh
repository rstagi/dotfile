#!/bin/zsh
# loop-merge.sh — the daemon events of a merge. The fake-emit seam (LOOP_EMIT_SH) records
# each loop_emit call in $EMIT_LOG; no daemon is touched.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
trap 'rm -rf "$TMP"' EXIT
export LOOP_EMIT_SH="$HERE/fake/loop-emit.sh"

# new_repo <name> — an integration worktree on feat/int plus a lane branch feat/lane that
# writes result.txt. Prints the worktree path.
new_repo() {
  local repo="$TMP/$1"
  git init -q -b main "$repo"
  git -C "$repo" config user.email test@example.com
  git -C "$repo" config user.name Test
  print base > "$repo/result.txt"
  git -C "$repo" add result.txt
  git -C "$repo" commit -qm base
  git -C "$repo" branch feat/int
  git -C "$repo" switch -q -c feat/lane
  print lane > "$repo/result.txt"
  git -C "$repo" commit -qam lane
  git -C "$repo" switch -q main
  git -C "$repo" worktree add -q "$TMP/$1-int" feat/int
  print -r -- "$TMP/$1-int"
}

# merge <worktree> <verify-cmd> — run loop-merge with a run id, a phase and a repository.
# Sets RC, OUT and ERR.
merge() {
  zsh "$ROOT/loop-merge.sh" --worktree "$1" --lane-branch feat/lane --verify-cmd "$2" --no-push \
    --run-id run-1 --phase 7 --repository acme/api > "$TMP/out" 2> "$TMP/err"
  RC=$?
  OUT="$(cat "$TMP/out")"
  ERR="$(cat "$TMP/err")"
}

# emitted <event> — the JSON body of each emitted event with that name.
emitted() {
  sed -n 's/^run-1 event //p' "$EMIT_LOG" | jq -c --arg e "$1" 'select(.event == $e)'
}

echo "loop-merge: a merged-tree Verify failure emits merge.verify-fail"
export EMIT_LOG="$TMP/emit-fail.log"; : > "$EMIT_LOG"
WT="$(new_repo fail)"
merge "$WT" "sh -c 'echo first-out; echo second-err >&2; echo last-out; exit 3'"
assert_exit "$RC" "4" "a failed Verify exits 4"
assert_contains "$OUT" "last-out" "Verify stdout still reaches stdout"
assert_contains "$ERR" "second-err" "Verify stderr still reaches stderr"
ev="$(emitted merge.verify-fail)"
assert_eq "$(print -r -- "$ev" | jq -r '[.phase, .repository] | join(" ")')" "7 acme/api" \
  "the event names the phase and the repository"
detail="$(print -r -- "$ev" | jq -r .detail)"
assert_eq "${detail%%$'\n'*}" "verify exit 3" "the detail starts with the Verify exit"
assert_contains "$detail" "first-out" "the detail has the Verify stdout lines"
assert_contains "$detail" "second-err" "the detail has the Verify stderr lines"
[[ "$detail" == *first-out*last-out* ]] && in_order=yes || in_order=no
assert_eq "$in_order" "yes" "the detail keeps the order of one stream"
assert_eq "$(emitted phase.merged)" "" "a failed Verify emits no phase.merged"
assert_eq "$(git -C "$WT" status --porcelain)" "" "the captured output stays out of the worktree"

echo "loop-merge: the merge.verify-fail detail keeps the last lines, 2 KB at most"
export EMIT_LOG="$TMP/emit-long.log"; : > "$EMIT_LOG"
WT="$(new_repo long)"
merge "$WT" "pad=''; for i in {1..200}; do print -r -- \"line-\$i \${(l:90::é:)pad}\"; done; false"
assert_exit "$RC" "4" "a failed Verify exits 4"
detail="$(emitted merge.verify-fail | jq -r .detail)"
bytes="$(print -rn -- "$detail" | LC_ALL=C wc -c | tr -d ' ')"
assert_eq "$(( bytes <= 2048 ))" "1" "the detail is 2048 bytes or less (got $bytes)"
assert_eq "${detail%%$'\n'*}" "verify exit 1" "the long detail starts with the Verify exit"
assert_contains "${detail##*$'\n'}" "line-200 " "the long detail ends with the last Verify line"
assert_eq "${detail//[^$'\uFFFD']/}" "" "the cut keeps whole characters (no U+FFFD)"

echo "loop-merge: a verified merge emits only phase.merged"
export EMIT_LOG="$TMP/emit-pass.log"; : > "$EMIT_LOG"
WT="$(new_repo pass)"
merge "$WT" 'test "$(cat result.txt)" = lane'
assert_exit "$RC" "0" "a passed Verify exits 0"
assert_eq "$(emitted phase.merged | jq -r .phase)" "7" "phase.merged names the phase"
assert_eq "$(emitted merge.verify-fail)" "" "a passed Verify emits no merge.verify-fail"

test_summary
