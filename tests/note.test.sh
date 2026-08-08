#!/bin/zsh
# Steering-note CLI: write/list/clear notes and reject unsafe or completed targets.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

export LOOP_DAEMON_URL="http://127.0.0.1:9"
TMP="$(mktemp -d)"
LOOP="$TMP/loop"
trap 'rm -rf "$TMP"' EXIT

run_note() {
  OUT="$(zsh "$ROOT/loop-state.sh" note --dir "$LOOP" "$@" 2>&1)"
  RC=$?
}

echo "note: init + write via argument"
zsh "$ROOT/loop-state.sh" init --dir "$LOOP" --json '{"phases":{"2":{"status":"todo"}}}' >/dev/null
assert_eq "$(test -d "$LOOP/notes" && echo yes)" "yes" "init creates notes directory"
run_note 2 "rebase first"
assert_exit "$RC" "0" "argument write succeeds"
assert_eq "$(cat "$LOOP/notes/2.md")" "rebase first" "argument text is persisted"

echo "note: write via stdin + list first lines"
printf 'review the migration\nkeep API stable\n' | zsh "$ROOT/loop-state.sh" note --dir "$LOOP" pr-review
assert_eq "$(cat "$LOOP/notes/pr-review.md")" $'review the migration\nkeep API stable' "stdin preserves multiline text"
OUT="$(zsh "$ROOT/loop-state.sh" notes --dir "$LOOP")"
assert_contains "$OUT" $'2\trebase first' "list shows numeric note first line"
assert_contains "$OUT" $'pr-review\treview the migration' "list shows review note first line"

echo "note: clear"
run_note --clear 2
assert_exit "$RC" "0" "clear succeeds"
assert_eq "$(test ! -e "$LOOP/notes/2.md" && echo yes)" "yes" "clear removes note"

echo "note: key sanitization"
run_note ../evil "escape"
assert_exit "$RC" "1" "path traversal is refused"
assert_contains "$OUT" "invalid note key" "path traversal explains failure"
assert_eq "$(test ! -e "$LOOP/evil.md" && echo yes)" "yes" "nothing is written outside notes"
run_note . "escape"
assert_exit "$RC" "1" "dot key is refused"

echo "note: completed phase guard"
zsh "$ROOT/loop-state.sh" set --dir "$LOOP" '.phases["2"].status = "done"' >/dev/null
run_note 2 "too late"
assert_exit "$RC" "1" "done phase is refused"
assert_contains "$OUT" "already done" "done-phase failure is clear"
run_note --force 2 "late override"
assert_exit "$RC" "0" "force allows completed phase note"
assert_eq "$(cat "$LOOP/notes/2.md")" "late override" "forced note is persisted"

test_summary
