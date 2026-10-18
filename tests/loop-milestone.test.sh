#!/bin/zsh
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
trap 'rm -rf "$TMP"' EXIT
export LOOP_REPO_REGISTRY="$TMP/home/repos.json"
export LOOP_WORKTREE_ROOT="$TMP/worktrees"
mkdir -p "$TMP/bin" "$TMP/home"

# Fake gh: PR state from FAKE_PR_STATES ("<url>=<STATE>" lines), repo default branch main.
cat > "$TMP/bin/gh" <<'GH'
#!/bin/sh
if [ "$1" = pr ] && [ "$2" = view ]; then
  [ "${FAKE_GH_FAIL:-0}" = 0 ] || exit 1
  state="$(printf '%s\n' "$FAKE_PR_STATES" | sed -n "s|^$3=||p")"
  [ -n "$state" ] || exit 1
  printf '{"state":"%s"}\n' "$state"
  exit 0
fi
printf 'main\n'
GH
chmod +x "$TMP/bin/gh"
export PATH="$TMP/bin:$PATH"

# make_repo <slug>: bare remote + checkout whose origin fetches from the bare remote.
make_repo() {
  local slug="$1" remote="$TMP/${1:t}.git" checkout="$TMP/${1:t}"
  git init --bare -q "$remote"
  git init -q -b main "$checkout"
  git -C "$checkout" config user.email test@example.com
  git -C "$checkout" config user.name Test
  print one > "$checkout/file.txt"
  git -C "$checkout" add file.txt
  git -C "$checkout" commit -qm initial
  git -C "$checkout" remote add origin "git@github.com:$slug.git"
  git -C "$checkout" config url."$remote".insteadOf "git@github.com:$slug.git"
  git -C "$checkout" push -q origin main
  "$ROOT/loop-repo.sh" map "$slug" "$checkout" >/dev/null
}
make_repo acme/api
branch_hash() { printf '%s' "$1" | shasum | cut -c1-8; }
make_repo acme/web

API_PR=https://github.com/acme/api/pull/1
WEB_PR=https://github.com/acme/web/pull/2
RUN=run-m
STATE="$TMP/state.json"
write_state() {
  jq -n --arg run "$RUN" --arg api "$API_PR" --arg web "$WEB_PR" \
    --arg apiwt "$LOOP_WORKTREE_ROOT/$RUN/acme--api/integration" \
    --arg webwt "$LOOP_WORKTREE_ROOT/$RUN/acme--web/integration" \
    '{runId:$run, milestone:1, runtimeRoot:$apiwt, integrationBranch:"feat/m1",
      repositories:{
        "acme/api":{integrationBranch:"feat/m1-api", integrationWorktree:$apiwt, adopted:false, prUrl:$api},
        "acme/web":{integrationBranch:"feat/m1-web", integrationWorktree:$webwt, adopted:false, prUrl:$web}},
      awaitingMerge:{milestone:1, prs:{"acme/api":$api, "acme/web":$web}}}' > "$STATE"
}
write_state

echo "loop-milestone: check"
export FAKE_PR_STATES="$API_PR=MERGED
$WEB_PR=MERGED"
out="$($ROOT/loop-milestone.sh check --state "$STATE")"; rc=$?
assert_exit "$rc" "0" "all merged"
assert_eq "$out" "acme/api	$API_PR	merged
acme/web	$WEB_PR	merged" "one row per gate PR"

export FAKE_PR_STATES="$API_PR=MERGED
$WEB_PR=OPEN"
out="$($ROOT/loop-milestone.sh check --state "$STATE")"; rc=$?
assert_exit "$rc" "3" "any open"
assert_contains "$out" "acme/web	$WEB_PR	open" "open row"

export FAKE_PR_STATES="$API_PR=CLOSED
$WEB_PR=OPEN"
out="$($ROOT/loop-milestone.sh check --state "$STATE")"; rc=$?
assert_exit "$rc" "4" "closed beats open"
assert_contains "$out" "acme/api	$API_PR	closed" "closed row"

FAKE_GH_FAIL=1 $ROOT/loop-milestone.sh check --state "$STATE" >/dev/null 2>&1
assert_exit "$?" "1" "gh failure"

jq 'del(.awaitingMerge)' "$STATE" > "$TMP/s2.json"
$ROOT/loop-milestone.sh check --state "$TMP/s2.json" >/dev/null 2>&1
assert_exit "$?" "1" "check without awaitingMerge refused"

echo "loop-milestone: advance"
# Old milestone worktrees exist (created by prepare), like a real parked loop.
for pair in "acme/api feat/m1-api" "acme/web feat/m1-web"; do
  $ROOT/loop-repo.sh prepare --repo ${pair%% *} --run-id $RUN --integration-branch ${pair#* } >/dev/null
done
OLD_API="$LOOP_WORKTREE_ROOT/$RUN/acme--api/integration"
OLD_WEB="$LOOP_WORKTREE_ROOT/$RUN/acme--web/integration"
BRANCHES='{"acme/api":"feat/m2-api","acme/web":"feat/m2-web"}'
NEW_API="$LOOP_WORKTREE_ROOT/$RUN/acme--api/integration-feat-m2-api-$(branch_hash feat/m2-api)"
NEW_WEB="$LOOP_WORKTREE_ROOT/$RUN/acme--web/integration-feat-m2-web-$(branch_hash feat/m2-web)"
before="$(cat "$STATE")"

# Partial failure: web's next branch is checked out elsewhere -> prepare refuses.
git -C "$TMP/web" worktree add -q -b feat/m2-web "$TMP/stolen" main
out="$($ROOT/loop-milestone.sh advance --state "$STATE" --next 2 --branches "$BRANCHES" 2>/dev/null)"; rc=$?
assert_exit "$rc" "4" "partial prepare failure surfaces prepare exit"
assert_eq "$out" "" "no patch on failure"
[[ -d "$OLD_API" && -d "$OLD_WEB" ]]
assert_exit "$?" "0" "old worktrees kept when any prepare fails"
[[ -d "$NEW_API" ]]
assert_exit "$?" "0" "first participant prepared before the failure"
git -C "$TMP/web" worktree remove --force "$TMP/stolen"
git -C "$TMP/web" branch -D feat/m2-web >/dev/null

out="$($ROOT/loop-milestone.sh advance --state "$STATE" --next 2 --branches "$BRANCHES")"; rc=$?
assert_exit "$rc" "0" "retry succeeds"
assert_eq "$(print -r -- "$out" | jq -r .milestone)" "2" "patch milestone"
assert_eq "$(print -r -- "$out" | jq -r '.repositories["acme/api"].integrationWorktree')" "$NEW_API" "api worktree"
assert_eq "$(print -r -- "$out" | jq -r '.repositories["acme/web"].integrationWorktree')" "$NEW_WEB" "web worktree"
assert_eq "$(print -r -- "$out" | jq -r '.repositories | keys | length')" "2" "repositories replaced with participants"
assert_eq "$(print -r -- "$out" | jq -r '.awaitingMerge')" "null" "gate cleared"
assert_eq "$(print -r -- "$out" | jq -r '.resumePending')" "2" "patch marks the restart pending"
assert_eq "$(print -r -- "$out" | jq -r --arg u "$API_PR" '.milestones["1"].prs["acme/api"] == $u')" "true" "history records old PRs"
assert_eq "$(print -r -- "$out" | jq -r .runtimeRoot)" "$NEW_API" "runtime root follows the old runtime repo"
assert_eq "$(print -r -- "$out" | jq -r .integrationBranch)" "feat/m2-api" "top-level integration branch"
[[ ! -e "$OLD_API" && ! -e "$OLD_WEB" ]]
assert_exit "$?" "0" "old worktrees retired after all prepares"
assert_eq "$(cat "$STATE")" "$before" "state not written"
jq -n '{prUrl:"https://x/pr/1", review:{a:1}, reviewPipeline:{b:2}, integrationWorktree:"/old", repositories:{"acme/old":{}}}' > "$TMP/parked.json"
applied="$(jq --argjson patch "$out" '. + $patch' "$TMP/parked.json")"
assert_eq "$(print -r -- "$applied" | jq -c '[.prUrl, .review, .reviewPipeline]')" "[null,null,null]" "patch clears loop-level review scalars"
assert_eq "$(print -r -- "$applied" | jq -r .integrationWorktree)" "$NEW_API" "patch moves integrationWorktree"
assert_eq "$(print -r -- "$applied" | jq -r '.repositories | keys | join(",")')" "acme/api,acme/web" "patch replaces repositories wholesale"

echo "loop-milestone: advance is idempotent"
out2="$($ROOT/loop-milestone.sh advance --state "$STATE" --next 2 --branches "$BRANCHES")"; rc=$?
assert_exit "$rc" "0" "repeat advance"
assert_eq "$out2" "$out" "same patch on repeat"

echo "loop-milestone: check and advance on a stubbed plan (milestone 1 archived)"
# Coordinator .loop: the 3-milestone fixture plan (acme/api#1 + acme/web#2 gate milestone 1) beside the parked state.
COORD="$TMP/coord/.loop"; mkdir -p "$COORD"
cp "$HERE/fixtures/milestones/plan.md" "$COORD/plan.md"
cp "$STATE" "$COORD/state.json"
export FAKE_PR_STATES="$API_PR=MERGED
$WEB_PR=MERGED"
LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" zsh "$ROOT/loop-plan.sh" archive --milestone 1 --dir "$COORD" >/dev/null
assert_exit "$?" "0" "milestone 1 archived beside the parked state"
assert_contains "$(cat "$COORD/plan.md")" '- **Phases (all done):** 1, 2, 9, 3' "plan.md now carries the milestone 1 stub"
assert_eq "$(cat "$COORD/state.json")" "$(cat "$STATE")" "archive leaves state.json and its phase entries unchanged"
out3="$($ROOT/loop-milestone.sh check --state "$COORD/state.json")"; rc=$?
assert_exit "$rc" "0" "check: all gate PRs merged"
assert_eq "$out3" "acme/api	$API_PR	merged
acme/web	$WEB_PR	merged" "check rows unchanged by the archive"
next_web="$(node --input-type=module -e '
  import fs from "node:fs";
  const { parsePlan } = await import(process.argv[1]);
  process.stdout.write(parsePlan(fs.readFileSync(process.argv[2], "utf8")).milestones[1].integrationBranches["acme/web"]);
' "$ROOT/loop-web/src/model/parse-plan.ts" "$COORD/plan.md")"
assert_eq "$next_web" "feat/m2-web" "the next milestone's branch still resolves from the stubbed plan"
out3="$($ROOT/loop-milestone.sh advance --state "$COORD/state.json" --next 2 --branches "$BRANCHES")"; rc=$?
assert_exit "$rc" "0" "advance succeeds"
assert_eq "$out3" "$out" "advance prints the same patch"

echo "loop-milestone: advance validation"
$ROOT/loop-milestone.sh advance --state "$STATE" --next 1 --branches "$BRANCHES" >/dev/null 2>&1
assert_exit "$?" "1" "next must follow the parked milestone"
$ROOT/loop-milestone.sh advance --state "$STATE" --next 2 --branches '{}' >/dev/null 2>&1
assert_exit "$?" "1" "empty branches refused"
$ROOT/loop-milestone.sh advance --state "$STATE" --next 2 --branches '{"acme/other":"feat/x"}' >/dev/null 2>&1
assert_exit "$?" "2" "unmapped repository surfaces prepare exit"
jq 'del(.awaitingMerge.prs)' "$STATE" > "$TMP/s3.json"
$ROOT/loop-milestone.sh advance --state "$TMP/s3.json" --next 2 --branches "$BRANCHES" >/dev/null 2>&1
assert_exit "$?" "1" "advance without awaitingMerge PRs refused"
jq 'del(.awaitingMerge)' "$STATE" > "$TMP/s4.json"
$ROOT/loop-milestone.sh advance --state "$TMP/s4.json" --next 2 --branches "$BRANCHES" >/dev/null 2>&1
assert_exit "$?" "1" "advance without awaitingMerge refused"
$ROOT/loop-milestone.sh advance --state "$STATE" --next 3 --branches "$BRANCHES" >/dev/null 2>&1
assert_exit "$?" "1" "next must be parked milestone + 1"
# bounded <cmd...>: kill after 5s (a hung parser would otherwise spin forever).
bounded() {
  setopt local_options no_monitor no_notify
  "$@" & local pid=$!
  ( sleep 5; kill -9 $pid 2>/dev/null ) &
  local killer=$! rc
  wait $pid; rc=$?
  kill $killer 2>/dev/null
  return $rc
}
for opt in --state --next --branches; do
  bounded $ROOT/loop-milestone.sh advance $opt >/dev/null 2>&1
  assert_exit "$?" "1" "$opt without a value refused"
done

echo "loop-milestone: self-hosted runtime"
# acme/zdot hosts the Loop scripts (committed) and sorts after acme/web; state has no runtimeRoot.
make_repo acme/zdot
cp "$ROOT/loop-milestone.sh" "$ROOT/loop-repo.sh" "$TMP/zdot/"
git -C "$TMP/zdot" add loop-milestone.sh loop-repo.sh
git -C "$TMP/zdot" commit -qm scripts
git -C "$TMP/zdot" push -q origin main
self_state() { # <run> <state-file>
  local run="$1"
  for pair in "acme/zdot feat/m1-zdot-$run" "acme/web feat/m1-web-$run"; do
    $ROOT/loop-repo.sh prepare --repo ${pair%% *} --run-id $run --integration-branch ${pair#* } >/dev/null
  done
  jq -n --arg run "$run" --arg api "$API_PR" --arg web "$WEB_PR" \
    --arg zwt "$LOOP_WORKTREE_ROOT/$run/acme--zdot/integration" \
    --arg webwt "$LOOP_WORKTREE_ROOT/$run/acme--web/integration" \
    '{runId:$run, milestone:1, integrationBranch:"feat/m1-zdot", integrationWorktree:$zwt, prUrl:$api,
      repositories:{
        "acme/zdot":{integrationBranch:"feat/m1-zdot", integrationWorktree:$zwt, adopted:false, prUrl:$api},
        "acme/web":{integrationBranch:"feat/m1-web", integrationWorktree:$webwt, adopted:false, prUrl:$web}},
      awaitingMerge:{milestone:1, prs:{"acme/zdot":$api, "acme/web":$web}}}' > "$2"
}
RUN=run-self
self_state $RUN "$TMP/self.json"
OLD_Z="$LOOP_WORKTREE_ROOT/$RUN/acme--zdot/integration"
NEW_Z="$LOOP_WORKTREE_ROOT/$RUN/acme--zdot/integration-feat-m2-zdot-$(branch_hash feat/m2-zdot)"
SB='{"acme/zdot":"feat/m2-zdot","acme/web":"feat/m2-web-self"}'
out="$("$OLD_Z/loop-milestone.sh" advance --state "$TMP/self.json" --next 2 --branches "$SB")"; rc=$?
assert_exit "$rc" "0" "advance from the old host worktree succeeds"
assert_eq "$(print -r -- "$out" | jq -r .runtimeRoot)" "$NEW_Z" "runtime root follows host, not first participant"
assert_eq "$(print -r -- "$out" | jq -r .integrationWorktree)" "$NEW_Z" "integrationWorktree follows host"
[[ ! -e "$OLD_Z" && ! -e "$LOOP_WORKTREE_ROOT/$RUN/acme--web/integration" ]]
assert_exit "$?" "0" "old host + web worktrees retired"
out2="$("$NEW_Z/loop-milestone.sh" advance --state "$TMP/self.json" --next 2 --branches "$SB")"; rc=$?
assert_exit "$rc" "0" "retry from new host worktree"
assert_eq "$out2" "$out" "retry yields same patch"

echo "loop-milestone: host not participating keeps its runtime"
RUN=run-self-2
self_state $RUN "$TMP/self2.json"
OLD_Z="$LOOP_WORKTREE_ROOT/$RUN/acme--zdot/integration"
NEW_W="$LOOP_WORKTREE_ROOT/$RUN/acme--web/integration-feat-m2-web-self2-$(branch_hash feat/m2-web-self2)"
out="$("$OLD_Z/loop-milestone.sh" advance --state "$TMP/self2.json" --next 2 --branches '{"acme/web":"feat/m2-web-self2"}')"; rc=$?
assert_exit "$rc" "0" "advance without host participant"
assert_eq "$(print -r -- "$out" | jq -r .runtimeRoot)" "$OLD_Z" "runtime root stays in old host worktree"
assert_eq "$(print -r -- "$out" | jq -r .integrationWorktree)" "$NEW_W" "integrationWorktree = first participant"
assert_eq "$(print -r -- "$out" | jq -r .integrationBranch)" "feat/m2-web-self2" "integrationBranch = first participant"
[[ -d "$OLD_Z" && ! -e "$LOOP_WORKTREE_ROOT/$RUN/acme--web/integration" ]]
assert_exit "$?" "0" "host worktree kept, web retired"
jq --argjson patch "$out" --arg web "$WEB_PR" '. + $patch | .awaitingMerge = {milestone:2, prs:{"acme/web":$web}}' \
  "$TMP/self2.json" > "$TMP/self3.json"
out="$("$OLD_Z/loop-milestone.sh" advance --state "$TMP/self3.json" --next 3 --branches '{"acme/web":"feat/m3-web-self2"}')"; rc=$?
assert_exit "$rc" "0" "later advance with retained host"
assert_eq "$(print -r -- "$out" | jq -r .runtimeRoot)" "$OLD_Z" "retained host runtime root survives later milestones"
[[ -d "$OLD_Z" ]]
assert_exit "$?" "0" "retained host worktree still present"

echo "loop-milestone: installed runtime never guesses a host"
# Scripts run from the installed checkout ($ROOT); no runtimeRoot/runtimeRepo in state.
RUN=run-inst
for pair in "acme/api feat/m1-api-inst" "acme/web feat/m1-web-inst"; do
  $ROOT/loop-repo.sh prepare --repo ${pair%% *} --run-id $RUN --integration-branch ${pair#* } >/dev/null
done
I1_API="$LOOP_WORKTREE_ROOT/$RUN/acme--api/integration"
I1_WEB="$LOOP_WORKTREE_ROOT/$RUN/acme--web/integration"
jq -n --arg run "$RUN" --arg api "$API_PR" --arg web "$WEB_PR" --arg apiwt "$I1_API" --arg webwt "$I1_WEB" \
  '{runId:$run, milestone:1, integrationBranch:"feat/m1-api-inst", integrationWorktree:$apiwt,
    repositories:{
      "acme/api":{integrationBranch:"feat/m1-api-inst", integrationWorktree:$apiwt, adopted:false},
      "acme/web":{integrationBranch:"feat/m1-web-inst", integrationWorktree:$webwt, adopted:false}},
    awaitingMerge:{milestone:1, prs:{"acme/api":$api, "acme/web":$web}}}' > "$TMP/inst1.json"
out="$($ROOT/loop-milestone.sh advance --state "$TMP/inst1.json" --next 2 \
  --branches '{"acme/api":"feat/m2-api-inst","acme/web":"feat/m2-web-inst"}')"; rc=$?
assert_exit "$rc" "0" "installed advance to M2"
assert_eq "$(print -r -- "$out" | jq -c '[.runtimeRoot, .runtimeRepo]')" "[null,null]" "no host: runtimeRoot/runtimeRepo null"
I2_API="$(print -r -- "$out" | jq -r '.repositories["acme/api"].integrationWorktree')"
I2_WEB="$(print -r -- "$out" | jq -r '.repositories["acme/web"].integrationWorktree')"
assert_eq "$(print -r -- "$out" | jq -r .integrationWorktree)" "$I2_API" "integrationWorktree still first participant"
[[ ! -e "$I1_API" && ! -e "$I1_WEB" ]]
assert_exit "$?" "0" "every M1 worktree retired"
jq --argjson patch "$out" --arg api "$API_PR" '. + $patch | .awaitingMerge = {milestone:2, prs:{"acme/api":$api}}' \
  "$TMP/inst1.json" > "$TMP/inst2.json"
out="$($ROOT/loop-milestone.sh advance --state "$TMP/inst2.json" --next 3 \
  --branches '{"acme/api":"feat/m3-api-inst","acme/web":"feat/m3-web-inst"}')"; rc=$?
assert_exit "$rc" "0" "installed advance to M3"
assert_eq "$(print -r -- "$out" | jq -c '[.runtimeRoot, .runtimeRepo]')" "[null,null]" "M3 runtimeRoot still null"
[[ -d "$I2_API" || -d "$I2_WEB" ]]
assert_exit "$?" "1" "every M2 worktree retired"

echo "loop-milestone: runtimeRepo host sits out M2, rejoins M3"
RUN=run-rejoin
for pair in "acme/zdot feat/m1-zdot-rejoin" "acme/web feat/m1-web-rejoin"; do
  $ROOT/loop-repo.sh prepare --repo ${pair%% *} --run-id $RUN --integration-branch ${pair#* } >/dev/null
done
R1_Z="$LOOP_WORKTREE_ROOT/$RUN/acme--zdot/integration"
R1_WEB="$LOOP_WORKTREE_ROOT/$RUN/acme--web/integration"
jq -n --arg run "$RUN" --arg api "$API_PR" --arg web "$WEB_PR" --arg zwt "$R1_Z" --arg webwt "$R1_WEB" \
  '{runId:$run, milestone:1, runtimeRepo:"acme/zdot", runtimeRoot:$zwt, integrationWorktree:$zwt,
    repositories:{
      "acme/zdot":{integrationBranch:"feat/m1-zdot-rejoin", integrationWorktree:$zwt, adopted:false},
      "acme/web":{integrationBranch:"feat/m1-web-rejoin", integrationWorktree:$webwt, adopted:false}},
    awaitingMerge:{milestone:1, prs:{"acme/zdot":$api, "acme/web":$web}}}' > "$TMP/rejoin1.json"
out="$($ROOT/loop-milestone.sh advance --state "$TMP/rejoin1.json" --next 2 --branches '{"acme/web":"feat/m2-web-rejoin"}')"; rc=$?
assert_exit "$rc" "0" "host sits out M2"
assert_eq "$(print -r -- "$out" | jq -c '[.runtimeRoot, .runtimeRepo]')" "[\"$R1_Z\",\"acme/zdot\"]" "runtime stays in retained host worktree"
[[ -d "$R1_Z" && ! -e "$R1_WEB" ]]
assert_exit "$?" "0" "host retained, web retired"
jq --argjson patch "$out" --arg web "$WEB_PR" '. + $patch | .awaitingMerge = {milestone:2, prs:{"acme/web":$web}}' \
  "$TMP/rejoin1.json" > "$TMP/rejoin2.json"
R3_Z="$LOOP_WORKTREE_ROOT/$RUN/acme--zdot/integration-feat-m3-zdot-rejoin-$(branch_hash feat/m3-zdot-rejoin)"
RB='{"acme/zdot":"feat/m3-zdot-rejoin","acme/web":"feat/m3-web-rejoin"}'
out="$($ROOT/loop-milestone.sh advance --state "$TMP/rejoin2.json" --next 3 --branches "$RB")"; rc=$?
assert_exit "$rc" "0" "host rejoins M3"
assert_eq "$(print -r -- "$out" | jq -c '[.runtimeRoot, .runtimeRepo]')" "[\"$R3_Z\",\"acme/zdot\"]" "runtime moves to host's new M3 worktree"
[[ ! -e "$R1_Z" ]]
assert_exit "$?" "0" "stale retained host worktree retired"
out2="$($ROOT/loop-milestone.sh advance --state "$TMP/rejoin2.json" --next 3 --branches "$RB")"; rc=$?
assert_exit "$rc" "0" "rejoin retry"
assert_eq "$out2" "$out" "rejoin retry yields same patch"

echo "loop-milestone: adopted host sits out M2, rejoins M3"
# M1 adopted the host's existing worktree (outside the run root): it is the user's, never retired.
RUN=run-adopt
A1_Z="$TMP/zdot-adopted"
git -C "$TMP/zdot" worktree add -q -b feat/m1-zdot-adopt "$A1_Z" main
$ROOT/loop-repo.sh prepare --repo acme/web --run-id $RUN --integration-branch feat/m1-web-adopt >/dev/null
A1_WEB="$LOOP_WORKTREE_ROOT/$RUN/acme--web/integration"
jq -n --arg run "$RUN" --arg api "$API_PR" --arg web "$WEB_PR" --arg zwt "$A1_Z" --arg webwt "$A1_WEB" \
  '{runId:$run, milestone:1, runtimeRepo:"acme/zdot", runtimeRoot:$zwt, integrationWorktree:$zwt,
    repositories:{
      "acme/zdot":{integrationBranch:"feat/m1-zdot-adopt", integrationWorktree:$zwt, adopted:true},
      "acme/web":{integrationBranch:"feat/m1-web-adopt", integrationWorktree:$webwt, adopted:false}},
    awaitingMerge:{milestone:1, prs:{"acme/zdot":$api, "acme/web":$web}}}' > "$TMP/adopt1.json"
out="$($ROOT/loop-milestone.sh advance --state "$TMP/adopt1.json" --next 2 --branches '{"acme/web":"feat/m2-web-adopt"}')"; rc=$?
assert_exit "$rc" "0" "adopted host sits out M2"
assert_eq "$(print -r -- "$out" | jq -c '[.runtimeRoot, .runtimeAdopted]')" "[\"$A1_Z\",true]" "retained host keeps its adopted flag"
jq --argjson patch "$out" --arg web "$WEB_PR" '. + $patch | .awaitingMerge = {milestone:2, prs:{"acme/web":$web}}' \
  "$TMP/adopt1.json" > "$TMP/adopt2.json"
A3_Z="$LOOP_WORKTREE_ROOT/$RUN/acme--zdot/integration-feat-m3-zdot-adopt-$(branch_hash feat/m3-zdot-adopt)"
AB='{"acme/zdot":"feat/m3-zdot-adopt","acme/web":"feat/m3-web-adopt"}'
out="$($ROOT/loop-milestone.sh advance --state "$TMP/adopt2.json" --next 3 --branches "$AB" 2>&1)"; rc=$?
assert_exit "$rc" "0" "adopted host rejoins M3"
assert_eq "$(print -r -- "$out" | jq -c '[.runtimeRoot, .runtimeAdopted]')" "[\"$A3_Z\",false]" "runtime moves to host's new M3 worktree"
[[ -d "$A1_Z" ]]
assert_exit "$?" "0" "adopted retained host worktree never retired"

test_summary
