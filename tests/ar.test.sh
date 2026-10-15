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

echo "ar: clean opening review requires third-provider confirmation"
record() {
  local count=0
  [[ -f "$LEDGER" ]] && count=$(wc -l < "$LEDGER")
  printf 'a%d\t%s\t%s\t%s\t%s\truns/a%d\n' "$(( count + 1 ))" "$1" "$2" "$3" "$4" "$(( count + 1 ))" >> "$LEDGER"
}
next() { ar_next --rounds 3 --inner 2 --ledger "$LEDGER" "$@"; }
assert_eq "$(next)" "round1${T}review${T}gpt" "missing ledger starts review"
record round1 review gpt APPROVE
assert_eq "$(next)" "round1${T}critic${T}claude" "opening review goes to critic"
record round1 critic claude AGREE
assert_eq "$(next)" "round1${T}confirm${T}grok" "clean pair requires T"
record round1 confirm grok AGREE
assert_eq "$(next)" "final${T}final${T}claude" "three clean runs skip later rounds"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" clean "confirmed opening is clean"
record final final claude APPROVE
assert_eq "$(next)" done "posted final ends pipeline"

echo "ar: findings, fix completion, and three outer rounds"
rm -f "$LEDGER"
record round1 review gpt NEEDS_CHANGES
record round1 critic claude AGREE
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" findings "converged flags are findings"
assert_eq "$(next)" "fix1${T}fix${T}claude" "findings before last round require fix"
assert_eq "$(next --rounds 1)" "final${T}final${T}claude" "one outer round goes directly to final"
record fix1 fix claude NOOP
assert_eq "$(next)" "round2${T}review${T}claude" "NOOP fix advances to round 2"
record round2 review claude NEEDS_CHANGES
assert_eq "$(next)" "round2${T}critic${T}grok" "round 2 critic rotates"
record round2 critic grok AGREE
assert_eq "$(next)" "fix2${T}fix${T}claude" "round 2 findings require fix"
record fix2 fix claude PUSHED
assert_eq "$(next)" "round3${T}review${T}grok" "pushed fix advances to round 3"
record round3 review grok NEEDS_CHANGES
assert_eq "$(next)" "round3${T}critic${T}gpt" "round 3 critic rotates"
record round3 critic gpt AGREE
assert_eq "$(next)" "final${T}final${T}claude" "last round omits fix"
record final final claude REQUEST_CHANGES
assert_eq "$(next)" done "request-changes final also completes"

echo "ar: disagreement converges after one response; replay is deterministic"
rm -f "$LEDGER"
record round1 review gpt NEEDS_CHANGES
record round1 critic claude DISAGREE_EVIDENCE
assert_eq "$(next)" "round1${T}respond${T}gpt" "disagreement gets a reviewer response"
cp "$LEDGER" "$TMP/replay.tsv"
assert_eq "$(ar_next --rounds 3 --inner 2 --ledger "$TMP/replay.tsv")" \
  "round1${T}respond${T}gpt" "fresh ledger replay yields same step"
assert_eq "$(next)" "round1${T}respond${T}gpt" "asking for next does not mutate ledger"
record round1 respond gpt NEEDS_CHANGES
assert_eq "$(next)" "round1${T}critic${T}claude" "response returns to critic"
record round1 critic claude AGREE
assert_eq "$(next)" "fix1${T}fix${T}claude" "agreement after response converges"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" findings "remaining flags need fixes"

echo "ar: third-provider veto takes over the critic slot"
rm -f "$LEDGER"
record round1 review gpt APPROVE
record round1 critic claude AGREE
record round1 confirm grok DISAGREE_CONCERN
assert_eq "$(next)" "round1${T}respond${T}gpt" "T veto requests response"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" findings "vetoed round is no longer clean"
record round1 respond gpt APPROVE
assert_eq "$(next)" "round1${T}critic${T}grok" "T becomes critic after veto"
record round1 critic grok AGREE
assert_eq "$(next)" "round2${T}review${T}claude" "veto prevents early stop even after convergence"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" findings "veto is retained in round status"

echo "ar: inner cap preserves disputes, including critic-only issues"
for reviewer_verdict in NEEDS_CHANGES APPROVE; do
  rm -f "$LEDGER"
  record round1 review gpt "$reviewer_verdict"
  record round1 critic claude DISAGREE_CONCERN
  assert_eq "$(next --inner 1)" "disputed${T}disputed${T}-" "cap 1 stops without response: $reviewer_verdict"
  assert_eq "$(next)" "round1${T}respond${T}gpt" "cap 2 permits exactly one response"
  record round1 respond gpt "$reviewer_verdict"
  record round1 critic claude DISAGREE_EVIDENCE
  assert_eq "$(next)" "disputed${T}disputed${T}-" "second disagreement exhausts cap"
  record disputed disputed - DISPUTED
  print -r -- 'F2: critic-only unresolved issue' > "${LEDGER%.tsv}.disputed.md"
  assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" disputed "dispute outranks reviewer verdict"
  assert_eq "$(next)" "fix1${T}fix${T}claude" "disputed round requires fix: $reviewer_verdict"
  assert_eq "$(next --rounds 1)" "final${T}final${T}claude" "last-round dispute goes to final"
  assert_eq "$(next --single-round)" done "plan dispute ends without fix/final"
  record fix1 fix claude PUSHED
  assert_eq "$(next)" "round2${T}review${T}claude" "disputed marker is complete after fix"
  record round2 review claude APPROVE
  record round2 critic grok AGREE
  record round2 confirm gpt AGREE
  assert_eq "$(ar_round_status --ledger "$LEDGER" --round 2)" clean "earlier dispute does not taint next round"
  assert_eq "$(next)" "final${T}final${T}claude" "later clean confirmation still stops early"
done
rm -f "$LEDGER" "${LEDGER%.tsv}.disputed.md"
record round1 review gpt APPROVE
record round1 critic claude AGREE
record round1 confirm grok DISAGREE_EVIDENCE
assert_eq "$(next --inner 1)" "disputed${T}disputed${T}-" "T veto shares the opening inner count"

echo "ar: independent plan-review round starts at round 2"
rm -f "$LEDGER"
assert_eq "$(next --start-round 2 --single-round)" "round2${T}review${T}claude" "empty ledger starts specified round"
record round2 review claude APPROVE
assert_eq "$(next --start-round 2 --single-round)" "round2${T}critic${T}grok" "round 2 uses Grok critic"
record round2 critic grok AGREE
assert_eq "$(next --start-round 2 --single-round)" "round2${T}confirm${T}gpt" "round 2 uses GPT confirmation"
record round2 confirm gpt AGREE
assert_eq "$(next --start-round 2 --single-round)" done "single-round clean has no final"
rm -f "$LEDGER"
record round2 review claude NEEDS_CHANGES
record round2 critic grok AGREE
assert_eq "$(next --start-round 2 --single-round)" done "single-round findings have no fix or final"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 2)" findings "planner receives findings status"

echo "ar: revised approval skips fixes but cannot trigger opening early stop"
rm -f "$LEDGER"
record round1 review gpt NEEDS_CHANGES
record round1 critic claude DISAGREE_EVIDENCE
record round1 respond gpt APPROVE
record round1 critic claude AGREE
assert_eq "$(next)" "round2${T}review${T}claude" "last R approval omits fix, without third confirmation"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" clean "withdrawn findings no longer count"
assert_eq "$(next --rounds 1)" "final${T}final${T}claude" "revised approval in last round goes to final"
record fix1 fix claude NOOP
assert_eq "$(next --single-round)" done "single-round cannot advance after a completed fix"

echo "ar: configuration and ledger validation"
for args in '--rounds 0' '--rounds 6' '--inner 0' '--inner 6' '--inner nope' '--start-round 4' '--unknown'; do
  next ${=args} >/dev/null 2>&1
  assert_exit "$?" 1 "invalid arguments: $args"
done
ar_next --rounds 2 --inner 2 --ledger >/dev/null 2>&1
assert_exit "$?" 1 "missing ledger value rejected"
rm -f "$LEDGER"
record round1 review gpt UNKNOWN
next >/dev/null 2>&1; assert_exit "$?" 1 "unknown ledger verdict rejected"
ar_round_status --ledger "$LEDGER" --round 1 >/dev/null 2>&1
assert_exit "$?" 1 "status also rejects unknown verdict"
rm -f "$LEDGER"
record round1 fix claude NOOP
next >/dev/null 2>&1; assert_exit "$?" 1 "mismatched ledger role/stage rejected"
printf 'a1\tround1\treview\tgpt\tAPPROVE\n' > "$LEDGER"
next >/dev/null 2>&1; assert_exit "$?" 1 "ledger must have all six columns"

test_summary
