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
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
export HOME="$TMP/home" WS_HOME="$TMP/ws" WS_STATE="$TMP/state" WS_SOCK="$TMP/ws.sock"
mkdir -p "$HOME"
unset WS_LOOP_ID WS_SESSION_ID LOOP_DAEMON_URL
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
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

# new_origin <name> creates a private origin and publishes the integration base.
new_origin() {
  WT="$(new_repo "$1")"
  ORIGIN="$TMP/$1-origin.git"
  git init -q --bare "$ORIGIN"
  git -C "$WT" remote add origin "$ORIGIN"
  git -C "$WT" push -q origin HEAD
  export EMIT_LOG="$TMP/$1-emit.log" FAKE_WS_LOG="$TMP/$1-ws.log"
  : > "$EMIT_LOG"
  : > "$FAKE_WS_LOG"
}

# skills_merge <args...> runs the public CLI with private event sinks.
skills_merge() {
  WS_LOOP_ID=LOOP1 PATH="$TMP/fakews:$PATH" zsh "$ROOT/loop-merge.sh" --worktree "$WT" \
    --run-id run-1 --phase 7 --repository acme/api "$@" > "$TMP/out" 2> "$TMP/err"
  RC=$?
  OUT="$(cat "$TMP/out")"
  ERR="$(cat "$TMP/err")"
}

if [[ "${1:-all}" == all ]]; then
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
new_origin wspass
skills_merge --lane-branch feat/lane --verify-cmd 'test "$(cat result.txt)" = lane'
assert_exit "$RC" "0" "a passed Verify exits 0 with WS_LOOP_ID"
assert_eq "$(wc -l < "$FAKE_WS_LOG" | tr -d ' ')" "1" "one ws call is made"
call="$(head -1 "$FAKE_WS_LOG")"
assert_eq "$(print -r -- "$call" | jq -c '[.[0], .[1], .[2], .[4], .[5], .[6], .[7]]')" '["event","loop.merge","--body","--source","tool","--target","LOOP1"]' "the call appends loop.merge with source tool and the loop target"
assert_eq "$(print -r -- "$call" | jq -c '.[8:]')" "[\"--key\",\"loop.merge:7:acme/api:$(git -C "$WT" rev-parse HEAD):pass\"]" "the key names the phase, repository, head and Verify result"
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
assert_eq "$(jq -c '.[8:]' "$FAKE_WS_LOG")" "[\"--key\",\"loop.merge:7:acme/api:$(git -C "$WT" rev-parse HEAD):fail\"]" "a failed Verify has its own key"

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
  new_origin "warn$case"
  verify=true expected=0
  [[ "$case" == fail ]] && verify=false expected=4
  FAKE_WS_EXIT=1 skills_merge --lane-branch feat/lane --verify-cmd "$verify"
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

echo "loop-merge: a runner-verified tree skips the merged-tree Verify"
# record <worktree> <verify-cmd> <file> — a runner Verify record for the lane tree.
record() {
  jq -cn --arg tree "$(git -C "$1" rev-parse feat/lane^{tree})" --arg verify "$2" \
    '{tree:$tree, verify:$verify}' > "$3"
}
# merge_recorded <worktree> <verify-cmd> <record> — merge with a runner Verify record.
merge_recorded() {
  zsh "$ROOT/loop-merge.sh" --worktree "$1" --lane-branch feat/lane --verify-cmd "$2" --no-push \
    --verified-record "$3" --run-id run-1 --phase 7 --repository acme/api > "$TMP/out" 2> "$TMP/err"
  RC=$?
  ERR="$(cat "$TMP/err")"
}
VERIFY_CMD="touch $TMP/verify-ran"
export EMIT_LOG="$TMP/emit-skip.log"; : > "$EMIT_LOG"
WT="$(new_repo skip)"
record "$WT" "$VERIFY_CMD" "$TMP/skip.json"
rm -f "$TMP/verify-ran"
merge_recorded "$WT" "$VERIFY_CMD" "$TMP/skip.json"
assert_exit "$RC" "0" "a merge of a verified tree exits 0"
assert_eq "$([[ -e "$TMP/verify-ran" ]] && print ran)" "" "the merged-tree Verify does not run again"
assert_contains "$ERR" "Verify skipped" "stderr names the skip"
assert_eq "$(emitted phase.merged | jq -r .phase)" "7" "a skipped Verify still emits phase.merged"

echo "loop-merge: a record of another tree or command does not skip the Verify"
WT="$(new_repo moved)"
record "$WT" "$VERIFY_CMD" "$TMP/moved.json"
print other > "$WT/other.txt"
git -C "$WT" add other.txt
git -C "$WT" commit -qm "integration moved"
rm -f "$TMP/verify-ran"
merge_recorded "$WT" "$VERIFY_CMD" "$TMP/moved.json"
assert_exit "$RC" "0" "the merge of a moved integration branch exits 0"
assert_eq "$([[ -e "$TMP/verify-ran" ]] && print ran)" "ran" "a merged tree that differs runs the Verify"
WT="$(new_repo command)"
record "$WT" "true" "$TMP/command.json"
rm -f "$TMP/verify-ran"
merge_recorded "$WT" "$VERIFY_CMD" "$TMP/command.json"
assert_eq "$([[ -e "$TMP/verify-ran" ]] && print ran)" "ran" "a record of another Verify command runs the Verify"
WT="$(new_repo missing)"
rm -f "$TMP/verify-ran"
merge_recorded "$WT" "$VERIFY_CMD" "$TMP/no-such-record.json"
assert_exit "$RC" "0" "a missing record is not an error"
assert_eq "$([[ -e "$TMP/verify-ran" ]] && print ran)" "ran" "a missing record runs the Verify"

fi

make_fake_ws "$TMP/fakews"
if [[ "${1:-all}" == (all|retry) ]]; then
  echo "loop-merge: a failed push retries Verify and publishes the merge once"
  new_origin retry
  cat > "$ORIGIN/hooks/pre-receive" <<'HOOK'
#!/bin/sh
exit 1
HOOK
  chmod +x "$ORIGIN/hooks/pre-receive"
  verify="print verified >> '$TMP/retry-verify.log'"
  skills_merge --lane-branch feat/lane --verify-cmd "$verify"
  assert_exit "$RC" 1 "the rejected push exits 1"
  merged="$(git -C "$WT" rev-parse HEAD)"
  assert_eq "$(git -C "$WT" rev-list --count HEAD^2..HEAD)" 1 "the local merge commit remains"
  assert_eq "$(cat "$FAKE_WS_LOG")" "" "the failed push appends no loop.merge"
  assert_eq "$(emitted phase.merged)" "" "the failed push emits no phase.merged"
  rm "$ORIGIN/hooks/pre-receive"
  skills_merge --lane-branch feat/lane --verify-cmd "$verify"
  assert_exit "$RC" 0 "the retry succeeds"
  assert_eq "$(wc -l < "$TMP/retry-verify.log" | tr -d ' ')" 2 "the retry runs Verify"
  assert_eq "$(git --git-dir="$ORIGIN" rev-parse refs/heads/feat/int)" "$merged" "the retry publishes the existing merge"
  assert_eq "$(git --git-dir="$ORIGIN" show feat/int:result.txt)" lane "the remote tree contains the lane"
  assert_eq "$(wc -l < "$FAKE_WS_LOG" | tr -d ' ')" 1 "the retry appends one loop.merge"
  body="$(jq -r '.[3]' "$FAKE_WS_LOG")"
  assert_eq "$(print -r -- "$body" | jq -c '{n,repo,verify}')" '{"n":7,"repo":"acme/api","verify":"pass"}' "the result records Verify pass"
  assert_eq "$(jq -c '.[8:]' "$FAKE_WS_LOG")" "[\"--key\",\"loop.merge:7:acme/api:$merged:pass\"]" "the retry uses the Phase 134 key"
  skills_merge --lane-branch feat/lane --verify-cmd "$verify"
  assert_exit "$RC" 0 "a published merge stays complete"
  assert_eq "$(wc -l < "$TMP/retry-verify.log" | tr -d ' ')" 2 "a published merge needs no Verify"
  assert_eq "$(wc -l < "$FAKE_WS_LOG" | tr -d ' ')" 1 "a published merge adds no second result"
fi

if [[ "${1:-all}" == (all|undo) ]]; then
  echo "loop-merge: undo removes a failed merge before the fixed lane merges"
  new_origin undo
  base="$(git -C "$WT" rev-parse HEAD)"
  lane="$(git -C "$WT" rev-parse feat/lane)"
  skills_merge --lane-branch feat/lane --verify-cmd false
  assert_exit "$RC" 4 "a merged-tree Verify failure exits 4"
  assert_eq "$(jq -r '.[3] | fromjson | .verify' "$FAKE_WS_LOG")" fail "Verify failure records no pass"
  skills_merge --undo
  assert_exit "$RC" 0 "undo needs no Verify command"
  assert_eq "$(git -C "$WT" rev-parse HEAD)" "$base" "undo restores the first parent"
  assert_eq "$(git -C "$WT" rev-parse feat/lane)" "$lane" "undo preserves the lane branch"
  assert_eq "$(git -C "$WT" status --porcelain)" "" "undo leaves the integration tree clean"
  assert_eq "$(emitted phase.merged)" "" "undo emits no phase.merged"
  assert_eq "$(wc -l < "$FAKE_WS_LOG" | tr -d ' ')" 1 "undo appends no loop.merge"
  git -C "$TMP/undo" switch -q feat/lane
  print fixed > "$TMP/undo/fixed.txt"
  git -C "$TMP/undo" add fixed.txt
  git -C "$TMP/undo" commit -qm fix
  skills_merge --lane-branch feat/lane --verify-cmd 'test "$(cat result.txt)" = lane && test "$(cat fixed.txt)" = fixed'
  assert_exit "$RC" 0 "the fixed lane merges and passes Verify"
  assert_eq "$(git --git-dir="$ORIGIN" show feat/int:result.txt)" lane "the remote keeps the first lane change"
  assert_eq "$(git --git-dir="$ORIGIN" show feat/int:fixed.txt)" fixed "the remote includes the fix"
  assert_eq "$(jq -r '.[3] | fromjson | .verify' "$FAKE_WS_LOG" | tr '\n' ' ')" 'fail pass ' "only the published fixed merge records pass"
fi

if [[ "${1:-all}" == (all|no-push) ]]; then
  echo "loop-merge: a local-only skills merge records no published result"
  new_origin nopush
  base="$(git -C "$WT" rev-parse HEAD)"
  skills_merge --lane-branch feat/lane --verify-cmd true --no-push
  assert_exit "$RC" 0 "an explicit local-only merge succeeds"
  assert_eq "$(cat "$FAKE_WS_LOG")" "" "a local-only merge appends no loop.merge"
  assert_eq "$(emitted phase.merged)" "" "a local-only skills merge emits no phase.merged"
  assert_eq "$(git --git-dir="$ORIGIN" rev-parse refs/heads/feat/int)" "$base" "the origin stays at the base"
fi

if [[ "${1:-all}" == (all|safety) ]]; then
  echo "loop-merge: undo refuses an ordinary commit and a foreign merge"
  for kind in ordinary foreign; do
    new_origin "$kind"
    if [[ "$kind" == foreign ]]; then
      git -C "$WT" merge -q --no-ff -m 'manual merge' feat/lane
    fi
    before="$(git -C "$WT" rev-parse HEAD)"
    skills_merge --undo
    assert_exit "$RC" 1 "undo refuses the $kind commit"
    assert_eq "$(git -C "$WT" rev-parse HEAD)" "$before" "the $kind HEAD stays unchanged"
    assert_eq "$(git -C "$WT" status --porcelain)" "" "the $kind tree stays clean"
    assert_eq "$(cat "$FAKE_WS_LOG")" "" "the refused $kind undo records no merge"
  done

  echo "loop-merge: undo preserves tracked, staged and untracked changes"
  for kind in tracked staged untracked; do
    new_origin "dirty-$kind"
    skills_merge --lane-branch feat/lane --verify-cmd false
    before="$(git -C "$WT" rev-parse HEAD)"
    if [[ "$kind" == untracked ]]; then
      print keep > "$WT/keep.txt"
    else
      print keep > "$WT/result.txt"
      [[ "$kind" == staged ]] && git -C "$WT" add result.txt
    fi
    dirty="$(git -C "$WT" status --porcelain)"
    skills_merge --undo
    assert_exit "$RC" 1 "undo refuses $kind changes"
    assert_eq "$(git -C "$WT" rev-parse HEAD)" "$before" "undo keeps HEAD with $kind changes"
    assert_eq "$(git -C "$WT" status --porcelain)" "$dirty" "undo keeps the $kind status"
    if [[ "$kind" == untracked ]]; then
      assert_eq "$(cat "$WT/keep.txt")" keep "undo preserves the untracked content"
    else
      assert_eq "$(cat "$WT/result.txt")" keep "undo preserves the $kind content"
    fi
    assert_eq "$(git -C "$WT" rev-parse feat/lane)" "$(git -C "$WT" rev-parse HEAD^2)" "undo preserves the lane ref"
  done

  echo "loop-merge: undo refuses a published merge"
  new_origin published
  skills_merge --lane-branch feat/lane --verify-cmd true
  assert_exit "$RC" 0 "the merge publishes successfully"
  before="$(git -C "$WT" rev-parse HEAD)"
  skills_merge --undo
  assert_exit "$RC" 1 "undo refuses the published merge"
  assert_eq "$(git -C "$WT" rev-parse HEAD)" "$before" "the published HEAD stays unchanged"
  assert_eq "$(git --git-dir="$ORIGIN" rev-parse refs/heads/feat/int)" "$before" "the remote stays unchanged"
  assert_eq "$(wc -l < "$FAKE_WS_LOG" | tr -d ' ')" 1 "undo adds no merge result"

  echo "loop-merge: undo reads the remote when its tracking ref is stale"
  new_origin stale
  base="$(git -C "$WT" rev-parse HEAD)"
  skills_merge --lane-branch feat/lane --verify-cmd false
  before="$(git -C "$WT" rev-parse HEAD)"
  git clone -q --branch feat/int "$ORIGIN" "$TMP/publisher"
  git -C "$TMP/publisher" config user.email test@example.com
  git -C "$TMP/publisher" config user.name Test
  git -C "$TMP/publisher" fetch -q "$WT" feat/int
  git -C "$TMP/publisher" merge -q --ff-only FETCH_HEAD
  print remote > "$TMP/publisher/remote.txt"
  git -C "$TMP/publisher" add remote.txt
  git -C "$TMP/publisher" commit -qm 'remote change'
  git -C "$TMP/publisher" push -q origin HEAD
  assert_eq "$(git -C "$WT" rev-parse origin/feat/int)" "$base" "the local tracking ref still names the base"
  skills_merge --undo
  assert_exit "$RC" 1 "undo refuses a merge contained in the newer remote tip"
  assert_eq "$(git -C "$WT" rev-parse HEAD)" "$before" "the stale local HEAD stays unchanged"
  skills_merge --lane-branch feat/lane --verify-cmd false
  assert_exit "$RC" 0 "a remote descendant proves the lane is already published"
  assert_eq "$(wc -l < "$FAKE_WS_LOG" | tr -d ' ')" 1 "the ancestor check adds no merge result"

  echo "loop-merge: undo refuses an unreadable remote"
  new_origin unreadable
  skills_merge --lane-branch feat/lane --verify-cmd false
  before="$(git -C "$WT" rev-parse HEAD)"
  git -C "$WT" remote set-url origin "$TMP/no-origin"
  skills_merge --undo
  assert_exit "$RC" 1 "undo refuses when it cannot read the remote"
  assert_eq "$(git -C "$WT" rev-parse HEAD)" "$before" "the unreadable remote causes no reset"
  skills_merge --lane-branch feat/lane --verify-cmd true
  assert_exit "$RC" 1 "a retry also refuses an unreadable remote"
  assert_eq "$(wc -l < "$FAKE_WS_LOG" | tr -d ' ')" 1 "the retry appends no success result"

  echo "loop-merge: undo permits a merge whose remote branch does not exist"
  new_origin absent
  base="$(git -C "$WT" rev-parse HEAD)"
  skills_merge --lane-branch feat/lane --verify-cmd false
  git --git-dir="$ORIGIN" update-ref -d refs/heads/feat/int
  skills_merge --undo
  assert_exit "$RC" 0 "undo permits the unpublished branch"
  assert_eq "$(git -C "$WT" rev-parse HEAD)" "$base" "the unpublished branch returns to its base"
fi

if [[ "${1:-all}" == (all|conflict) ]]; then
  echo "loop-merge: a conflict finishes with one published merge"
  new_origin conflict
  print integration > "$WT/result.txt"
  git -C "$WT" commit -qam integration
  git -C "$WT" push -q origin HEAD
  base="$(git -C "$WT" rev-parse HEAD)"
  skills_merge --lane-branch feat/lane --verify-cmd true
  assert_exit "$RC" 2 "a conflicting merge exits 2"
  assert_eq "$(cat "$FAKE_WS_LOG")" "" "the conflict appends no loop.merge"
  assert_eq "$(emitted phase.merged)" "" "the conflict emits no phase.merged"
  skills_merge --finish --verify-cmd true
  assert_exit "$RC" 2 "finish refuses unresolved conflicts"
  skills_merge --undo
  assert_exit "$RC" 1 "undo refuses a merge in progress"
  print -l integration lane > "$WT/result.txt"
  git -C "$WT" add result.txt
  skills_merge --finish --verify-cmd 'test "$(head -1 result.txt)" = integration && test "$(tail -1 result.txt)" = lane'
  assert_exit "$RC" 0 "finish commits, verifies and pushes the resolution"
  assert_eq "$(git -C "$WT" rev-parse HEAD^1)" "$base" "the merge keeps the integration parent"
  assert_eq "$(git -C "$WT" rev-parse HEAD^2)" "$(git -C "$WT" rev-parse feat/lane)" "the merge keeps the lane parent"
  assert_eq "$(git --git-dir="$ORIGIN" rev-parse refs/heads/feat/int)" "$(git -C "$WT" rev-parse HEAD)" "finish publishes the merge"
  assert_eq "$(git --git-dir="$ORIGIN" show feat/int:result.txt)" $'integration\nlane' "the remote tree keeps both changes"
  assert_eq "$(git --git-dir="$ORIGIN" rev-list --count --min-parents=2 feat/int)" 1 "the remote has one merge commit"
  assert_eq "$(jq -r '.[3] | fromjson | .verify' "$FAKE_WS_LOG")" pass "finish appends one passed result"
fi

if [[ "${1:-all}" == (all|legacy) ]]; then
  echo "loop-merge: the old engine keeps its ancestor fast path"
  new_origin legacy
  base="$(git -C "$WT" rev-parse HEAD)"
  merge "$WT" true
  assert_exit "$RC" 0 "the old engine still permits a local merge"
  before="$(git -C "$WT" rev-parse HEAD)"
  merge "$WT" false
  assert_exit "$RC" 0 "the old engine skips Verify on its ancestor path"
  assert_eq "$OUT" '{"already":true}' "the ancestor output stays unchanged"
  assert_eq "$(git --git-dir="$ORIGIN" rev-parse refs/heads/feat/int)" "$base" "the old engine does not push the ancestor"
  env -u WS_LOOP_ID zsh "$ROOT/loop-merge.sh" --worktree "$WT" --undo > "$TMP/out" 2> "$TMP/err"
  assert_exit "$?" 1 "undo requires the skills engine"
  assert_eq "$(git -C "$WT" rev-parse HEAD)" "$before" "the old engine undo changes nothing"
fi

test_summary
