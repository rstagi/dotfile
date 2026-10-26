#!/bin/zsh
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
source "$HERE/home-guard.sh"

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
WS_TEST_DAEMON_PID=""
cleanup() {
  if [[ -n "$WS_TEST_DAEMON_PID" ]]; then
    kill "$WS_TEST_DAEMON_PID" 2>/dev/null
    wait "$WS_TEST_DAEMON_PID" 2>/dev/null
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
home_guard_sandbox "$TMP/home"
unset WS_LOOP_ID WS_SESSION_ID WS_SESSION LOOP_DAEMON_URL
export LOOP_DAEMON_URL="http://127.0.0.1:9"
export WS_HOME="$TMP/ws" WS_STATE="$TMP/ws-state" WS_TMUX_SOCKET="$TMP/tmux.sock"
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
for blocked in curl launchctl; do
  printf '#!/bin/sh\nexit 99\n' > "$TMP/bin/$blocked"
  chmod +x "$TMP/bin/$blocked"
done
export PATH="$TMP/bin:$PATH"
export WS_SOCK="$TMP/wsd.sock" FAKE_WS_SOCKET="$TMP/wsd.sock" FAKE_WS_LOG="$TMP/ws.log"
unset WS_BIN
cat > "$TMP/bin/ws" <<'WS'
#!/bin/sh
[ "$WS_SOCK" = "$FAKE_WS_SOCKET" ] || exit 99
printf '%s\n' "$@" > "$FAKE_WS_LOG"
printf '%s' "${FAKE_WS_OUTPUT:-}"
exit "${FAKE_WS_EXIT:-0}"
WS
chmod +x "$TMP/bin/ws"

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

# Resume tests use the command boundary and private repositories.
resume_fixture() {
  local name="$1"
  RESUME="$TMP/$name/.loop"
  mkdir -p "$RESUME"
  sed "s|feat/m2-web|feat/$name-m2-web|g" "$HERE/fixtures/milestones/plan.md" > "$RESUME/plan.md"
  jq -n --arg run "$name" --arg apiwt "$TMP/api" --arg webwt "$TMP/web" '
    {runId:$run,milestone:1,custom:{keep:true},
      phases:{"1":{status:"done"},"2":{status:"merged"},"9":{status:"done"},"3":{status:"done"},
        "4":{status:"todo"},"10":{status:"blocked"}},
      repositories:{"acme/api":{integrationWorktree:$apiwt,adopted:true},
        "acme/web":{integrationWorktree:$webwt,adopted:true}},
      awaitingMerge:{milestone:1,prs:{"acme/api":"https://github.com/acme/api/pull/1",
        "acme/web":"https://github.com/acme/web/pull/2"}}}' > "$RESUME/state.json"
}

echo "loop-milestone: resume refuses unmerged gate PRs"
resume_fixture resume-refused
for pr_state expected in OPEN 3 CLOSED 4 ERROR 1; do
  export FAKE_PR_STATES="https://github.com/acme/api/pull/1=MERGED
https://github.com/acme/web/pull/2=$pr_state"
  cp "$RESUME/state.json" "$TMP/resume-state-before.json"
  cp "$RESUME/plan.md" "$TMP/resume-plan-before.md"
  "$ROOT/loop-milestone.sh" resume --dir "$RESUME" > "$TMP/resume-$pr_state.out" 2>&1
  assert_exit "$?" "$expected" "resume stops for $pr_state"
  assert_eq "$(cat "$RESUME/state.json")" "$(cat "$TMP/resume-state-before.json")" "resume preserves state for $pr_state"
  assert_eq "$(cat "$RESUME/plan.md")" "$(cat "$TMP/resume-plan-before.md")" "resume preserves the plan for $pr_state"
  assert_eq "$([[ -e "$RESUME/milestones" ]] && print yes || print no)" no "resume creates no archive for $pr_state"
done
echo "loop-milestone: resume advances after the gate merge"
mkdir -p "$TMP/resume-bin"
cat > "$TMP/resume-bin/ws" <<'WS'
#!/bin/sh
[ "$WS_SOCK" = "$FAKE_WS_SOCKET" ] || exit 99
if [ "$1" = usage ]; then
  printf 'prepare\n' >> "$FAKE_RESUME_STEPS"
  exit 0
fi
[ "$1" = event ] || exit 99
jq -cn '$ARGS.positional' --args -- "$@" >> "$FAKE_RESUME_REQUESTS"
[ "${FAKE_RESUME_FAIL_TYPE:-}" != "$2" ] || exit 1
key="${10}"
if ! jq -es --arg key "$key" 'any(.[]; .[9]==$key)' "$FAKE_WS_LOG" >/dev/null; then
  jq -cn '$ARGS.positional' --args -- "$@" >> "$FAKE_WS_LOG"
fi
WS
chmod +x "$TMP/resume-bin/ws"
cat > "$TMP/bin/curl" <<'CURL'
#!/bin/sh
case "$*" in
  */state)
    if [ -n "${FAKE_RESUME_STATE_LOG:-}" ]; then
      jq -c . >> "$FAKE_RESUME_STATE_LOG"
      printf 'apply\n' >> "$FAKE_RESUME_STEPS"
      exit 0
    fi ;;
esac
exit 99
CURL
export FAKE_PR_STATES="https://github.com/acme/api/pull/1=MERGED
https://github.com/acme/web/pull/2=MERGED"
resume_fixture resume-merged
export FAKE_WS_LOG="$TMP/resume-events.jsonl" FAKE_RESUME_REQUESTS="$TMP/resume-requests.jsonl"
export FAKE_RESUME_STATE_LOG="$TMP/resume-states.jsonl" FAKE_RESUME_STEPS="$TMP/resume-steps.log"
export EMIT_LOG="$TMP/resume-emit.log"
: > "$FAKE_WS_LOG"; : > "$FAKE_RESUME_REQUESTS"; : > "$FAKE_RESUME_STATE_LOG"; : > "$FAKE_RESUME_STEPS"; : > "$EMIT_LOG"
WS_LOOP_ID=RESUME1 WS_BIN="$TMP/resume-bin/ws" LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" \
  "$ROOT/loop-milestone.sh" resume --dir "$RESUME" > "$TMP/resume-merged.out" 2> "$TMP/resume-merged.err"
assert_exit "$?" 0 "resume succeeds after the merge"
assert_eq "$(jq -c '[.milestone,.resumePending,.awaitingMerge,.custom]' "$RESUME/state.json")" '[2,null,null,{"keep":true}]' "resume advances and preserves unrelated state"
assert_eq "$(jq -c '.repositories|keys' "$RESUME/state.json")" '["acme/web"]' "resume replaces repositories with the next participants"
assert_eq "$(jq -r '.repositories["acme/web"].integrationBranch' "$RESUME/state.json")" feat/resume-merged-m2-web "resume reads the next integration branch from the plan"
assert_eq "$([[ -f "$RESUME/milestones/m1.md" ]] && print yes || print no)" yes "resume archives the shipped milestone"
assert_eq "$(node "$ROOT/loop-plan-parse.mjs" --plan "$RESUME/plan.md" | jq -r .status)" 'in progress' "resume restores plan Status"
assert_eq "$(jq -sc '[.[]|select(.resumePending==.milestone)]|length' "$FAKE_RESUME_STATE_LOG")" 1 "resume applies the advance patch once"
assert_eq "$(jq -sc '[.[]|.resumePending]' "$FAKE_RESUME_STATE_LOG")" '[2,null]' "resume clears the pending marker last"
assert_eq "$(jq -sc '[.[]|.[1]]' "$FAKE_WS_LOG")" '["loop.gate","loop.milestone","loop.phases"]' "resume appends tool events in order"
assert_eq "$(jq -c 'select(.[1]=="loop.gate")|.[3]|fromjson' "$FAKE_WS_LOG")" '{"kind":"merge","state":"merged","milestone":1,"prs":{"acme/api":"https://github.com/acme/api/pull/1","acme/web":"https://github.com/acme/web/pull/2"}}' "the merged gate records the parked PRs"
assert_eq "$(jq -c 'select(.[1]=="loop.milestone")|.[3]|fromjson' "$FAKE_WS_LOG")" '{"milestone":2}' "resume records the new milestone"
assert_eq "$(jq -c 'select(.[1]=="loop.phases")|.[3]|fromjson|[.phases[]|select(.n==4 or .n==10)|.status]' "$FAKE_WS_LOG")" '["todo","todo"]' "resume uses the open phase status mapping"
assert_eq "$(jq -s 'all(.[]; .[4:9]==["--source","tool","--target","RESUME1","--key"] and (.[9]|length)>0)' "$FAKE_WS_LOG")" true "each resume event has tool provenance and a key"
assert_eq "$(grep -c ' register ' "$EMIT_LOG")" 2 "resume pushes the archive and the active plan"
echo "loop-milestone: resume recovers after the apply"
for failed_type in loop.gate loop.milestone loop.phases; do
  resume_fixture "resume-crash-${failed_type#loop.}"
  : > "$FAKE_WS_LOG"; : > "$FAKE_RESUME_REQUESTS"; : > "$FAKE_RESUME_STATE_LOG"; : > "$FAKE_RESUME_STEPS"; : > "$EMIT_LOG"
  FAKE_RESUME_FAIL_TYPE="$failed_type" WS_LOOP_ID=RESUME1 WS_BIN="$TMP/resume-bin/ws" \
    LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" "$ROOT/loop-milestone.sh" resume --dir "$RESUME" \
    > "$TMP/resume-crash.out" 2> "$TMP/resume-crash.err"
  assert_exit "$?" 1 "a failed $failed_type stops resume after the apply"
  assert_eq "$(jq -c '[.milestone,.resumePending,.awaitingMerge]' "$RESUME/state.json")" '[2,2,null]' "a failed $failed_type keeps the pending marker"
  assert_eq "$(node "$ROOT/loop-plan-parse.mjs" --plan "$RESUME/plan.md" | jq -r .status)" 'awaiting merge' "a failed $failed_type does not activate the plan"
  archive_identity="$(stat -f '%i:%m' "$RESUME/milestones/m1.md")"
  FAKE_GH_FAIL=1 WS_LOOP_ID=RESUME1 WS_BIN="$TMP/resume-bin/ws" LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" \
    "$ROOT/loop-milestone.sh" resume --dir "$RESUME" > "$TMP/resume-retry.out" 2> "$TMP/resume-retry.err"
  assert_exit "$?" 0 "a retry after $failed_type skips the gate check"
  assert_eq "$(jq -c '[.milestone,.resumePending]' "$RESUME/state.json")" '[2,null]' "a retry after $failed_type completes resume"
  assert_eq "$(stat -f '%i:%m' "$RESUME/milestones/m1.md")" "$archive_identity" "a retry after $failed_type preserves the archive"
  assert_eq "$(jq -sc '[.[]|.resumePending]' "$FAKE_RESUME_STATE_LOG")" '[2,null]' "a retry after $failed_type does not apply the patch again"
  assert_eq "$(cat "$FAKE_RESUME_STEPS")" $'prepare\napply\napply' "a retry after $failed_type does not prepare repositories again"
  assert_eq "$(jq -sc '[.[]|.[1]]' "$FAKE_WS_LOG")" '["loop.gate","loop.milestone","loop.phases"]' "a retry after $failed_type records each event once"
  assert_eq "$(jq -s '[.[]|select(.[1]=="loop.gate")|.[9]]|unique|length' "$FAKE_RESUME_REQUESTS")" 1 "a retry after $failed_type reuses the gate key"
  assert_eq "$(jq -s '[.[]|select(.[1]=="loop.milestone")|.[9]]|unique|length' "$FAKE_RESUME_REQUESTS")" 1 "a retry after $failed_type reuses the milestone key"
  assert_eq "$(jq -s '[.[]|select(.[1]=="loop.phases")|.[9]]|unique|length' "$FAKE_RESUME_REQUESTS")" 1 "a retry after $failed_type reuses the phase key"
done

echo "loop-milestone: resume without a loop identity"
resume_fixture resume-manual
: > "$FAKE_WS_LOG"; : > "$FAKE_RESUME_REQUESTS"; : > "$FAKE_RESUME_STATE_LOG"; : > "$FAKE_RESUME_STEPS"; : > "$EMIT_LOG"
WS_BIN="$TMP/resume-bin/ws" LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" \
  "$ROOT/loop-milestone.sh" resume --dir "$RESUME" > "$TMP/resume-manual.out" 2> "$TMP/resume-manual.err"
assert_exit "$?" 0 "resume works without WS_LOOP_ID"
assert_eq "$(jq -c '[.milestone,.resumePending,.awaitingMerge]' "$RESUME/state.json")" '[2,null,null]' "manual resume advances the state"
assert_eq "$(node "$ROOT/loop-plan-parse.mjs" --plan "$RESUME/plan.md" | jq -r .status)" 'in progress' "manual resume activates the plan"
assert_eq "$(jq -s length "$FAKE_WS_LOG")" 0 "manual resume appends no tool events"
assert_eq "$(grep -c ' register ' "$EMIT_LOG")" 2 "manual resume pushes the archive and the active plan"
cp "$RESUME/state.json" "$TMP/resume-complete-state.json"
cp "$RESUME/plan.md" "$TMP/resume-complete-plan.md"
WS_BIN="$TMP/resume-bin/ws" LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" \
  "$ROOT/loop-milestone.sh" resume --dir "$RESUME" > "$TMP/resume-complete.out" 2>&1
assert_exit "$?" 1 "a completed resume cannot advance an unparked milestone"
assert_eq "$(cat "$RESUME/state.json")" "$(cat "$TMP/resume-complete-state.json")" "a completed resume preserves the state on repeat"
assert_eq "$(cat "$RESUME/plan.md")" "$(cat "$TMP/resume-complete-plan.md")" "a completed resume preserves the plan on repeat"

echo "loop-milestone: an archive refusal stops resume"
resume_fixture resume-archive-refused
sed 's/Add the core API `\[lane: A\]` `\[status: done\]`/Add the core API `[lane: A]` `[status: todo]`/' \
  "$RESUME/plan.md" > "$TMP/resume-not-done.md"
cp "$TMP/resume-not-done.md" "$RESUME/plan.md"
cp "$RESUME/state.json" "$TMP/resume-archive-state.json"
: > "$FAKE_WS_LOG"; : > "$FAKE_RESUME_STATE_LOG"; : > "$FAKE_RESUME_STEPS"
WS_LOOP_ID=RESUME1 WS_BIN="$TMP/resume-bin/ws" LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" \
  "$ROOT/loop-milestone.sh" resume --dir "$RESUME" > "$TMP/resume-archive.out" 2> "$TMP/resume-archive.err"
assert_exit "$?" 3 "resume returns the archive refusal code"
assert_eq "$(cat "$RESUME/state.json")" "$(cat "$TMP/resume-archive-state.json")" "an archive refusal preserves the state"
assert_eq "$(cat "$FAKE_RESUME_STEPS")" '' "an archive refusal does not prepare or apply"
assert_eq "$(jq -s length "$FAKE_WS_LOG")" 0 "an archive refusal appends no event"
echo "loop-milestone: resume archives every shipped milestone"
resume_fixture resume-multi
sed -e 's/feat\/m3-api/feat\/resume-multi-m3-api/' \
  -e 's|\*\*PR (acme/web):\*\* _none yet_|**PR (acme/web):** https://github.com/acme/web/pull/3|' \
  -e 's/status: in-progress/status: done/g' \
  -e '/^### Phase 10 /s/status: todo/status: done/' \
  -e '/^### Phase 5 /s/status: todo/status: done/; /^### Phase 5 /s/kind: pr-review/kind: pr-review]` `[gate: merge/' \
  "$RESUME/plan.md" > "$TMP/resume-multi-plan.md"
cp "$TMP/resume-multi-plan.md" "$RESUME/plan.md"
jq '.milestone=2 | .awaitingMerge={milestone:2,prs:{"acme/web":"https://github.com/acme/web/pull/3"}} |
  .phases["4"].status="done" | .phases["10"].status="done" | .phases["5"]={status:"done"} |
  .milestones={"1":{prs:{"acme/api":"https://github.com/acme/api/pull/1","acme/web":"https://github.com/acme/web/pull/2"}}}' \
  "$RESUME/state.json" > "$TMP/resume-multi-state.json"
cp "$TMP/resume-multi-state.json" "$RESUME/state.json"
export FAKE_PR_STATES="$FAKE_PR_STATES
https://github.com/acme/web/pull/3=MERGED"
: > "$FAKE_WS_LOG"; : > "$FAKE_RESUME_STATE_LOG"; : > "$FAKE_RESUME_STEPS"
WS_LOOP_ID=RESUME1 WS_BIN="$TMP/resume-bin/ws" LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" \
  "$ROOT/loop-milestone.sh" resume --dir "$RESUME" > "$TMP/resume-multi.out" 2> "$TMP/resume-multi.err"
assert_exit "$?" 0 "resume advances from milestone two"
assert_eq "$([[ -f "$RESUME/milestones/m1.md" && -f "$RESUME/milestones/m2.md" ]] && print yes || print no)" yes "resume archives milestones one and two"
assert_eq "$(jq -c '[.milestone,.resumePending,.repositories["acme/api"].integrationBranch]' "$RESUME/state.json")" '[3,null,"feat/resume-multi-m3-api"]' "resume prepares the branch of milestone three"
assert_eq "$(jq -c 'select(.[1]=="loop.gate")|.[3]|fromjson|[.milestone,.prs]' "$FAKE_WS_LOG")" '[2,{"acme/web":"https://github.com/acme/web/pull/3"}]' "the gate event uses the parked milestone"
unset FAKE_RESUME_STATE_LOG FAKE_RESUME_STEPS
[[ "${1:-}" == --resume-only ]] && test_summary

echo "loop-milestone: open a fresh plan"
OPEN="$TMP/fresh/.loop"
mkdir -p "$OPEN"
cat > "$OPEN/plan.md" <<'PLAN'
<!-- loop-plan
planId: skills-fresh
-->
# Skills start
## Loop config
- **Integration branch:** `feat/skills-fresh`
## Repositories
### `acme/api`
- **Verify:** `true`
## Phases
### Phase 1 — Build API `[status: todo]`
- **Repository:** `acme/api`
- **Suggested branch:** `feat/api`
### Phase 2 — Review API `[kind: pr-review]` `[gate: merge]` `[status: todo]`
- **Depends on:** Phase 1
PLAN
make_fake_ws "$TMP/open-bin"
export FAKE_WS_LOG="$TMP/open-events.jsonl"
: > "$FAKE_WS_LOG"
WS_LOOP_ID=LOOP1 WS_BIN="$TMP/open-bin/ws" "$ROOT/loop-milestone.sh" open --dir "$OPEN" > "$TMP/open.out" 2> "$TMP/open.err"
assert_exit "$?" 0 "a plan alone starts a skills loop"
assert_eq "$(jq -c '[.runId,.milestone,.awaitingMerge,.phases["1"].status]' "$OPEN/state.json" 2>/dev/null)" '["skills-fresh",1,null,"todo"]' "fresh state starts at milestone one"
assert_eq "$(jq -r '.repositories["acme/api"].integrationBranch' "$OPEN/state.json" 2>/dev/null)" feat/skills-fresh "fresh state records the prepared repository"
for dir in runs answers hil notes; do
  assert_eq "$([[ -d "$OPEN/$dir" ]] && print yes)" yes "open creates $dir"
done
assert_eq "$(jq -s 'length' "$FAKE_WS_LOG")" 2 "open emits phases and milestone"
assert_eq "$(jq -c 'select(.[1]=="loop.phases") | .[3] | fromjson | .phases[1]' "$FAKE_WS_LOG")" '{"n":2,"title":"Review API","kind":"review","output":"pr","milestone":1,"gate":"merge","repo":null,"branch":null,"depends_on":[1],"status":"todo"}' "review phase uses the event schema"
echo "loop-milestone: open repeats without changes"
cp "$OPEN/state.json" "$TMP/open-before.json"
open_identity="$(stat -f "%i:%m" "$OPEN/state.json")"
WS_LOOP_ID=LOOP1 WS_BIN="$TMP/open-bin/ws" "$ROOT/loop-milestone.sh" open --dir "$OPEN" > "$TMP/repeat.out" 2> "$TMP/repeat.err"
assert_exit "$?" 0 "a second open succeeds"
assert_eq "$(cat "$OPEN/state.json")" "$(cat "$TMP/open-before.json")" "a second open preserves every state byte"
assert_eq "$(stat -f "%i:%m" "$OPEN/state.json")" "$open_identity" "a second open does not replace state"
assert_eq "$(jq -s 'length' "$FAKE_WS_LOG")" 2 "a second open calls no event append"
assert_eq "$(jq -s 'all(.[]; .[4:8]==["--source","tool","--target","LOOP1"] and .[8]=="--key" and (.[9]|length)>0)' "$FAKE_WS_LOG")" true "each append has tool provenance and a key"

# A later snapshot can return to earlier metadata. Its key must still be new.
sed 's/Build API/Build service/' "$OPEN/plan.md" > "$TMP/replan.md"
cp "$OPEN/plan.md" "$TMP/original-plan.md"
cp "$TMP/replan.md" "$OPEN/plan.md"
WS_LOOP_ID=LOOP1 WS_BIN="$TMP/open-bin/ws" "$ROOT/loop-milestone.sh" open --dir "$OPEN" > "$TMP/replan.out" 2> "$TMP/replan.err"
assert_exit "$?" 0 "open emits changed plan metadata"
cp "$TMP/original-plan.md" "$OPEN/plan.md"
WS_LOOP_ID=LOOP1 WS_BIN="$TMP/open-bin/ws" "$ROOT/loop-milestone.sh" open --dir "$OPEN" > "$TMP/restore.out" 2> "$TMP/restore.err"
assert_exit "$?" 0 "open accepts restored plan metadata"
assert_eq "$(jq -s '[.[]|select(.[1]=="loop.phases")|.[9]]|unique|length' "$FAKE_WS_LOG")" 3 "each successive snapshot has a separate key"

cat >> "$OPEN/plan.md" <<'PLAN'
### Phase 3 — Publish notes `[output: doc]` `[status: todo]`
### Phase 4 — Write a file `[output: file]` `[status: todo]`
### Phase 5 — Report results `[output: output.md]` `[status: todo]`
PLAN
WS_LOOP_ID=LOOP1 WS_BIN="$TMP/open-bin/ws" "$ROOT/loop-milestone.sh" open --dir "$OPEN" > "$TMP/outputs.out" 2> "$TMP/outputs.err"
assert_exit "$?" 0 "open adds phases with non-PR outputs"
assert_eq "$(jq -sc '[.[]|select(.[1]=="loop.phases")|.[3]|fromjson][-1].phases[2:]|map([.output,.repo,.branch])' "$FAKE_WS_LOG")" '[["doc",null,null],["file",null,null],["output.md",null,null]]' "non-PR phases use canonical outputs and no repository"

cp "$OPEN/state.json" "$TMP/no-loop-before.json"
"$ROOT/loop-milestone.sh" open --dir "$OPEN" > "$TMP/no-loop.out" 2> "$TMP/no-loop.err"
assert_exit "$?" 1 "open requires WS_LOOP_ID"
assert_eq "$(cat "$OPEN/state.json")" "$(cat "$TMP/no-loop-before.json")" "missing loop identity preserves state"
WS_LOOP_ID=LOOP1 "$ROOT/loop-milestone.sh" open --dir "$TMP/no-plan" > "$TMP/no-plan.out" 2>&1
assert_exit "$?" 1 "open requires a plan"
assert_eq "$([[ -e "$TMP/no-plan" ]] && print yes || print no)" no "a missing plan creates no directory"

PARKED="$TMP/parked-open"
mkdir -p "$PARKED"
print -r -- '{"runId":"parked","milestone":6,"awaitingMerge":{"milestone":6,"prs":{"acme/api":"https://example.test/1"}}}' > "$PARKED/state.json"
parked_before="$(cat "$PARKED/state.json")"
WS_LOOP_ID=LOOP1 "$ROOT/loop-milestone.sh" open --dir "$PARKED" > "$TMP/parked.out" 2> "$TMP/parked.err"
assert_exit "$?" 3 "a parked loop refuses open before parsing"
assert_contains "$(cat "$TMP/parked.err")" 'loop-milestone.sh resume' "a parked loop names resume"
assert_eq "$(cat "$PARKED/state.json")" "$parked_before" "a parked loop preserves state"
echo "loop-milestone: open retries a partial prepare"
TWO="$TMP/two/.loop"
mkdir -p "$TWO"
sed 's/loop-archive-fixture-2026-10-01-000000/skills-two/' "$HERE/fixtures/milestones/plan.md" > "$TWO/plan.md"
git -C "$TMP/web" worktree add -q -b feat/m1-web "$TMP/open-stolen" main
print dirty > "$TMP/open-stolen/dirty"
WS_LOOP_ID=LOOP2 WS_BIN="$TMP/open-bin/ws" "$ROOT/loop-milestone.sh" open --dir "$TWO" > "$TMP/two.out" 2> "$TMP/two.err"
assert_exit "$?" 4 "the second repository prepare failure surfaces"
assert_eq "$([[ -e "$TWO/state.json" ]] && print yes || print no)" no "a partial prepare writes no state"
FIRST_WT="$LOOP_WORKTREE_ROOT/skills-two/acme--api/integration"
first_head="$(git -C "$FIRST_WT" rev-parse HEAD)"
first_record="$(cat "$(git -C "$FIRST_WT" rev-parse --absolute-git-dir)/loop-repo-prepare.json")"
assert_eq "$([[ -d "$FIRST_WT" ]] && print yes)" yes "the first prepare keeps its worktree"
rm "$TMP/open-stolen/dirty"
git -C "$TMP/web" worktree remove "$TMP/open-stolen"
git -C "$TMP/web" branch -D feat/m1-web > /dev/null
WS_LOOP_ID=LOOP2 WS_BIN="$TMP/open-bin/ws" "$ROOT/loop-milestone.sh" open --dir "$TWO" > "$TMP/two-retry.out" 2> "$TMP/two-retry.err"
assert_exit "$?" 0 "a rerun prepares both repositories"
assert_eq "$(jq '.repositories|length' "$TWO/state.json")" 2 "the rerun records both repositories"
assert_eq "$(git -C "$FIRST_WT" rev-parse HEAD)" "$first_head" "the rerun reuses the first worktree"
assert_eq "$(cat "$(git -C "$FIRST_WT" rev-parse --absolute-git-dir)/loop-repo-prepare.json")" "$first_record" "the rerun keeps the first prepare record"
assert_eq "$(jq -c '[.phases[]|.status]|unique' "$TWO/state.json")" '["todo"]' "a fresh start ignores plan progress"

# Leave the legacy milestone tests free to prepare their own branches.
for repo in api web; do
  "$ROOT/loop-repo.sh" retire --repo "acme/$repo" --run-id skills-two \
    --worktree "$LOOP_WORKTREE_ROOT/skills-two/acme--$repo/integration" --adopted false >/dev/null
  git -C "$TMP/$repo" branch -D "feat/m1-$repo" >/dev/null
done

echo "loop-milestone: adopt milestone seven with six merged gates"
make_repo rstagi/dotfile
ADOPT="$TMP/adopt/.loop"
mkdir -p "$ADOPT"
cp "$HERE/fake/loop-skills-plan.md" "$ADOPT/plan.md"
jq -n '{runId:"skills-adopt",milestone:7,resumePending:7,awaitingMerge:null,
  custom:{keep:true},repositories:{"acme/retained":{integrationBranch:"keep",adopted:true}},
  phases:{"137":{status:"merged",attempt:2},"121":{status:"in-progress"},"122":{status:"blocked"}},
  milestones:([range(1;7)|{key:tostring,value:{prs:{"rstagi/dotfile":("https://example.test/pr/"+tostring)}}}]|from_entries)}' > "$ADOPT/state.json"
cp "$ADOPT/state.json" "$TMP/adopt-before.json"
export FAKE_WS_LOG="$TMP/adopt-events.jsonl"
: > "$FAKE_WS_LOG"
FAKE_WS_EXIT=1 WS_LOOP_ID=ADOPT1 WS_BIN="$TMP/open-bin/ws" "$ROOT/loop-milestone.sh" open --dir "$ADOPT" > "$TMP/adopt-failed.out" 2> "$TMP/adopt-failed.err"
assert_exit "$?" 1 "an event failure refuses adoption completion"
assert_eq "$(cat "$ADOPT/state.json")" "$(cat "$TMP/adopt-before.json")" "an event failure keeps resumePending and all saved state"
: > "$FAKE_WS_LOG"
WS_LOOP_ID=ADOPT1 WS_BIN="$TMP/open-bin/ws" "$ROOT/loop-milestone.sh" open --dir "$ADOPT" > "$TMP/adopt.out" 2> "$TMP/adopt.err"
assert_exit "$?" 0 "an advanced old-engine loop is adopted"
assert_eq "$(jq -c '[.milestone,.resumePending,.custom,.phases["137"],.phases["1"].status,.phases["138"].status]' "$ADOPT/state.json")" '[7,null,{"keep":true},{"status":"merged","attempt":2},"done","todo"]' "adoption preserves state and adds archived and new phases"
assert_eq "$(jq '.repositories|length' "$ADOPT/state.json")" 2 "adoption retains old repositories and prepares missing participants"
assert_eq "$(jq -s 'length' "$FAKE_WS_LOG")" 8 "adoption emits phases, milestone and six gates"
assert_eq "$(jq -c 'select(.[1]=="loop.milestone")|.[3]|fromjson' "$FAKE_WS_LOG")" '{"milestone":7}' "adoption starts at the saved milestone"
assert_eq "$(jq -sc '[.[]|select(.[1]=="loop.gate")|.[3]|fromjson|.milestone]' "$FAKE_WS_LOG")" '[1,2,3,4,5,6]' "history emits each merged gate in order"
assert_eq "$(jq -s 'all(.[]|select(.[1]=="loop.gate")|.[3]|fromjson; .kind=="merge" and .state=="merged" and (.prs["rstagi/dotfile"]|startswith("https://example.test/pr/")))' "$FAKE_WS_LOG")" true "history carries merged PR links"
body="$(jq -r 'select(.[1]=="loop.phases")|.[3]' "$FAKE_WS_LOG")"
assert_eq "$(jq -c '[.phases[]|select(.n==121 or .n==122)|.status]' <<< "$body")" '["running","todo"]' "event statuses map in-progress and blocked"
assert_eq "$(jq '.phases|length' <<< "$body")" 132 "the full WS v6 plan emits every phase"
cp "$ADOPT/state.json" "$TMP/adopt-opened.json"
WS_LOOP_ID=ADOPT1 WS_BIN="$TMP/open-bin/ws" "$ROOT/loop-milestone.sh" open --dir "$ADOPT" > "$TMP/adopt-repeat.out" 2> "$TMP/adopt-repeat.err"
assert_exit "$?" 0 "a second adoption succeeds"
assert_eq "$(cat "$ADOPT/state.json")" "$(cat "$TMP/adopt-opened.json")" "a second adoption preserves state"
assert_eq "$(jq -s 'length' "$FAKE_WS_LOG")" 8 "a second adoption emits nothing"
[[ "${1:-}" == --open-only ]] && test_summary
echo "loop-milestone: a sandbox wsd accepts the full plan phase body"
WS_RUST_BIN="${WS_MILESTONE_TEST_BIN:-$ROOT/wsd/target/debug/ws}"
[[ -x "$WS_RUST_BIN" ]] || { print -u2 "Build the ws test binary first."; exit 1; }
"$WS_RUST_BIN" daemon > "$TMP/ws-daemon.log" 2>&1 &
WS_TEST_DAEMON_PID=$!
deadline=$(( SECONDS + 30 ))
until [[ -S "$WS_SOCK" ]] && "$WS_RUST_BIN" ping >/dev/null 2>&1; do
  if ! kill -0 "$WS_TEST_DAEMON_PID" 2>/dev/null || (( SECONDS >= deadline )); then
    cat "$TMP/ws-daemon.log" >&2
    exit 1
  fi
  sleep 0.05
done
mkdir -p "$TMP/contract-loop"
contract_id="$("$WS_RUST_BIN" loop create --dir "$TMP/contract-loop" --json | jq -r .id)"
"$WS_RUST_BIN" event loop.phases --body "$body" --source tool --target "$contract_id" \
  --key full-plan > "$TMP/body-check.out" 2> "$TMP/body-check.err"
assert_exit "$?" 0 "the daemon accepts all phase fields for the full plan"
assert_eq "$("$WS_RUST_BIN" log "$contract_id" --type loop.phases | jq '.entries[0].body.phases|length')" 132 "the daemon records every plan phase"
kill "$WS_TEST_DAEMON_PID"
wait "$WS_TEST_DAEMON_PID"
WS_TEST_DAEMON_PID=""

export FAKE_WS_LOG="$TMP/ws.log"

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
cp "$HERE/fixtures/milestones/plan.md" "$TMP/plan.md"

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

echo "loop-milestone: quota preflight is advisory"
assert_contains "$(cat "$FAKE_WS_LOG" 2>/dev/null)" "usage
--preflight
--plan
$TMP/plan.md" "advance requests quota for the coordinator plan"
assert_contains "$(cat "$FAKE_WS_LOG" 2>/dev/null)" "--milestone
2" "advance scopes quota to the next milestone"
for mode in missing unknown failure; do
  case "$mode" in
    missing) export WS_BIN="$TMP/no-ws" FAKE_WS_EXIT=0 ;;
    unknown) export WS_BIN=ws FAKE_WS_EXIT=2 ;;
    failure) export WS_BIN=ws FAKE_WS_EXIT=9 ;;
  esac
  quota_patch="$($ROOT/loop-milestone.sh advance --state "$STATE" --next 2 --branches "$BRANCHES" 2>"$TMP/quota.err")"; rc=$?
  assert_exit "$rc" "0" "$mode quota command never blocks advance"
  assert_eq "$quota_patch" "$out2" "$mode quota command preserves the state patch"
  assert_contains "$(cat "$TMP/quota.err")" "no quota data" "$mode quota command warns"
done
unset WS_BIN FAKE_WS_EXIT
FAKE_WS_OUTPUT='quota warning from ws' $ROOT/loop-milestone.sh advance --state "$STATE" --next 2 --branches "$BRANCHES" >"$TMP/quota.patch" 2>"$TMP/quota.err"
assert_exit "$?" "0" "quota warning does not block advance"
assert_eq "$(cat "$TMP/quota.patch")" "$out2" "quota output does not corrupt the JSON patch"
assert_contains "$(cat "$TMP/quota.err")" 'quota warning from ws' "quota output reaches stderr"

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
next_web="$(node "$ROOT/loop-plan-parse.mjs" --plan "$COORD/plan.md" | jq -r '.milestones[1].integrationBranches["acme/web"]')"
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


echo "loop-milestone: park a skills loop at a merge gate"
PARK="$TMP/park/.loop"
mkdir -p "$PARK"
cat > "$PARK/plan.md" <<'PLAN'
<!-- loop-plan
planId: skills-park
-->
# Park fixture — Multi-Phase Plan

**Status:** in progress

## Repositories

### `acme/api`
- **Verify:** `true`

### `acme/web`
- **Verify:** `true`

## Milestone 1 — Ship both
- **Integration branch (acme/api):** `feat/p1-api`
- **Integration branch (acme/web):** `feat/p1-web`

### Phase 1 — Build API `[lane: A]` `[status: in-progress]`
- **Repository:** `acme/api`
- **Suggested branch:** `feat/p1-a`

### Phase 2 — Build web `[lane: B]` `[status: todo]`
- **Repository:** `acme/web`
- **Suggested branch:** `feat/p1-w`

### Phase 3 — Review milestone 1 `[lane: review]` `[status: todo]` `[kind: pr-review]` `[gate: merge]`
- **Depends on:** Phase 1, Phase 2

## Milestone 2 — Later
- **Integration branch (acme/api):** `feat/p2-api`

### Phase 4 — Later work `[lane: A]` `[status: todo]`
- **Repository:** `acme/api`
- **Depends on:** Phase 3
- **Suggested branch:** `feat/p2-a`

### Phase 5 — Review milestone 2 `[lane: review]` `[status: todo]` `[kind: pr-review]`
- **Depends on:** Phase 4
PLAN
jq -n '{runId:"skills-park",milestone:1,awaitingMerge:null,phases:{},repositories:{"acme/api":{integrationBranch:"feat/p1-api"},"acme/web":{integrationBranch:"feat/p1-web"}}}' > "$PARK/state.json"
mkdir -p "$TMP/park-bin"
cat > "$TMP/park-bin/ws" <<'WS'
#!/bin/sh
if [ "$1" = loop ] && [ "$2" = get ]; then
  cat "$FAKE_LOOP_GET"
  exit 0
fi
jq -cn '$ARGS.positional' --args -- "$@" >> "$FAKE_WS_LOG"
WS
chmod +x "$TMP/park-bin/ws"
export FAKE_WS_LOG="$TMP/park-events.jsonl" FAKE_LOOP_GET="$TMP/park-loop.json" EMIT_LOG="$TMP/park-emit.log"
: > "$FAKE_WS_LOG"; : > "$EMIT_LOG"
PARK_PRS='{"acme/api":"https://github.com/acme/api/pull/7","acme/web":"https://github.com/acme/web/pull/8"}'
park() { WS_LOOP_ID=LOOP1 WS_BIN="$TMP/park-bin/ws" LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" "$ROOT/loop-milestone.sh" park --dir "$PARK" --milestone 1 --prs "${1:-$PARK_PRS}" 2> "$TMP/park.err"; }
loop_json() {
  jq -n --arg s1 "$1" --arg s2 "$2" --arg s3 "$3" --argjson gates "${4:-{\}}" '{milestone:1,merge_gates:$gates,phases:{
    "1":{n:1,milestone:1,status:$s1},"2":{n:2,milestone:1,status:$s2},"3":{n:3,milestone:1,status:$s3},
    "4":{n:4,milestone:2,status:"todo"}}}' > "$FAKE_LOOP_GET"
}

loop_json merged running todo
cp "$PARK/plan.md" "$TMP/park-plan-before.md"; cp "$PARK/state.json" "$TMP/park-state-before.json"
park >/dev/null
assert_exit "$?" 3 "an open phase exits 3"
assert_eq "$(cat "$PARK/plan.md")" "$(cat "$TMP/park-plan-before.md")" "an open phase leaves the plan unchanged"
assert_eq "$(cat "$PARK/state.json")" "$(cat "$TMP/park-state-before.json")" "an open phase leaves the state unchanged"
assert_eq "$(wc -c < "$FAKE_WS_LOG" | tr -d ' ')" 0 "an open phase appends no event"
assert_eq "$(wc -c < "$EMIT_LOG" | tr -d ' ')" 0 "an open phase pushes no plan"

WS_BIN="$TMP/park-bin/ws" "$ROOT/loop-milestone.sh" park --dir "$PARK" --milestone 1 --prs "$PARK_PRS" >/dev/null 2>&1
assert_exit "$?" 1 "park needs WS_LOOP_ID"
WS_LOOP_ID=LOOP1 WS_BIN="$TMP/park-bin/ws" "$ROOT/loop-milestone.sh" park --dir "$PARK" --milestone 2 --prs "$PARK_PRS" >/dev/null 2>&1
assert_exit "$?" 1 "park refuses a milestone other than the state milestone"

loop_json merged done merged
park >/dev/null
assert_exit "$?" 0 "park succeeds when every phase of the milestone is done or merged"
parsed="$(node "$ROOT/loop-plan-parse.mjs" --plan "$PARK/plan.md")"
assert_eq "$(jq -c '[.phases[]|select(.milestone==1)|.status]' <<< "$parsed")" '["done","done","done"]' "the parser reads done for each phase of milestone 1"
assert_eq "$(jq -c '[.phases[]|select(.milestone==2)|.status]' <<< "$parsed")" '["todo","todo"]' "later milestones keep their status"
assert_eq "$(jq -c '.milestones[0].prs' <<< "$parsed")" '{"acme/api":"https://github.com/acme/api/pull/7","acme/web":"https://github.com/acme/web/pull/8"}' "the parser reads the PR lines of milestone 1"
assert_eq "$(jq -r '.status' <<< "$parsed")" "awaiting merge" "the plan Status is awaiting merge"
assert_eq "$(grep -c 'skills-park register' "$EMIT_LOG")" 1 "park pushes the plan once"
assert_eq "$(jq -c '[.awaitingMerge, .repositories["acme/api"].prUrl, .repositories["acme/web"].prUrl, .repositories["acme/api"].integrationBranch]' "$PARK/state.json")" '[{"milestone":1,"prs":{"acme/api":"https://github.com/acme/api/pull/7","acme/web":"https://github.com/acme/web/pull/8"}},"https://github.com/acme/api/pull/7","https://github.com/acme/web/pull/8","feat/p1-api"]' "state records awaitingMerge and the PR URLs"
assert_eq "$(jq -s 'length' "$FAKE_WS_LOG")" 1 "park appends one event"
assert_eq "$(jq -c '.[1], (.[3]|fromjson), .[4:8]' "$FAKE_WS_LOG" | tr '\n' ' ')" '"loop.gate" {"kind":"merge","state":"open","milestone":1,"prs":{"acme/api":"https://github.com/acme/api/pull/7","acme/web":"https://github.com/acme/web/pull/8"}} ["--source","tool","--target","LOOP1"] ' "the gate event has tool provenance"
assert_eq "$(jq -r '.[8]' "$FAKE_WS_LOG")" "--key" "the gate event has a key"

echo "loop-milestone: park repeats without changes"
cp "$PARK/plan.md" "$TMP/park-plan-after.md"; cp "$PARK/state.json" "$TMP/park-state-after.json"
loop_json merged done merged '{"1":"open"}'
park >/dev/null
assert_exit "$?" 0 "a second park with the same PRs succeeds"
assert_eq "$(cat "$PARK/plan.md")" "$(cat "$TMP/park-plan-after.md")" "a second park leaves the plan unchanged"
assert_eq "$(cat "$PARK/state.json")" "$(cat "$TMP/park-state-after.json")" "a second park leaves the state unchanged"
assert_eq "$(jq -s 'length' "$FAKE_WS_LOG")" 1 "a second park appends no event"
assert_eq "$(grep -c 'skills-park register' "$EMIT_LOG")" 1 "a second park pushes no plan"
echo "loop-milestone: park recovers a lost gate event"
loop_json merged done merged
park >/dev/null
assert_exit "$?" 0 "a repeat appends the gate when the loop shows none"
assert_eq "$(jq -s 'length' "$FAKE_WS_LOG")" 2 "the repeat re-appends the keyed gate event"
assert_eq "$(jq -s '[.[]|.[9]]|unique|length' "$FAKE_WS_LOG")" 1 "both appends carry the same key"

echo "loop-milestone: park with other PRs"
park '{"acme/api":"https://github.com/acme/api/pull/9","acme/web":"https://github.com/acme/web/pull/8"}' >/dev/null
assert_exit "$?" 1 "other PRs for a parked milestone exit 1"
assert_eq "$(cat "$PARK/plan.md")" "$(cat "$TMP/park-plan-after.md")" "other PRs leave the plan unchanged"
assert_eq "$(cat "$PARK/state.json")" "$(cat "$TMP/park-state-after.json")" "other PRs leave the state unchanged"
assert_eq "$(jq -s 'length' "$FAKE_WS_LOG")" 2 "other PRs append no event"

echo "loop-milestone: park a plan without milestone sections"
IMPLICIT="$TMP/park-implicit/.loop"
mkdir -p "$IMPLICIT"
cat > "$IMPLICIT/plan.md" <<'PLAN'
<!-- loop-plan
planId: skills-implicit
-->
# Implicit

**Status:** in progress

## Repositories

### `acme/api`
- **Verify:** `true`
- **PR:** _none yet_

## Phases

### Phase 1 — Build `[status: in-progress]`
- **Repository:** `acme/api`
PLAN
jq -n '{runId:"skills-implicit",milestone:1,awaitingMerge:null}' > "$IMPLICIT/state.json"
jq -n '{milestone:1,phases:{"1":{n:1,milestone:1,status:"done"}}}' > "$FAKE_LOOP_GET"
WS_LOOP_ID=LOOP1 WS_BIN="$TMP/park-bin/ws" LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" "$ROOT/loop-milestone.sh" park --dir "$IMPLICIT" --milestone 1 --prs '{"acme/api":"https://github.com/acme/api/pull/5"}' >/dev/null 2>&1
assert_exit "$?" 0 "park accepts a plan with one implicit milestone"
parsed="$(node "$ROOT/loop-plan-parse.mjs" --plan "$IMPLICIT/plan.md")"
assert_eq "$(jq -c '[.phases[0].status, .repositories[0].pr, .status]' <<< "$parsed")" '["done","https://github.com/acme/api/pull/5","awaiting merge"]' "the implicit milestone keeps its PR on the repository"

test_summary
