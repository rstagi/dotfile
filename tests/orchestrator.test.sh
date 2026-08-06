#!/bin/zsh
# Phase 2 — loop-orchestrator.sh: sequential SUB respawner. Drives it with a fake engine
# whose per-instance outcome is scripted via FAKE_OUTCOMES.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
FAKE="$HERE/fake"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
DIR="$TMP/.kestral/loop"
SUB="$DIR/sub"
mkdir -p "$SUB"

export ENGINE_CMD="$FAKE/fake-engine.sh"
export LOOP_EMIT_SH="$FAKE/loop-emit.sh"
export LOOP_NOTIFY_SH="$FAKE/loop-notify.sh"
export LOOP_ORCH_POLL_SEC=1
export LOOP_ORCH_STALL_SEC=2
export EMIT_LOG="$TMP/emit.log"
export NOTIFY_LOG="$TMP/notify.log"

reset() { rm -rf "$SUB"; mkdir -p "$SUB"; : > "$EMIT_LOG"; : > "$NOTIFY_LOG"; }
orch() { zsh "$ROOT/loop-orchestrator.sh" --dir "$DIR" --run-id "test-run" > "$TMP/orch.out" 2>&1; RC=$?; }
count_transcripts() { ls "$SUB"/transcript-*.jsonl 2>/dev/null | wc -l | tr -d ' '; }

echo "orchestrator: recycle → respawn → complete"
reset
FAKE_OUTCOMES="recycle complete" orch
assert_exit "$RC" "0" "exits 0 on complete"
assert_eq "$([[ -f "$SUB/PHASES_DONE" ]] && echo yes)" "yes" "touches PHASES_DONE on complete"
assert_eq "$(count_transcripts)" "2" "spawned two instances (one recycle, one complete)"
assert_contains "$(cat "$EMIT_LOG")" "sub.recycle" "emitted a sub.recycle event"
assert_contains "$(cat "$EMIT_LOG")" '"recycleIndex":1' "sub.recycle carries recycleIndex 1"
assert_contains "$(cat "$EMIT_LOG")" '"tokens":151000' "sub.recycle carries tokens from status.json"

echo "orchestrator: crash → respawn, capped at LOOP_ORCH_MAX_RESPAWN"
reset
LOOP_ORCH_MAX_RESPAWN=2 FAKE_OUTCOMES="crash crash crash" orch
assert_exit "$RC" "1" "gives up (exit 1) after MAX_RESPAWN consecutive crashes"
assert_eq "$(count_transcripts)" "2" "spawned exactly MAX_RESPAWN=2 instances"
assert_contains "$(cat "$NOTIFY_LOG")" "error" "notified on give-up"
assert_eq "$([[ -f "$SUB/PHASES_DONE" ]] && echo yes || echo no)" "no" "no PHASES_DONE on crash-cap"

echo "orchestrator: a transient crash then complete still finishes (cap not reached)"
reset
LOOP_ORCH_MAX_RESPAWN=2 FAKE_OUTCOMES="crash complete" orch
assert_exit "$RC" "0" "one crash then complete → exit 0 (cap not reached)"
assert_eq "$([[ -f "$SUB/PHASES_DONE" ]] && echo yes)" "yes" "completes after a transient crash"

echo "orchestrator: a clean recycle RESETS the consecutive-crash streak"
reset
LOOP_ORCH_MAX_RESPAWN=2 FAKE_OUTCOMES="crash recycle crash complete" orch
assert_exit "$RC" "0" "crash→recycle(reset)→crash→complete does NOT hit the cap of 2"
assert_eq "$([[ -f "$SUB/PHASES_DONE" ]] && echo yes)" "yes" "completes because recycle reset the crash streak"

echo "orchestrator: an unknown/drifted status.json outcome is CAPPED (no infinite respawn)"
reset
LOOP_ORCH_MAX_RESPAWN=2 FAKE_OUTCOMES="weird weird weird" orch
assert_exit "$RC" "1" "unknown outcome respawns then gives up at the cap (regression: reset was misplaced)"
assert_eq "$(count_transcripts)" "2" "spawned exactly MAX_RESPAWN=2 instances on a persistent unknown outcome"
assert_contains "$(cat "$NOTIFY_LOG")" "error" "notified on unknown-outcome give-up"

echo "orchestrator: fatal → exit 2 + notify"
reset
FAKE_OUTCOMES="fatal" orch
assert_exit "$RC" "2" "outcome fatal → exit 2"
assert_contains "$(cat "$NOTIFY_LOG")" "error" "notified on fatal"

echo "orchestrator: recycleIndex falls back to the local counter when the SUB omits it"
reset
FAKE_OUTCOMES="recycle-noidx complete" orch
assert_exit "$RC" "0" "recycle without recycleIndex still respawns then completes"
assert_contains "$(cat "$EMIT_LOG")" "sub.recycle" "still emits sub.recycle"
assert_contains "$(cat "$EMIT_LOG")" '"recycleIndex":1' "recycleIndex filled from the local counter (1)"

echo "orchestrator: stall watchdog kills a hung SUB (transcript mtime stale)"
reset
start=$(date +%s)
LOOP_ORCH_MAX_RESPAWN=1 FAKE_OUTCOMES="hang" orch
elapsed=$(( $(date +%s) - start ))
assert_exit "$RC" "1" "hung SUB → stall-kill → crash → give up at cap 1"
assert_eq "$([[ $elapsed -lt 15 ]] && echo fast)" "fast" "killed at ~STALL_SEC, not the 20s self-exit (elapsed=${elapsed}s)"

test_summary
