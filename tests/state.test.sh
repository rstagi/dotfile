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
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
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

echo "loop-state: HIL answer writes the file and resolves the phase"
zsh "$SCRIPT" log --dir "$LOOP" hil.raise 72 'Choose the next step.'
ANSWER=$'Use option B. Keep "quotes" and $variables.\nRun the verification.'
OUT="$(zsh "$SCRIPT" hil-resolve --dir "$LOOP" 72 p72-loop-hil-resolve "$ANSWER" 2>&1)"
assert_exit "$?" "0" "hil-resolve succeeds"
assert_eq "$(cat "$LOOP/hil/p72-loop-hil-resolve.answer.md" 2>/dev/null)" "$ANSWER" "answer text persists verbatim"
assert_eq "$(jq -sr 'map(.event) | join(" ")' "$LOOP/events.jsonl")" "hil.raise hil.resolve" "the answer resolves the raised HIL"
assert_eq "$(jq -r 'select(.event == "hil.resolve") | .phase' "$LOOP/events.jsonl")" "72" "the resolve event identifies the phase"
assert_eq "$(jq -r 'select(.event == "hil.resolve") | .detail' "$LOOP/events.jsonl")" "$ANSWER" "the resolve event carries the answer"

EVENTS_BEFORE="$(cat "$LOOP/events.jsonl")"
for bad_args in phase slug answer; do
  case "$bad_args" in
    phase) args=(zero p72-safe text) ;;
    slug) args=(72 ../outside text) ;;
    answer) args=(72 p72-safe '') ;;
  esac
  zsh "$SCRIPT" hil-resolve --dir "$LOOP" "${args[@]}" >/dev/null 2>&1
  assert_exit "$?" "1" "hil-resolve rejects invalid $bad_args"
done
assert_eq "$(cat "$LOOP/events.jsonl")" "$EVENTS_BEFORE" "invalid input does not resolve a HIL"
assert_eq "$([[ -e "$LOOP/outside.answer.md" ]] && echo yes || echo no)" "no" "the slug cannot escape the HIL directory"

mkdir "$LOOP/hil/p72-unwritable.answer.md"
zsh "$SCRIPT" hil-resolve --dir "$LOOP" 72 p72-unwritable text >/dev/null 2>&1
assert_exit "$?" "1" "hil-resolve fails when the answer path is a directory"
assert_eq "$(cat "$LOOP/events.jsonl")" "$EVENTS_BEFORE" "a failed answer write does not resolve a HIL"

test_summary
