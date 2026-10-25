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

echo "loop-merge: WS_LOOP_ID appends loop.merge with the merged head"
make_fake_ws "$TMP/fakews"
export FAKE_WS_LOG="$TMP/ws-pass.log"; : > "$FAKE_WS_LOG"
export EMIT_LOG="$TMP/emit-ws-pass.log"; : > "$EMIT_LOG"
WT="$(new_repo wspass)"
WS_LOOP_ID=LOOP1 PATH="$TMP/fakews:$PATH" merge "$WT" 'test "$(cat result.txt)" = lane'
assert_exit "$RC" "0" "a passed Verify exits 0 with WS_LOOP_ID"
assert_eq "$(wc -l < "$FAKE_WS_LOG" | tr -d ' ')" "1" "one ws call is made"
call="$(head -1 "$FAKE_WS_LOG")"
assert_eq "$(print -r -- "$call" | jq -c '[.[0], .[1], .[2], .[4], .[5], .[6], .[7]]')" '["event","loop.merge","--body","--source","tool","--target","LOOP1"]' "the call appends loop.merge with source tool and the loop target"
body="$(print -r -- "$call" | jq -r '.[3]')"
assert_eq "$(print -r -- "$body" | jq -c '{n,repo,verify}')" '{"n":7,"repo":"acme/api","verify":"pass"}' "the body names phase, repository and verify pass"
assert_eq "$(print -r -- "$body" | jq -r .head)" "$(git -C "$WT" rev-parse HEAD)" "the body head is the merged tree commit"
assert_eq "$(emitted phase.merged | jq -r .phase)" "7" "the old-engine event stays"

echo "loop-merge: a failed merged-tree Verify appends loop.merge with verify fail"
export FAKE_WS_LOG="$TMP/ws-fail.log"; : > "$FAKE_WS_LOG"
WT="$(new_repo wsfail)"
WS_LOOP_ID=LOOP1 PATH="$TMP/fakews:$PATH" merge "$WT" false
assert_exit "$RC" "4" "a failed Verify keeps exit 4"
body="$(jq -r '.[3]' "$FAKE_WS_LOG")"
assert_eq "$(print -r -- "$body" | jq -c '{n,repo,verify}')" '{"n":7,"repo":"acme/api","verify":"fail"}' "the body has verify fail"

echo "loop-merge: loop.merge waits for the push"
export FAKE_WS_LOG="$TMP/ws-push.log"; : > "$FAKE_WS_LOG"
WT="$(new_repo wspush)"
git -C "$WT" remote add origin "$TMP/missing-remote"
WS_LOOP_ID=LOOP1 PATH="$TMP/fakews:$PATH" zsh "$ROOT/loop-merge.sh" --worktree "$WT" --lane-branch feat/lane \
  --verify-cmd true --run-id run-1 --phase 7 --repository acme/api > "$TMP/out" 2> "$TMP/err"
assert_exit "$?" "1" "a failed push exits 1"
assert_eq "$(cat "$FAKE_WS_LOG")" "" "a failed push appends no loop.merge"

echo "loop-merge: a failed append warns once and keeps the exit code"
for case in pass fail; do
  export FAKE_WS_LOG="$TMP/ws-warn-$case.log"; : > "$FAKE_WS_LOG"
  WT="$(new_repo warn$case)"
  verify=true expected=0
  [[ "$case" == fail ]] && verify=false expected=4
  WS_LOOP_ID=LOOP1 FAKE_WS_EXIT=1 PATH="$TMP/fakews:$PATH" merge "$WT" "$verify"
  assert_exit "$RC" "$expected" "$case keeps its exit code when the append fails"
  assert_eq "$(print -r -- "$ERR" | grep -c 'warning: cannot append loop.merge')" "1" "$case prints one warning"
done

echo "loop-merge: without WS_LOOP_ID no ws call is made"
export FAKE_WS_LOG="$TMP/ws-unset.log"; : > "$FAKE_WS_LOG"
WT="$(new_repo unset)"
env -u WS_LOOP_ID PATH="$TMP/fakews:$PATH" FAKE_WS_LOG="$FAKE_WS_LOG" zsh "$ROOT/loop-merge.sh" --worktree "$WT" \
  --lane-branch feat/lane --verify-cmd true --no-push --run-id run-1 --phase 7 --repository acme/api > "$TMP/out" 2> "$TMP/err"
assert_exit "$?" "0" "the merge succeeds"
assert_eq "$(cat "$FAKE_WS_LOG")" "" "no ws call is made"
assert_eq "$(cat "$TMP/err")" "" "no warning is printed"

test_summary
