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
