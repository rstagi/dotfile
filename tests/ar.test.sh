#!/bin/zsh
# AR public contract: report parsing, rotation, and replay of completed ledger rows.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
source "$ROOT/loop-ar.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
REPORT="$TMP/report.md"
LEDGER="$TMP/review.ledger.tsv"
T=$'\t'

echo "ar: last non-empty verdict line"
printf 'F1: example\nNEEDS_CHANGES: broken replay\n\n \t\n' > "$REPORT"
assert_eq "$(ar_parse_verdict "$REPORT" reviewer)" NEEDS_CHANGES "reviewer summary becomes token"
printf 'NEEDS_CHANGES: old finding\nAPPROVE\n' > "$REPORT"
assert_eq "$(ar_parse_verdict "$REPORT" reviewer)" APPROVE "only last verdict counts"
for verdict in AGREE 'DISAGREE_EVIDENCE: code refutes F1' 'DISAGREE_CONCERN: missed F2'; do
  print -r -- "$verdict" > "$REPORT"
  assert_eq "$(ar_parse_verdict "$REPORT" critic)" "${verdict%%:*}" "critic token: $verdict"
done
for verdict in 'APPROVE followed by prose' 'NEEDS_CHANGES:' 'DISAGREE_EVIDENCE: ' 'AGREE'; do
  print -r -- "$verdict" > "$REPORT"
  actual="$(ar_parse_verdict "$REPORT" reviewer)"; rc=$?
  assert_eq "$actual" invalid "malformed reviewer verdict: $verdict"
  assert_exit "$rc" 2 "invalid verdict exits 2"
done
printf 'APPROVE\ntrailing prose\n' > "$REPORT"
actual="$(ar_parse_verdict "$REPORT" reviewer)"; rc=$?
assert_eq "$actual" invalid "earlier valid line cannot hide trailing prose"
assert_exit "$rc" 2 "trailing prose exits 2"

echo "ar: provider rotation"
assert_eq "$(ar_rotation 1)" "gpt${T}claude${T}grok" "round 1 R/C/T"
assert_eq "$(ar_rotation 2)" "claude${T}grok${T}gpt" "round 2 R/C/T"
assert_eq "$(ar_rotation 3)" "grok${T}gpt${T}claude" "round 3 R/C/T"
assert_eq "$(ar_rotation 4)" "gpt${T}claude${T}grok" "rotation wraps"
ar_rotation 0 > /dev/null 2>&1; assert_exit "$?" 1 "round zero rejected"

test_summary
