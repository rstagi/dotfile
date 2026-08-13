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
cleanup() { [[ -n "$DAEMON_PID" ]] && kill "$DAEMON_PID" 2>/dev/null; rm -rf "$TMP" "$STORE"; }
trap cleanup EXIT

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
  fi
fi

test_summary
