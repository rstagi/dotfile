#!/bin/zsh
# State CLI preserves state.json when a set expression is invalid or produces no object.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

SCRIPT="${LOOP_STATE_SCRIPT:-$ROOT/loop-state.sh}"
export LOOP_DAEMON_URL="http://127.0.0.1:9"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
LOOP="$TMP/loop"

zsh "$SCRIPT" init --dir "$LOOP" --json '{"phases":{}}' >/dev/null
BEFORE="$(cat "$LOOP/state.json")"

OUT="$(zsh "$SCRIPT" set --dir "$LOOP" --help 2>&1)"
assert_exit "$?" "1" "set rejects jq options"
assert_eq "$(cat "$LOOP/state.json")" "$BEFORE" "option input leaves state intact"

OUT="$(zsh "$SCRIPT" set --dir "$LOOP" 'empty' 2>&1)"
assert_exit "$?" "1" "set rejects empty jq output"
assert_eq "$(cat "$LOOP/state.json")" "$BEFORE" "empty output leaves state intact"

OUT="$(zsh "$SCRIPT" set --dir "$LOOP" '., .' 2>&1)"
assert_exit "$?" "1" "set rejects multiple JSON objects"
assert_eq "$(cat "$LOOP/state.json")" "$BEFORE" "multiple outputs leave state intact"

OUT="$(zsh "$SCRIPT" set --dir "$LOOP" '.ok = true' 2>&1)"
assert_exit "$?" "0" "set accepts one JSON object"
assert_eq "$(jq -r '.ok' "$LOOP/state.json")" "true" "valid update persists"

test_summary
