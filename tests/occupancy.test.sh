#!/bin/zsh
# Phase 1 — loop-state.sh occupancy: parse a stream-json transcript, sum the last
# usage-bearing assistant event, flag recycle at LOOP_ORCH_RECYCLE_TOKENS.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
FIX="$HERE/fixtures/occupancy"

# No state.json in cwd → occupancy emits nothing; keep the daemon out of tests regardless.
export LOOP_DAEMON_URL="http://127.0.0.1:9"

# run <fixture> [extra args...] → sets OUT (stdout) and RC (exit code)
occ() {
  local fx="$1"; shift
  OUT="$(zsh "$ROOT/loop-state.sh" occupancy --transcript "$fx" "$@" 2>/dev/null)"
  RC=$?
}

echo "occupancy: token sum = last usage-bearing assistant event"
occ "$FIX/small.jsonl"
assert_eq "$OUT" "5000" "small: 100+4000+900"
assert_exit "$RC" "0" "small: under threshold"

occ "$FIX/multi.jsonl"
assert_eq "$OUT" "8000" "multi: picks LAST assistant (1000+6500+500), ignores earlier + result.usage"
assert_exit "$RC" "0" "multi: under threshold"

echo "occupancy: recycle flag at/over LOOP_ORCH_RECYCLE_TOKENS (150000)"
occ "$FIX/over.jsonl"
assert_eq "$OUT" "151200" "over: 200+151000+0"
assert_exit "$RC" "10" "over: >= threshold → exit 10 (recycle)"

echo "occupancy: --window adds percent"
occ "$FIX/over.jsonl" --window 200000
assert_eq "$OUT" "151200 75" "over --window: tokens + percent (151200/200000)"
assert_exit "$RC" "10" "over --window: still recycle"

echo "occupancy: tolerance"
occ "$FIX/malformed-last.jsonl"
assert_eq "$OUT" "12000" "malformed/partial last line skipped → previous complete event"
assert_exit "$RC" "0" "malformed: under threshold"

occ "$FIX/no-usage.jsonl"
assert_eq "$OUT" "0" "no assistant usage → 0"
assert_exit "$RC" "0" "no-usage: exit 0"

occ "$FIX/does-not-exist.jsonl"
assert_eq "$OUT" "0" "missing transcript → 0 (not an error)"
assert_exit "$RC" "0" "missing transcript: exit 0"

echo "occupancy: usage-bearing guard — a trailing assistant without/empty usage falls back"
occ "$FIX/trailing-no-usage.jsonl"
assert_eq "$OUT" "120000" "last assistant lacks a usage key → previous usage-bearing event (120000)"
occ "$FIX/empty-usage.jsonl"
assert_eq "$OUT" "8000" "last assistant has empty usage {} → previous usage-bearing event (8000)"

echo "occupancy: threshold boundary + integer/non-integer env override"
LOOP_ORCH_RECYCLE_TOKENS=5000 occ "$FIX/small.jsonl"
assert_exit "$RC" "10" "tokens == threshold (5000) → exit 10 (>= boundary, not >)"
LOOP_ORCH_RECYCLE_TOKENS=5001 occ "$FIX/small.jsonl"
assert_exit "$RC" "0" "tokens just under threshold (5001) → exit 0"
LOOP_ORCH_RECYCLE_TOKENS=20k occ "$FIX/small.jsonl"
assert_exit "$RC" "0" "non-integer threshold (20k) → safe default, never dies with exit 1"
assert_eq "$OUT" "5000" "non-integer threshold still prints the token total"

test_summary
