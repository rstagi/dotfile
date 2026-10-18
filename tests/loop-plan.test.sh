#!/bin/zsh
# loop-plan.sh — register/push/get/list/note against the central daemon. Part 1 drives the
# fake-emit seam (no daemon); Part 2 boots a real `node server/index.mjs --daemon` on a random
# port + tmp store and round-trips a plan through the HTTP API (skipped when node < 22.18).
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
FAKE="$HERE/fake"

TMP="$(mktemp -d)"
DAEMON_PID=""
STORE=""
cleanup() {
  [[ -n "$DAEMON_PID" ]] && kill "$DAEMON_PID" 2>/dev/null
  [[ -n "$DAEMON_PID" ]] && wait "$DAEMON_PID" 2>/dev/null
  rm -rf "$TMP" "$STORE"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

plan() { zsh "$ROOT/loop-plan.sh" "$@"; }

# ---------------------------------------------------------------------------------------
# Part 1 — fake-emit seam (no daemon touched)
# ---------------------------------------------------------------------------------------
export LOOP_EMIT_SH="$FAKE/loop-emit.sh"
export LOOP_DAEMON_URL="http://127.0.0.1:9"   # unreachable — the fake never curls anyway

echo "loop-plan: register — planId from the plan header wins over minting"
D1="$TMP/repo1/.loop"; mkdir -p "$D1"
cat > "$D1/plan.md" <<'EOF'
<!-- loop-plan
planId: loop-fixed-2026-01-01-000000
daemon: http://localhost:7717
-->
# Fixed effort — Multi-Phase Plan
EOF
export EMIT_LOG="$TMP/emit1.log"; : > "$EMIT_LOG"
OUT="$(plan register --dir "$D1" --effort "Whatever")"
assert_eq "$OUT" "loop-fixed-2026-01-01-000000" "prints the header planId"
assert_contains "$(cat "$EMIT_LOG")" "loop-fixed-2026-01-01-000000 register" "POSTs register for the header planId"
assert_contains "$(cat "$EMIT_LOG")" "\"effort\":\"Whatever\"" "register body carries the effort"
assert_contains "$(cat "$EMIT_LOG")" "$D1/plan.md" "register body carries the abs planFile"

echo "loop-plan: register — mints when the header has no planId, then writes it back"
D2="$TMP/repo2/.loop"; mkdir -p "$D2"
cat > "$D2/plan.md" <<'EOF'
<!-- loop-plan
daemon: http://localhost:7717
-->
# Cart revamp — Multi-Phase Plan
EOF
export EMIT_LOG="$TMP/emit2.log"; : > "$EMIT_LOG"
MINTED="$(plan register --dir "$D2" --effort "Cart revamp")"
assert_eq "$([[ "$MINTED" == loop-cart-revamp-* ]] && echo yes)" "yes" "mints loop-cart-revamp-<date>-<time> ($MINTED)"
assert_contains "$(cat "$D2/plan.md")" "planId: $MINTED" "writes the minted planId back into the header"
# push (no --effort) must resolve the SAME id from the now-written header — one daemon record.
PUSHED="$(plan push --dir "$D2")"
assert_eq "$PUSHED" "$MINTED" "push re-reads the header → same planId (idempotent record)"

echo "loop-plan: note — posts an injection-safe progress.note event"
export EMIT_LOG="$TMP/emit3.log"; : > "$EMIT_LOG"
plan note --plan-id "loop-n" --phase 3 --body 'a "quoted" $note `x`' >/dev/null
line="$(cat "$EMIT_LOG")"
assert_contains "$line" "loop-n event" "POSTs an event for the plan"
assert_contains "$line" "progress.note" "event name is progress.note"
assert_contains "$line" "\"phase\":\"3\"" "carries the phase"
body="${line#loop-n event }"
assert_eq "$(printf '%s' "$body" | jq -r '.detail')" 'a "quoted" $note `x`' "detail round-trips verbatim (injection-safe)"

echo "loop-plan: usage errors"
plan get >/dev/null 2>&1; assert_exit "$?" "1" "get without --plan-id fails"
plan note --plan-id x >/dev/null 2>&1; assert_exit "$?" "1" "note without --body fails"
plan bogus >/dev/null 2>&1; assert_exit "$?" "1" "unknown command fails"
plan archive --dir "$D1" >/dev/null 2>&1; assert_exit "$?" "1" "archive without --milestone fails"

# ---------------------------------------------------------------------------------------
# Archive — fixture plan with 3 milestones, milestone 1 shipped (fake gh + fake emit)
# ---------------------------------------------------------------------------------------
FIX="$HERE/fixtures/milestones"
API_PR=https://github.com/acme/api/pull/1
WEB_PR=https://github.com/acme/web/pull/2
mkdir -p "$TMP/bin"
# Fake gh: PR state from FAKE_PR_STATES ("<url>=<STATE>" lines); FAKE_GH_FAIL=1 fails every call.
cat > "$TMP/bin/gh" <<'GH'
#!/bin/sh
[ "${FAKE_GH_FAIL:-0}" = 0 ] || exit 1
state="$(printf '%s\n' "$FAKE_PR_STATES" | sed -n "s|^$3=||p")"
[ -n "$state" ] || exit 1
printf '{"state":"%s"}\n' "$state"
GH
chmod +x "$TMP/bin/gh"
OLD_PATH="$PATH"
export PATH="$TMP/bin:$PATH"
merged() { export FAKE_PR_STATES="$API_PR=MERGED
$WEB_PR=MERGED"; }

# fresh_loop <name>: a .loop dir holding the fixture plan and a state.json with phase entries.
fresh_loop() {
  local d="$TMP/$1/.loop"
  mkdir -p "$d"
  cp "$FIX/plan.md" "$d/plan.md"
  print -r -- '{"runId":"loop-archive-fixture-2026-10-01-000000","milestone":2,"phases":{"1":{"status":"done"},"3":{"status":"done"},"4":{"status":"running"}}}' > "$d/state.json"
  print -r -- "$d"
}
files_in() { (cd "$1" && print -l -- *(DN)) }

echo "loop-plan: archive preserves the tool error and exit status"
FAILED_TOOL="$TMP/failing-tool"; mkdir -p "$FAILED_TOOL"
cp "$ROOT/loop-plan.sh" "$FAILED_TOOL/loop-plan.sh"
cat > "$FAILED_TOOL/loop-plan-archive.mjs" <<'JS'
console.error('archive: fixture read failed');
process.exit(42);
JS
FT="$(fresh_loop archive-tool-error)"
OUT="$(zsh "$FAILED_TOOL/loop-plan.sh" archive --milestone 1 --dir "$FT" 2>&1)"; rc=$?
assert_exit "$rc" "42" "archive returns the tool status"
assert_eq "$OUT" "archive: fixture read failed" "archive preserves the tool error without a Node requirement"
assert_eq "$(cat "$FT/plan.md")" "$(cat "$FIX/plan.md")" "tool failure leaves the plan unchanged"
assert_eq "$(files_in "$FT")" $'plan.md\nstate.json' "tool failure removes temporary files"

echo "loop-plan: archive explains a missing node"
MN="$(fresh_loop archive-no-node)"
OUT="$(PATH="$(path_without_command node)" plan archive --milestone 1 --dir "$MN" 2>&1)"; rc=$?
assert_exit "$rc" "1" "archive fails when node is missing"
assert_contains "$OUT" "needs Node >= 22.18" "missing node produces the Node requirement"
assert_eq "$(cat "$MN/plan.md")" "$(cat "$FIX/plan.md")" "missing node leaves the plan unchanged"

echo "loop-plan: archive — moves milestone 1 into milestones/m1.md, leaves the stub, pushes once"
A="$(fresh_loop archive-ok)"
STATE_BEFORE="$(cat "$A/state.json")"
merged
export EMIT_LOG="$TMP/emit-archive.log"; : > "$EMIT_LOG"
OUT="$(plan archive --milestone 1 --dir "$A" 2>&1)"; rc=$?
assert_exit "$rc" "0" "archive succeeds"
assert_eq "$(cat "$A/plan.md")" "$(cat "$FIX/archived-m1/plan.md")" "plan.md is the index with the milestone 1 stub"
assert_eq "$(cat "$A/milestones/m1.md")" "$(cat "$FIX/archived-m1/milestones/m1.md")" "milestones/m1.md holds the moved blocks and log entries"
assert_eq "$(cat "$A/state.json")" "$STATE_BEFORE" "state.json keeps its entries for archived phases"
assert_contains "$OUT" "archived milestone 1" "reports the archive"
assert_eq "$(grep -c ' register ' "$EMIT_LOG")" "1" "pushes the plan once"
assert_contains "$(cat "$EMIT_LOG")" "loop-archive-fixture-2026-10-01-000000 register" "pushes under the header planId"
assert_eq "$(files_in "$A")" "milestones
plan.md
state.json" "leaves no temp files"

echo "loop-plan: archive — a second run is a no-op"
: > "$EMIT_LOG"
PLAN_AFTER="$(cat "$A/plan.md")"; ARCHIVE_AFTER="$(cat "$A/milestones/m1.md")"
OUT="$(plan archive --milestone 1 --dir "$A" 2>&1)"; rc=$?
assert_exit "$rc" "0" "second run succeeds"
assert_contains "$OUT" "already archived" "says the milestone is already archived"
assert_eq "$(cat "$A/plan.md")" "$PLAN_AFTER" "plan.md unchanged"
assert_eq "$(cat "$A/milestones/m1.md")" "$ARCHIVE_AFTER" "archive file unchanged"
assert_eq "$(cat "$EMIT_LOG")" "" "no push"

echo "loop-plan: archive --outcome — a text with spaces reaches the stub as one argument"
O="$(fresh_loop archive-outcome)"
OUT="$(plan archive --milestone 1 --dir "$O" --outcome "The API and the shell are live" 2>&1)"; rc=$?
assert_exit "$rc" "0" "archive with --outcome succeeds ($OUT)"
assert_contains "$(cat "$O/plan.md")" "- **Outcome:** The API and the shell are live" "the stub carries the --outcome text"

echo "loop-plan: archive — a pending retro goes first in the archive, its Outcome into the stub, then it is removed"
RT="$(fresh_loop archive-retro)"
mkdir -p "$RT/milestones"; cp "$FIX/m1.retro.md" "$RT/milestones/m1.retro.md"
: > "$EMIT_LOG"
OUT="$(plan archive --milestone 1 --dir "$RT" 2>&1)"; rc=$?
assert_exit "$rc" "0" "archive with a retro succeeds ($OUT)"
assert_eq "$(cat "$RT/plan.md")" "$(cat "$FIX/archived-m1-retro/plan.md")" "the stub Outcome is the retro Outcome"
assert_eq "$(cat "$RT/milestones/m1.md")" "$(cat "$FIX/archived-m1-retro/milestones/m1.md")" "the archive holds the retro, then ## Raw log"
assert_eq "$(files_in "$RT/milestones")" "m1.md" "the retro file is removed"
assert_eq "$(grep -c ' register ' "$EMIT_LOG")" "1" "pushes the plan once"

echo "loop-plan: archive — an archived milestone with a pending retro attaches it; without one it stays a no-op"
AT="$TMP/archive-attach/.loop"; mkdir -p "$AT/milestones"
cp "$FIX/archived-m1/plan.md" "$AT/plan.md"; cp "$FIX/archived-m1/milestones/m1.md" "$AT/milestones/m1.md"
cp "$FIX/m1.retro.md" "$AT/milestones/m1.retro.md"
: > "$EMIT_LOG"
OUT="$(plan archive --milestone 1 --dir "$AT" 2>&1)"; rc=$?
assert_exit "$rc" "0" "attach succeeds ($OUT)"
assert_contains "$OUT" "attached" "reports the attach"
assert_eq "$(cat "$AT/plan.md")" "$(cat "$FIX/archived-m1-retro/plan.md")" "attach updates the stub Outcome"
assert_eq "$(cat "$AT/milestones/m1.md")" "$(cat "$FIX/archived-m1-retro/milestones/m1.md")" "attach puts the retro before the raw log"
assert_eq "$(files_in "$AT/milestones")" "m1.md" "attach removes the retro file"
assert_eq "$(grep -c ' register ' "$EMIT_LOG")" "1" "attach pushes the plan once"
: > "$EMIT_LOG"
OUT="$(plan archive --milestone 1 --dir "$AT" 2>&1)"; rc=$?
assert_exit "$rc" "0" "a run without a pending retro succeeds"
assert_contains "$OUT" "already archived" "it is a no-op"
assert_eq "$(cat "$EMIT_LOG")" "" "no push"
rm "$AT/milestones/m1.md"; cp "$FIX/m1.retro.md" "$AT/milestones/m1.retro.md"
plan archive --milestone 1 --dir "$AT" >/dev/null 2>&1
assert_exit "$?" "1" "attach fails when the archive file is missing"
assert_eq "$(cat "$AT/plan.md")" "$(cat "$FIX/archived-m1-retro/plan.md")" "plan.md unchanged after a failed attach"
assert_eq "$(files_in "$AT/milestones")" "m1.retro.md" "the retro file stays after a failed attach"

echo "loop-plan: archive — refuses while a phase of the milestone is not done"
R="$(fresh_loop archive-open)"
: > "$EMIT_LOG"
OUT="$(plan archive --milestone 2 --dir "$R" 2>&1)"; rc=$?
assert_exit "$rc" "3" "refused"
assert_contains "$OUT" "Phase 4 (in-progress)" "names the open phase"
assert_eq "$(cat "$R/plan.md")" "$(cat "$FIX/plan.md")" "plan.md unchanged"
assert_eq "$(files_in "$R")" "plan.md
state.json" "no archive file, no temp files"
assert_eq "$(cat "$EMIT_LOG")" "" "no push"

echo "loop-plan: archive — refuses unless every gate PR is merged"
export FAKE_PR_STATES="$API_PR=MERGED
$WEB_PR=OPEN"
OUT="$(plan archive --milestone 1 --dir "$R" 2>&1)"; rc=$?
assert_exit "$rc" "3" "an open gate PR refuses"
assert_contains "$OUT" "acme/web	$WEB_PR	open" "names the open PR"
export FAKE_PR_STATES="$API_PR=CLOSED
$WEB_PR=OPEN"
plan archive --milestone 1 --dir "$R" >/dev/null 2>&1
assert_exit "$?" "4" "a closed unmerged gate PR wins over open"
merged
FAKE_GH_FAIL=1 plan archive --milestone 1 --dir "$R" >/dev/null 2>&1
assert_exit "$?" "1" "a gh failure refuses"
assert_eq "$(cat "$R/plan.md")" "$(cat "$FIX/plan.md")" "plan.md unchanged after every refusal"
assert_eq "$(files_in "$R")" "plan.md
state.json" "no archive file after every refusal"
assert_eq "$(cat "$EMIT_LOG")" "" "no push after any refusal"

echo "loop-plan: archive — atomic: a failed write leaves plan.md unchanged"
F="$(fresh_loop archive-fail)"
print -r -- "not a directory" > "$F/milestones"
plan archive --milestone 1 --dir "$F" >/dev/null 2>&1
assert_exit "$?" "1" "an archive dir that cannot be created fails"
assert_eq "$(cat "$F/plan.md")" "$(cat "$FIX/plan.md")" "plan.md unchanged"
assert_eq "$(files_in "$F")" "milestones
plan.md
state.json" "no temp files left"
rm -f "$F/milestones"; mkdir "$F/milestones"; chmod 555 "$F"
plan archive --milestone 1 --dir "$F" >/dev/null 2>&1; rc=$?
chmod 755 "$F"
assert_exit "$rc" "1" "a read-only loop dir fails"
assert_eq "$(cat "$F/plan.md")" "$(cat "$FIX/plan.md")" "plan.md unchanged"
assert_eq "$(files_in "$F/milestones")" "" "no archive file written"
print -r -- "other content" > "$F/milestones/m1.md"
plan archive --milestone 1 --dir "$F" >/dev/null 2>&1
assert_exit "$?" "1" "an archive file with other content is never overwritten"
assert_eq "$(cat "$F/plan.md")" "$(cat "$FIX/plan.md")" "plan.md unchanged"
assert_eq "$(cat "$F/milestones/m1.md")" "other content" "existing archive file kept"
assert_eq "$(cat "$EMIT_LOG")" "" "no push after a failed write"
export PATH="$OLD_PATH"
unset FAKE_PR_STATES

# ---------------------------------------------------------------------------------------
# Part 2 — real daemon end-to-end (skipped when node < 22.18)
# ---------------------------------------------------------------------------------------
unset LOOP_EMIT_SH EMIT_LOG

node_ok=0
if command -v node >/dev/null 2>&1 && \
   node -e 'const [a,b]=process.versions.node.split(".").map(Number); process.exit((a>22||(a===22&&b>=18))?0:1)' 2>/dev/null; then
  node_ok=1
fi

wait_health() {
  local i=0
  while [ $i -lt 40 ]; do
    curl -sf -m 1 "$LOOP_DAEMON_URL/api/health" >/dev/null 2>&1 && return 0
    sleep 0.25; i=$((i + 1))
  done
  return 1
}

if [[ $node_ok -eq 0 ]]; then
  echo "loop-plan: SKIP end-to-end daemon test (node < 22.18 or missing)"
else
  echo "loop-plan: end-to-end — register → planned → GET /plan → state flips active → note on timeline"
  PORT=$(( (RANDOM % 1000) + 7810 ))
  export LOOP_DAEMON_URL="http://127.0.0.1:$PORT"
  STORE="$(mktemp -d)"
  LOOP_STORE_DIR="$STORE" node "$ROOT/loop-web/server/index.mjs" --daemon --port "$PORT" >"$TMP/daemon.log" 2>&1 &
  DAEMON_PID=$!

  if ! wait_health; then
    echo "loop-plan: SKIP end-to-end — daemon did not come up on :$PORT (see $TMP/daemon.log)"
  else
    E="$TMP/e2e/.loop"; mkdir -p "$E"
    cat > "$E/plan.md" <<'EOF'
<!-- loop-plan
daemon: http://localhost:7717
-->
# Demo effort — Multi-Phase Plan

## Goal
Demo the loop-plan CLI round-trip.
EOF
    PID="$(plan register --dir "$E" --effort demo)"
    assert_eq "$([[ "$PID" == loop-demo-* ]] && echo yes)" "yes" "register mints loop-demo-* ($PID)"

    st="$(curl -sf "$LOOP_DAEMON_URL/api/loops" | jq -r --arg id "$PID" '.[] | select(.runId==$id) | .status')"
    assert_eq "$st" "planned" "a register-only loop shows status planned in /api/loops"

    curl -sf -X POST --data-binary '{"event":"jev.decision","phase":"1","attempt":1,"stage":"route","mode":"shadow","candidate":"light","confidence":0.9,"probabilities":{"light":0.9},"appliedAction":"default","fallbackReason":"shadow-mode","resolvedModel":"systemone","ts":"2026-09-27T10:00:00Z"}' \
      "$LOOP_DAEMON_URL/api/loops/$PID/event" >/dev/null
    jev_candidate="$(curl -sf "$LOOP_DAEMON_URL/api/loops/$PID/snapshot" | jq -r '.decisions[0].candidate')"
    assert_eq "$jev_candidate" "light" "POST event preserves typed Jev decision in snapshot"
    st="$(curl -sf "$LOOP_DAEMON_URL/api/loops" | jq -r --arg id "$PID" '.[] | select(.runId==$id) | .status')"
    assert_eq "$st" "planned" "pre-attempt Jev POST does not activate a planned loop"

    got="$(plan get --plan-id "$PID")"
    assert_contains "$got" "Demo the loop-plan CLI round-trip." "get round-trips the plan markdown"

    assert_contains "$(plan list)" "$PID" "list shows the registered plan"

    # A state push flips planned → active.
    curl -sf -X POST --data-binary '{"runId":"'"$PID"'","phases":{"1":{"slug":"a","status":"running"}}}' \
      "$LOOP_DAEMON_URL/api/loops/$PID/state" >/dev/null
    st2="$(curl -sf "$LOOP_DAEMON_URL/api/loops" | jq -r --arg id "$PID" '.[] | select(.runId==$id) | .status')"
    assert_eq "$st2" "active" "a state push flips the loop planned → active"

    # A progress note lands on the loop timeline.
    plan note --plan-id "$PID" --phase 1 --body "hello from the timeline" >/dev/null
    note_hit="$(curl -sf "$LOOP_DAEMON_URL/api/loops/$PID/snapshot" | jq -r '[.events[]? | select(.detail=="hello from the timeline")] | length')"
    assert_eq "$note_hit" "1" "the progress.note event appears on the timeline"

    # Plural repository finish payloads round-trip; review retrieval is repository-selectable.
    curl -sf -X POST --data-binary '{"repositories":{"acme/api":{"integrationBranch":"feat/api","prUrl":"https://github.com/acme/api/pull/1"},"acme/web":{"integrationBranch":"feat/web","prUrl":"https://github.com/acme/web/pull/2"}},"phases":{"1":{"repository":"acme/api","status":"merged"}}}' \
      "$LOOP_DAEMON_URL/api/loops/$PID/state" >/dev/null
    curl -sf -X POST --data-binary '{"repositories":{"acme/api":{"prUrl":"https://github.com/acme/api/pull/1","review":{"outcome":"done","summary":"api ok","reportPath":null,"commentUrl":null}},"acme/web":{"prUrl":"https://github.com/acme/web/pull/2","review":{"outcome":"blocked","summary":"web fix","reportPath":null,"commentUrl":null}}}}' \
      "$LOOP_DAEMON_URL/api/loops/$PID/finish" >/dev/null
    api_review="$(curl -sf "$LOOP_DAEMON_URL/api/loops/$PID/review?repository=acme%2Fapi")"
    assert_eq "$(print "$api_review" | jq -r .outcome)" "done" "repository review endpoint selects API review"
    assert_eq "$(print "$api_review" | jq -r .prUrl)" "https://github.com/acme/api/pull/1" "repository review endpoint selects API PR"
    aggregate="$(curl -sf "$LOOP_DAEMON_URL/api/loops" | jq -r --arg id "$PID" '.[] | select(.runId==$id) | .reviewOutcome')"
    assert_eq "$aggregate" "blocked" "loop summary aggregates blocked over done"

    # A finished explicit-review flow reopens in place when new unfinished phases are pushed.
    EDIT="$TMP/editable/.loop"; mkdir -p "$EDIT"
    cat > "$EDIT/plan.md" <<'EOF'
<!-- loop-plan
planId: loop-editable-e2e
daemon: http://localhost:7717
-->
# Editable — Multi-Phase Plan
## Phases
### Phase 1 — Initial work `[lane: A]` `[status: done]`
- **Depends on:** none
### Phase 2 — First review `[lane: review]` `[status: done]` `[kind: pr-review]`
- **Depends on:** Phase 1
EOF
    EDIT_PID="$(plan register --dir "$EDIT")"
    curl -sf -X POST --data-binary '{"phases":{"1":{"status":"merged"},"2":{"status":"done"}}}' \
      "$LOOP_DAEMON_URL/api/loops/$EDIT_PID/state" >/dev/null
    curl -sf -X POST --data-binary '{}' "$LOOP_DAEMON_URL/api/loops/$EDIT_PID/finish" >/dev/null
    assert_eq "$(curl -sf "$LOOP_DAEMON_URL/api/loops" | jq -r --arg id "$EDIT_PID" '.[] | select(.runId==$id) | .status')" \
      "finished" "explicit flow finishes after its review phase"

    cat >> "$EDIT/plan.md" <<'EOF'
### Phase 3 — Follow-up work `[lane: A]` `[status: todo]`
- **Depends on:** Phase 2
### Phase 4 — Final review `[lane: review]` `[status: todo]` `[kind: pr-review]`
- **Depends on:** Phase 3
EOF
    plan push --dir "$EDIT" >/dev/null
    assert_eq "$(curl -sf "$LOOP_DAEMON_URL/api/loops" | jq -r --arg id "$EDIT_PID" '.[] | select(.runId==$id) | .status')" \
      "active" "pushing appended phases reopens the same loop"
    ids="$(curl -sf "$LOOP_DAEMON_URL/api/loops/$EDIT_PID/snapshot" | jq -r '[.graph.nodes[].id] | join(",")')"
    assert_eq "$ids" "plan,1,2,3,4" "snapshot keeps the prior review inline and adds a terminal review"

    echo "loop-plan: milestones — parked gate, current projections, and plan metadata"
    MILE="$TMP/milestones/.loop"; mkdir -p "$MILE"
    cat > "$MILE/plan.md" <<'EOF'
<!-- loop-plan
planId: loop-milestones-e2e
daemon: http://localhost:7717
-->
# Releases — Multi-Phase Plan
## Repositories
### `acme/api`
- **PR:** https://github.com/acme/api/pull/99
## Milestone 1 — First release
- **Integration branch:** `feat/first`
- **PR:** https://github.com/acme/api/pull/1
### Phase 1 — First work `[lane: A]` `[status: done]`
- **Repository:** `acme/api`
### Phase 2 — First gate `[lane: review]` `[status: done]` `[kind: pr-review]` `[gate: merge]`
- **Depends on:** Phase 1
## Milestone 2 — Second release
- **Integration branch:** `feat/second`
### Phase 3 — Second work `[lane: A]` `[status: todo]`
- **Repository:** `acme/api`
- **Depends on:** Phase 2
### Phase 4 — Second review `[lane: review]` `[status: todo]` `[kind: pr-review]`
- **Depends on:** Phase 3
EOF
    MILE_PID="$(plan register --dir "$MILE")"
    curl -sf -X POST --data-binary '{"milestone":1,"awaitingMerge":{"milestone":1,"prs":{"acme/api":"https://github.com/acme/api/pull/1"}},"repositories":{"acme/api":{"prUrl":"https://github.com/acme/api/pull/1"}}}' \
      "$LOOP_DAEMON_URL/api/loops/$MILE_PID/state" >/dev/null
    milestone_plan="$(curl -sf "$LOOP_DAEMON_URL/api/loops/$MILE_PID/plan")"
    assert_eq "$(print -r -- "$milestone_plan" | jq -r '[.status,.milestone,.milestoneTotal,.awaitingMerge.prs["acme/api"]] | map(tostring) | join(" ")')" \
      "awaiting-merge 1 2 https://github.com/acme/api/pull/1" "plan endpoint exposes milestone gate coordinates"

    # A newer finished loop must not hide a parked gate in back-compat snapshots.
    FIN="$TMP/new-finished/.loop"; mkdir -p "$FIN"
    curl -sf -X POST --data-binary "$(jq -cn --arg dir "$FIN" '{loopDir:$dir}')" \
      "$LOOP_DAEMON_URL/api/loops/loop-new-finished/register" >/dev/null
    curl -sf -X POST --data-binary '{}' "$LOOP_DAEMON_URL/api/loops/loop-new-finished/finish" >/dev/null
    assert_eq "$(curl -sf "$LOOP_DAEMON_URL/api/snapshot" | jq -r '.effort.name')" \
      "Releases" "awaiting-merge counts as active-ish for default selection"

    ARC="$TMP/archived-active/.loop"; mkdir -p "$ARC"
    curl -sf -X POST --data-binary "$(jq -cn --arg dir "$ARC" '{loopDir:$dir}')" \
      "$LOOP_DAEMON_URL/api/loops/loop-archived-active/register" >/dev/null
    curl -sf -X POST --data-binary '{"phases":{"1":{"status":"running"}}}' \
      "$LOOP_DAEMON_URL/api/loops/loop-archived-active/state" >/dev/null
    curl -sf -X POST --data-binary '{}' "$LOOP_DAEMON_URL/api/loops/loop-archived-active/archive" >/dev/null
    assert_eq "$(curl -sf "$LOOP_DAEMON_URL/api/snapshot" | jq -r '.effort.name')" \
      "Releases" "manually archived loops do not hide a live milestone gate"

    curl -sf -X POST --data-binary '{"milestone":2,"awaitingMerge":null,"milestones":{"1":{"prs":{"acme/api":"https://github.com/acme/api/pull/1"}}},"repositories":{"acme/api":{"integrationBranch":"feat/second","prUrl":null,"review":null}}}' \
      "$LOOP_DAEMON_URL/api/loops/$MILE_PID/state" >/dev/null
    milestone_snap="$(curl -sf "$LOOP_DAEMON_URL/api/loops/$MILE_PID/snapshot")"
    assert_eq "$(print -r -- "$milestone_snap" | jq -r '[.effort.milestone,.effort.milestoneTotal,.effort.pr,.plan.repositories[0].pr] | map(tostring) | join(" ")')" \
      "2 2 null null" "HTTP snapshot never falls back to the prior milestone or repository PR"
    assert_eq "$(print -r -- "$milestone_snap" | jq -r '.graph.nodes[] | select(.id=="2") | .prs["acme/api"]')" \
      "https://github.com/acme/api/pull/1" "historical gate retains its recorded PR through HTTP"

    echo "loop-plan: archive end-to-end — the daemon reads stub phases as existing done phases"
    ARCH="$(fresh_loop archive-e2e)"
    ARCH_PID="$(plan register --dir "$ARCH")"
    curl -sf -X POST --data-binary @"$ARCH/state.json" "$LOOP_DAEMON_URL/api/loops/$ARCH_PID/state" >/dev/null
    PATH="$TMP/bin:$PATH" FAKE_PR_STATES="$API_PR=MERGED
$WEB_PR=MERGED" plan archive --milestone 1 --dir "$ARCH" >/dev/null
    assert_eq "$?" "0" "archive pushes to the daemon"
    arch_plan="$(curl -sf "$LOOP_DAEMON_URL/api/loops/$ARCH_PID/plan" | jq -r '.planText')"
    assert_contains "$arch_plan" '## Milestone 1 — Core API and web shell ship `[archived]`' "the daemon holds the stubbed plan"
    arch_snap="$(curl -sf "$LOOP_DAEMON_URL/api/loops/$ARCH_PID/snapshot")"
    assert_eq "$(print -r -- "$arch_snap" | jq -r '[.graph.nodes[] | select(.milestone==1) | "\(.id):\(.status)"] | join(" ")')" \
      "1:done 2:done 9:done 3:done" "archived phases stay in the graph as done"
    assert_eq "$(print -r -- "$arch_snap" | jq -r '.graph.edges[] | select(.id=="3->4") | .blocking')" \
      "false" "the dependency on the archived gate is satisfied"
    assert_eq "$(print -r -- "$arch_snap" | jq -r '[.graph.milestones[] | .archived // false] | map(tostring) | join(" ")')" \
      "true false false" "milestone 1 is marked archived"
    assert_eq "$(print -r -- "$arch_snap" | jq -r '.warnings | type, length' | paste -sd' ' -)" "array 0" "the stubbed plan parses without warnings"

    echo "loop-plan: control — pause/resume (loop + phase) and model override via POST /control"
    CTL="$TMP/control/.loop"; mkdir -p "$CTL"
    cat > "$CTL/plan.md" <<'EOF'
<!-- loop-plan
planId: loop-control-e2e
daemon: http://localhost:7717
-->
# Control — Multi-Phase Plan
## Phases
### Phase 1 — Started work `[lane: A]` `[status: in-progress]`
- **Depends on:** none
### Phase 2 — Later work `[lane: A]` `[status: todo]`
- **Depends on:** Phase 1
EOF
    CTL_PID="$(plan register --dir "$CTL")"
    curl -sf -X POST --data-binary '{"phases":{"1":{"slug":"p1-started","status":"running","attempt":1},"2":{"slug":"p2-later","status":"todo"}}}' \
      "$LOOP_DAEMON_URL/api/loops/$CTL_PID/state" >/dev/null
    ctl() { curl -s -o "$TMP/ctl.out" -w '%{http_code}' -X POST --data-binary "$2" "$LOOP_DAEMON_URL/api/loops/$1/control"; }
    snap() { curl -sf "$LOOP_DAEMON_URL/api/loops/$CTL_PID/snapshot" | jq -r "$1"; }

    echo "loop-plan: HIL answer clears the phase pause in the daemon snapshot"
    jq -cn --arg runId "$CTL_PID" '{runId:$runId,phases:{"1":{slug:"p1-started",status:"running",attempt:1},"2":{slug:"p2-later",status:"todo"}}}' > "$CTL/state.json"
    mkdir -p "$CTL/hil"
    print -r -- 'Choose the next step.' > "$CTL/hil/p1-started.md"
    wait_hil() {
      local expected="$1" deadline=$(( SECONDS + 30 ))
      until [[ "$(snap '.graph.nodes[] | select(.id=="1") | .runtime.hilOpen')" == "$expected" ]]; do
        (( SECONDS < deadline )) || return 1
        sleep 0.05
      done
    }
    zsh "$ROOT/loop-state.sh" log --dir "$CTL" hil.raise 1 'Choose the next step.'
    wait_hil true; assert_exit "$?" "0" "the daemon reads the HIL brief"
    assert_eq "$(curl -sf "$LOOP_DAEMON_URL/api/loops" | jq -r --arg id "$CTL_PID" '.[] | select(.runId==$id) | .status')" "paused" "hil.raise pauses the loop"
    assert_eq "$(snap '.graph.nodes[] | select(.id=="1") | "\(.runtime.hilOpen) \(.ui)"')" "true awaiting" "the phase awaits a HIL answer"
    TOP="$(LOOP_STORE_DIR="$STORE" node "$ROOT/loop-top.mjs" "$CTL_PID" --once)"
    assert_contains "$TOP" "waiting on you (orchestrator)" "loop-top displays the unanswered HIL"
    zsh "$ROOT/loop-state.sh" hil-resolve --dir "$CTL" 1 p1-started 'Use option B.'
    wait_hil false; assert_exit "$?" "0" "the daemon reads the HIL answer"
    assert_eq "$(cat "$CTL/hil/p1-started.answer.md")" "Use option B." "the HIL answer persists"
    assert_eq "$(curl -sf "$LOOP_DAEMON_URL/api/loops" | jq -r --arg id "$CTL_PID" '.[] | select(.runId==$id) | .status')" "active" "hil-resolve clears the daemon pause"
    assert_eq "$(snap '.graph.nodes[] | select(.id=="1") | "\(.runtime.hilOpen) \(.ui)"')" "false running" "hil-resolve clears the pause in the loop-top snapshot"
    TOP="$(LOOP_STORE_DIR="$STORE" node "$ROOT/loop-top.mjs" "$CTL_PID" --once)"
    assert_eq "$([[ "$TOP" == *'waiting on you (orchestrator)'* ]] && echo yes || echo no)" "no" "loop-top clears the answered HIL"

    assert_eq "$(ctl "$CTL_PID" '{"action":"pause"}')" "200" "pause loop → 200"
    assert_eq "$([[ -f "$CTL/control/pause" ]] && echo yes)" "yes" "pause loop writes control/pause"
    assert_eq "$(snap '.paused')" "true" "snapshot.paused true while control/pause exists"
    assert_eq "$(snap '[.events[] | select(.event=="control.pause")] | length')" "1" "control.pause lands on the timeline"
    assert_eq "$(ctl "$CTL_PID" '{"action":"resume"}')" "200" "resume loop → 200"
    assert_eq "$([[ -e "$CTL/control/pause" ]] && echo yes || echo no)" "no" "resume loop removes control/pause"
    assert_eq "$(snap '.paused')" "false" "snapshot.paused false after resume"

    assert_eq "$(ctl "$CTL_PID" '{"action":"pause","phase":"2"}')" "200" "pause phase → 200"
    assert_eq "$([[ -f "$CTL/control/pause-2" ]] && echo yes)" "yes" "pause phase writes control/pause-2"
    assert_eq "$(snap '.graph.nodes[] | select(.id=="2") | "\(.paused) \(.ui)"')" "true paused" "paused phase node is paused/ui paused"
    assert_eq "$(ctl "$CTL_PID" '{"action":"resume","phase":"2"}')" "200" "resume phase → 200"
    assert_eq "$(snap '.graph.nodes[] | select(.id=="2") | .paused')" "false" "resumed phase node is unpaused"

    assert_eq "$(ctl "$CTL_PID" '{"action":"model","phase":"2","leg":"claude:claude-opus-5-5+sonnet"}')" "200" "model override on a todo phase → 200"
    assert_eq "$(cat "$CTL/control/model-2")" "claude:claude-opus-5-5+sonnet" "model override written to control/model-2"
    assert_eq "$(snap '.graph.nodes[] | select(.id=="2") | .modelOverride')" "claude:claude-opus-5-5+sonnet" "snapshot carries modelOverride"
    assert_eq "$(ctl "$CTL_PID" '{"action":"model","phase":"2","leg":"codex:@sol"}')" "200" "a latest-of-family leg (codex:@sol) is accepted"
    assert_eq "$(ctl "$CTL_PID" '{"action":"model","phase":"2","leg":"codex:@@sol"}')" "400" "a malformed family leg → 400"
    assert_eq "$(ctl "$CTL_PID" '{"action":"model","phase":"2","leg":null}')" "200" "clearing the model override → 200"
    assert_eq "$([[ -e "$CTL/control/model-2" ]] && echo yes || echo no)" "no" "clear removes control/model-2"
    assert_eq "$(ctl "$CTL_PID" '{"action":"model","phase":"1","leg":"codex:gpt-6-sol"}')" "409" "model override on a started phase → 409"

    assert_eq "$(ctl "$CTL_PID" '{"action":"model","phase":"2","leg":"gpt:foo"}')" "400" "unknown engine → 400"
    assert_eq "$(ctl "$CTL_PID" '{"action":"model","phase":"2","leg":"codex:bad model"}')" "400" "bad model charset → 400"
    assert_eq "$(ctl "$CTL_PID" '{"action":"explode"}')" "400" "unknown action → 400"
    assert_eq "$(ctl "$CTL_PID" '{"action":"pause","phase":"../x"}')" "400" "unsafe phase key → 400"
    assert_eq "$(ctl "loop-nope" '{"action":"pause"}')" "404" "unknown loop → 404"
    rm -rf "$CTL"
    assert_eq "$(ctl "$CTL_PID" '{"action":"pause"}')" "410" "archived loop (dir gone) → 410"
  fi
fi

test_summary
