#!/bin/zsh
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
source "$ROOT/loop-emit.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
CAPTURE="$TMP/event.json"

loop_emit() {
  assert_eq "$1" "loop-jev" "decision emits to the correlated run"
  assert_eq "$2" "event" "decision uses the event endpoint"
  cat > "$CAPTURE"
}

printf '%s' '{"phase":"2","attempt":1,"stage":"question","mode":"shadow","candidate":"plan-answer","appliedAction":"current-behavior","evidenceChecked":true,"evidenceSources":["question","plan"],"questionRound":2}' \
  | loop_emit_jev_decision "loop-jev"

assert_eq "$(jq -r '.event' "$CAPTURE")" "jev.decision" "adds the typed event name"
assert_eq "$(jq -r '.runId' "$CAPTURE")" "loop-jev" "adds run correlation"
assert_eq "$(jq -r '.phase + ":" + (.attempt|tostring) + ":" + .candidate' "$CAPTURE")" "2:1:plan-answer" "preserves decision fields"
assert_eq "$(jq -r '(.evidenceChecked|tostring) + ":" + (.evidenceSources|join(",")) + ":" + (.questionRound|tostring)' "$CAPTURE")" \
  "true:question,plan:2" "preserves typed question disposition fields"

test_summary
