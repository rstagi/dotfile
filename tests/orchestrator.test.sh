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
DIR="$TMP/.loop"
SUB="$DIR/sub"
mkdir -p "$SUB"

export ENGINE_CMD="$FAKE/fake-engine.sh"
export LOOP_EMIT_SH="$FAKE/loop-emit.sh"
export LOOP_NOTIFY_SH="$FAKE/loop-notify.sh"
export LOOP_ORCH_POLL_SEC=1
export LOOP_ORCH_STALL_SEC=2
export EMIT_LOG="$TMP/emit.log"
export NOTIFY_LOG="$TMP/notify.log"
export FAKE_ENGINE_ARGS_LOG="$TMP/engine-args.log"

reset() { rm -rf "$SUB"; mkdir -p "$SUB"; : > "$EMIT_LOG"; : > "$NOTIFY_LOG"; : > "$FAKE_ENGINE_ARGS_LOG"; }
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
assert_contains "$(cat "$FAKE_ENGINE_ARGS_LOG")" '--model opus ' "starts the SUB on the latest Opus"
assert_contains "$(cat "$FAKE_ENGINE_ARGS_LOG")" '--effort high' "SUB runs at EFFORT_ORCHESTRATE (high)"
assert_contains "$(cat "$SUB/prompt-1.md")" '--window 1000000' "uses Opus 5.5 context for occupancy"

echo "orchestrator: EFFORT_ORCHESTRATE=ultra is clamped to claude's max"
reset
EFFORT_ORCHESTRATE=ultra FAKE_OUTCOMES="complete" orch
assert_exit "$RC" "0" "exits 0 on complete"
assert_contains "$(cat "$FAKE_ENGINE_ARGS_LOG")" '--effort max' "ultra → --effort max"

echo "orchestrator: review barrier → hand control to supervise"
reset
FAKE_OUTCOMES="review-ready" orch
assert_exit "$RC" "0" "review-ready exits cleanly"
assert_eq "$([[ -f "$SUB/REVIEW_READY" ]] && echo yes)" "yes" "touches REVIEW_READY"
assert_eq "$([[ -f "$SUB/PHASES_DONE" ]] && echo yes || echo no)" "no" "does not mark every phase done"

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

# --- loop-wide pause via .loop/control/pause (loop-top [X]) ---------------------------------
orch_bg() { zsh "$ROOT/loop-orchestrator.sh" --dir "$DIR" --run-id "test-run" > "$TMP/orch.out" 2>&1 & ORCH_PID=$!; }
orch_wait() { wait "$ORCH_PID"; RC=$?; }
wait_for() { local i=0; while (( i < 50 )); do eval "$1" && return 0; sleep 0.2; i=$((i + 1)); done; return 1; }
export LOOP_ORCH_PAUSE_POLL_SEC=1

echo "orchestrator: pause mid-SUB → SUB killed (not a crash) → waits → unpause respawns with resume prompt"
reset; rm -rf "$DIR/control"
LOOP_ORCH_STALL_SEC=60 LOOP_ORCH_MAX_RESPAWN=1 FAKE_OUTCOMES="hang complete" orch_bg
wait_for '[[ -f "$SUB/transcript-1.jsonl" ]]'
mkdir -p "$DIR/control"; touch "$DIR/control/pause"
wait_for '[[ -f "$SUB/.paused-1" ]]'
assert_eq "$([[ -f "$SUB/.paused-1" ]] && echo yes)" "yes" "watchdog marks instance 1 paused"
sleep 2
assert_eq "$(kill -0 "$ORCH_PID" 2>/dev/null && echo alive)" "alive" "orchestrator keeps waiting while paused"
assert_eq "$(count_transcripts)" "1" "no new SUB spawned while paused"
assert_contains "$(cat "$EMIT_LOG")" "loop.paused" "emits loop.paused"
rm -f "$DIR/control/pause"
orch_wait
assert_exit "$RC" "0" "unpause → respawn → complete (a paused kill does not count as a crash)"
assert_eq "$(count_transcripts)" "2" "exactly one more SUB instance after unpause"
assert_contains "$(cat "$SUB/prompt-2.md")" "instance 2" "respawned SUB gets the resume prompt"
assert_contains "$(cat "$EMIT_LOG")" "loop.resumed" "emits loop.resumed"
assert_eq "$([[ -f "$SUB/PHASES_DONE" ]] && echo yes)" "yes" "loop completes normally after resume"

echo "orchestrator: pause present before start → no SUB until cleared"
reset; mkdir -p "$DIR/control"; touch "$DIR/control/pause"
FAKE_OUTCOMES="complete" orch_bg
sleep 2
assert_eq "$(count_transcripts)" "0" "no SUB spawned while paused at start"
rm -f "$DIR/control/pause"
orch_wait
assert_exit "$RC" "0" "starts and completes once unpaused"
assert_eq "$(count_transcripts)" "1" "one SUB instance after unpause"
assert_contains "$(cat "$SUB/prompt-1.md")" "control/pause-<N>" "SUB prompt explains per-phase pause files"
assert_contains "$(cat "$SUB/prompt-1.md")" "exit 30" "SUB prompt explains runner exit 30 = paused"
rm -rf "$DIR/control"

test_summary
