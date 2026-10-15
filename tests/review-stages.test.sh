#!/bin/zsh
# PR review ledger CLI: replay, evidence recording, recovery, and prompt framing.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
T=$'\t'
mkdir -p "$TMP/loop/runs"
LEDGER="$TMP/loop/runs/review-p7-acme--api.ledger.tsv"
cli() { zsh "$ROOT/loop-review.sh" "$@" 2>"$TMP/err"; }
next() { cli next --ledger "$LEDGER" "$@"; }

echo 'review: next validates configuration and maps providers to runner chains'
assert_eq "$(next)" "a1${T}round1${T}review${T}review-gpt" 'standard defaults start with GPT reviewer'
assert_eq "$(next --tier max --rounds 5 --inner 5)" "a1${T}round1${T}review${T}review-gpt" 'max uses provider chain names'
for args in '--tier medium' '--tier shallow' '--tier huge' '--rounds 0' '--rounds 6' '--inner 0' '--inner 6' '--inner nope' '--rounds' '--ledger'; do
  next ${=args} >/dev/null
  assert_exit "$?" 1 "rejects $args"
done
cli stages >/dev/null; assert_exit "$?" 1 'static stages command removed'

echo 'review: record verdicts atomically and replay standard two-round findings'
run_dir="$TMP/loop/runs/review-p7-acme--api-a1"
mkdir -p "$run_dir"
printf 'F1 [major] broken replay\nNEEDS_CHANGES: replay loses state\n' > "$run_dir/report.md"
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage round1 --role review --provider gpt
assert_exit "$?" 0 'records completed reviewer'
assert_eq "$(cat "$LEDGER" 2>/dev/null)" "a1${T}round1${T}review${T}gpt${T}NEEDS_CHANGES${T}runs/review-p7-acme--api-a1" 'frozen six-column contract and relative run dir'
assert_eq "$(next)" "a2${T}round1${T}critic${T}review-claude" 'record advances to critic'
printf 'invalid output\n' > "$run_dir/report.md"
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage round1 --role review --provider gpt
assert_exit "$?" 0 'duplicate attempt ignores changed report'
assert_eq "$(wc -l < "$LEDGER" | tr -d ' ')" 1 'duplicate never appends'
run_dir="$TMP/loop/runs/review-p7-acme--api-a2"
mkdir -p "$run_dir"
printf 'AGREE\ntrailing commentary\n' > "$run_dir/report.md"
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage round1 --role critic --provider claude
assert_exit "$?" 2 'invalid last verdict exits 2'
assert_eq "$(wc -l < "$LEDGER" | tr -d ' ')" 1 'invalid verdict writes no row'
printf 'AGREE\n' > "$run_dir/report.md"
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage round1 --role critic --provider claude
assert_exit "$?" 0 'valid critic completes opening exchange'
assert_eq "$(next)" "a3${T}fix1${T}fix${T}review-fix" 'findings require remediation before second round'

echo 'review: fix evidence is checked against a real remote before advancing'
REPO="$TMP/repo"
REMOTE="$TMP/remote.git"
git init -q "$REPO"
git -C "$REPO" -c user.name=Test -c user.email=test@example.test commit -qm initial --allow-empty
BEFORE="$(git -C "$REPO" rev-parse HEAD)"
git clone -q --bare "$REPO" "$REMOTE"
git --git-dir="$REMOTE" update-ref refs/heads/review "$BEFORE"
run_dir="$TMP/loop/runs/review-p7-acme--api-a3"
mkdir -p "$run_dir"
fix_status() {
  jq -n --arg result "$1" --arg before "$2" --arg after "$3" --arg remote "$REMOTE" --arg wt "$REPO" \
    '{outcome:"done",result:$result,headBefore:$before,headAfter:$after,remote:$remote,branch:"review",worktree:$wt}' > "$run_dir/status.json"
}
fix_status NOOP "$BEFORE" "$BEFORE"
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage fix1 --role fix --provider claude
assert_exit "$?" 0 'verified NOOP recorded'
assert_eq "$(next)" "a4${T}round2${T}review${T}review-claude" 'fix completion rotates reviewer'
record_report() {
  local k="$1" stage="$2" role="$3" provider="$4" report="$5"
  run_dir="${LEDGER%.ledger.tsv}-a$k"
  mkdir -p "$run_dir"
  print -r -- "$report" > "$run_dir/report.md"
  cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage "$stage" --role "$role" --provider "$provider"
}
record_report 4 round2 review claude 'NEEDS_CHANGES: F1 remains'
assert_eq "$(next)" "a5${T}round2${T}critic${T}review-grok" 'round two critic is Grok'
record_report 5 round2 critic grok AGREE
assert_eq "$(next)" "a6${T}final${T}final${T}review-final" 'last round omits remediation'
run_dir="$TMP/loop/runs/review-p7-acme--api-a6"
mkdir -p "$run_dir"
printf '{"outcome":"blocked","event":"REQUEST_CHANGES","commentUrl":"https://example.test/review/1"}\n' > "$run_dir/status.json"
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage final --role final --provider claude
assert_exit "$?" 0 'posted request-changes is a completed final row'
assert_eq "$(next)" done 'final event ends pipeline'
assert_eq "$(cut -f5 "$LEDGER" | tr '\n' '|')" 'NEEDS_CHANGES|AGREE|NOOP|NEEDS_CHANGES|AGREE|REQUEST_CHANGES|' 'standard findings pipeline end to end'

echo 'review: pushed fix requires matching remote head'
rm -f "$LEDGER"
record_report 1 round1 review gpt 'NEEDS_CHANGES: F1'
record_report 2 round1 critic claude AGREE
run_dir="$TMP/loop/runs/review-p7-acme--api-a3"
git -C "$REPO" -c user.name=Test -c user.email=test@example.test commit -qm fix --allow-empty
AFTER="$(git -C "$REPO" rev-parse HEAD)"
fix_status PUSHED "$BEFORE" "$AFTER"
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage fix1 --role fix --provider claude
assert_exit "$?" 1 'unpublished head cannot record PUSHED'
assert_eq "$(wc -l < "$LEDGER" | tr -d ' ')" 2 'failed remote verification leaves ledger unchanged'
git --git-dir="$REMOTE" fetch -q "$REPO" HEAD:refs/heads/review
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage fix1 --role fix --provider claude
assert_exit "$?" 0 'published matching head records PUSHED'
assert_eq "$(tail -1 "$LEDGER" | cut -f5)" PUSHED 'PUSHED token retained'
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage fix1 --role fix --provider claude
assert_exit "$?" 0 'pushed fix replay never repeats mutation'

echo 'review: clean early stop requires all three providers'
LEDGER="$TMP/loop/runs/review-p7-clean.ledger.tsv"
record_report 1 round1 review gpt APPROVE
record_report 2 round1 critic claude AGREE
assert_eq "$(next)" "a3${T}round1${T}confirm${T}review-grok" 'clean pair requires third provider'
record_report 3 round1 confirm grok AGREE
assert_eq "$(next)" "a4${T}final${T}final${T}review-final" 'clean triple skips all remaining rounds'
assert_eq "$(wc -l < "$LEDGER" | tr -d ' ')" 3 'no skipped rows added'
for event in COMMENT APPROVE; do
  run_dir="${LEDGER%.ledger.tsv}-a4"
  mkdir -p "$run_dir"
  jq -n --arg event "$event" '{outcome:"done",event:$event,commentUrl:"https://example.test/review/2"}' > "$run_dir/status.json"
  cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage final --role final --provider claude
  assert_exit "$?" 0 "posted $event accepted"
  assert_eq "$(next)" done "$event final completes pipeline"
  head -3 "$LEDGER" > "$TMP/reset.tsv"
  mv "$TMP/reset.tsv" "$LEDGER"
done

echo 'review: T veto takes over critic slot and prevents early stop'
LEDGER="$TMP/loop/runs/review-p7-veto.ledger.tsv"
record_report 1 round1 review gpt APPROVE
record_report 2 round1 critic claude AGREE
record_report 3 round1 confirm grok 'DISAGREE_CONCERN: F2 src/replay.sh:10'
assert_eq "$(next)" "a4${T}round1${T}respond${T}review-gpt" 'veto goes back to reviewer'
record_report 4 round1 respond gpt 'APPROVE'
assert_eq "$(next)" "a5${T}round1${T}critic${T}review-grok" 'vetoing third provider stays critic'
record_report 5 round1 critic grok AGREE
assert_eq "$(next)" "a6${T}round2${T}review${T}review-claude" 'veto prevents skipping remaining rounds'

echo 'review: exhausted inner cap writes dispute evidence and a synthetic row'
LEDGER="$TMP/loop/runs/review-p7-dispute.ledger.tsv"
record_report 1 round1 review gpt $'F1 [major] replay loses state\nNEEDS_CHANGES: F1'
record_report 2 round1 critic claude $'F2 [major] src/replay.sh:10 also loses state\nDISAGREE_CONCERN: F2'
assert_eq "$(next)" "a3${T}round1${T}respond${T}review-gpt" 'first disagreement permits one response'
record_report 3 round1 respond gpt 'APPROVE'
record_report 4 round1 critic claude $'F2 [major] src/replay.sh:10 still wrong\nDISAGREE_EVIDENCE: F2'
assert_eq "$(next)" "a6${T}fix1${T}fix${T}review-fix" 'cap materializes synthetic a5 then requires fix'
assert_eq "$(tail -1 "$LEDGER" | cut -f1-5)" "a5${T}disputed${T}disputed${T}-${T}DISPUTED" 'disputed row uses frozen contract'
assert_contains "$(cat "${LEDGER%.tsv}.disputed.md")" F2 'critic-only finding preserved in dispute sidecar'
assert_eq "$(next)" "a6${T}fix1${T}fix${T}review-fix" 'next replay never duplicates disputed row'
assert_eq "$(wc -l < "$LEDGER" | tr -d ' ')" 5 'exactly one dispute row'

echo 'review: recover replays finished unrecorded runs only from this pipeline'
LEDGER="$TMP/loop/runs/review-p8-recovery.ledger.tsv"
for k in 1 2 3; do
  run_dir="${LEDGER%.ledger.tsv}-a$k"
  mkdir -p "$run_dir"
  printf '{"engineExit":0,"timedOut":false}\n' > "$run_dir/meta.json"
  printf '{"outcome":"done","summary":"finished"}\n' > "$run_dir/status.json"
  if (( k == 1 )); then print APPROVE > "$run_dir/report.md"; else print AGREE > "$run_dir/report.md"; fi
done
mkdir -p "$TMP/loop/runs/review-p8-other-a1" "${LEDGER%.ledger.tsv}-a4"
printf '{"engineExit":0}\n' > "$TMP/loop/runs/review-p8-other-a1/meta.json"
printf 'invalid\n' > "$TMP/loop/runs/review-p8-other-a1/report.md"
# a4 remains live (no meta); it must not be recorded.
cli recover --ledger "$LEDGER" --runs-dir "$TMP/loop/runs"
assert_exit "$?" 0 'crash before record is recoverable'
assert_eq "$(next)" "a4${T}final${T}final${T}review-final" 'recovery reconstructs clean triple in order'
assert_eq "$(wc -l < "$LEDGER" | tr -d ' ')" 3 'unrelated pipeline and live run ignored'
cli recover --ledger "$LEDGER" --runs-dir "$TMP/loop/runs"
assert_exit "$?" 0 'recovery replay is idempotent'
assert_eq "$(wc -l < "$LEDGER" | tr -d ' ')" 3 'recovery does not duplicate rows'
run_dir="${LEDGER%.ledger.tsv}-a4"
printf '{"engineExit":0}\n' > "$run_dir/meta.json"
printf '{"outcome":"question","event":"COMMENT","commentUrl":"https://example.test/review/3"}\n' > "$run_dir/status.json"
cli recover --ledger "$LEDGER" --runs-dir "$TMP/loop/runs"
assert_exit "$?" 0 'recovery records posted final even for question outcome'
assert_eq "$(next)" done 'recovered final ends pipeline'

echo 'review: failed run and invalid recovered verdict cannot advance'
LEDGER="$TMP/loop/runs/review-p8-invalid.ledger.tsv"
run_dir="${LEDGER%.ledger.tsv}-a1"
mkdir -p "$run_dir"
printf '{"engineExit":1}\n' > "$run_dir/meta.json"
printf '{"outcome":"done"}\n' > "$run_dir/status.json"
printf 'not a verdict\n' > "$run_dir/report.md"
cli recover --ledger "$LEDGER" --runs-dir "$TMP/loop/runs"
assert_exit "$?" 0 'failed engine is not a completed review'
assert_eq "$(next)" "a1${T}round1${T}review${T}review-gpt" 'failed engine leaves first step pending'
printf '{"engineExit":0}\n' > "$run_dir/meta.json"
cli recover --ledger "$LEDGER" --runs-dir "$TMP/loop/runs"
assert_exit "$?" 2 'finished invalid report propagates retry signal'
assert_eq "$(next)" "a1${T}round1${T}review${T}review-gpt" 'invalid recovered report has no row'

echo 'review: R/C prompts preserve vendored AR text and full PR inspection framing'
NOTES="$TMP/notes.md"
printf 'Verify: zsh tests/run.sh\nSteering: preserve replay invariants\n' > "$NOTES"
LEDGER="$TMP/loop/runs/review-p9-prompts.ledger.tsv"
record_report 1 round1 review gpt $'F1 [major] first review evidence\nNEEDS_CHANGES: F1'
prompt() { cli prompt --role "$1" --pr https://github.com/acme/api/pull/7 --head "$AFTER" --notes "$NOTES" --ledger "$LEDGER" --run-dir "${LEDGER%.ledger.tsv}-a2"; }
for role in review critic respond confirm; do
  rendered="$(prompt "$role")"; rc=$?
  assert_exit "$rc" 0 "$role prompt renders"
  assert_contains "$rendered" 'https://github.com/acme/api/pull/7' "$role receives PR URL"
  assert_contains "$rendered" "$AFTER" "$role receives frozen head"
  assert_contains "$rendered" 'Steering: preserve replay invariants' "$role receives notes verbatim"
  assert_contains "$rendered" 'Prevention pass' "$role includes full inspection methodology"
  assert_contains "$rendered" 'Do not call any GitHub mutation endpoint' "$role is local-only"
  if [[ "$role" != review ]]; then
    assert_contains "$rendered" 'first review evidence' "$role receives prior exchange"
  fi
done
prompt critic > "$TMP/prompt.md"
python3 - "$ROOT/.claude/skills/loop-execute/references/ar-prompts.md" "$TMP/prompt.md" <<'CHECK'
from pathlib import Path
import sys
assert Path(sys.argv[2]).read_text().startswith(Path(sys.argv[1]).read_text())
CHECK
assert_exit "$?" 0 'vendored source is rendered verbatim'

echo 'review: fix/final receive current converged review and current dispute only'
LEDGER="$TMP/loop/runs/review-p7-dispute.ledger.tsv"
for role in fix final; do
  rendered="$(prompt "$role")"; rc=$?
  assert_exit "$rc" 0 "$role prompt renders"
  assert_contains "$rendered" 'Converged review' "$role gets last reviewer report"
  assert_contains "$rendered" 'F2' "$role gets critic-only disputed evidence"
  assert_contains "$rendered" 'Verify these disputed findings against the code' "$role verifies disputed claims"
done
rendered="$(prompt final)"
assert_contains "$rendered" 'REQUEST_CHANGES' 'final preserves severity-event mapping'
assert_contains "$rendered" '<!-- loop-review:' 'final preserves idempotency marker'
assert_contains "$rendered" 'commentUrl' 'final records posted event and URL'
# New outer round must not consume the old sidecar or old reviewer report.
run_dir="${LEDGER%.ledger.tsv}-a6"
mkdir -p "$run_dir"
fix_status NOOP "$AFTER" "$AFTER"
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage fix1 --role fix --provider claude
record_report 7 round2 review claude $'F3 [minor] current round only\nAPPROVE'
record_report 8 round2 critic grok AGREE
record_report 9 round2 confirm gpt AGREE
rendered="$(prompt final)"
assert_contains "$rendered" 'current round only' 'final uses latest round review'
[[ "$rendered" != *'F2 [major]'* ]]; assert_exit "$?" 0 'stale dispute excluded after later convergence'
[[ "$rendered" != *'F1 [major] replay loses state'* ]]; assert_exit "$?" 0 'earlier reviewer report excluded'
prompt nonsense >/dev/null; assert_exit "$?" 1 'unknown prompt role rejected'

echo 'review: recover retains nondefault pipeline limits without extra flags'
LEDGER="$TMP/loop/runs/review-p10-single.ledger.tsv"
next --tier max --rounds 1 --inner 1 >/dev/null
record_report 1 round1 review gpt 'NEEDS_CHANGES: F1'
record_report 2 round1 critic claude AGREE
run_dir="${LEDGER%.ledger.tsv}-a3"
mkdir -p "$run_dir"
printf '{"engineExit":0}\n' > "$run_dir/meta.json"
printf '{"outcome":"done","event":"APPROVE","commentUrl":"https://example.test/review/4"}\n' > "$run_dir/status.json"
cli recover --ledger "$LEDGER" --runs-dir "$TMP/loop/runs"
assert_exit "$?" 0 'recover remembers one outer round'
assert_eq "$(next --tier max --rounds 1 --inner 1)" done 'nondefault final recovered'

echo 'review: wrapper HEAD evidence cannot be overridden by agent status'
LEDGER="$TMP/loop/runs/review-p10-head-evidence.ledger.tsv"
record_report 1 round1 review gpt 'NEEDS_CHANGES: F1'
record_report 2 round1 critic claude AGREE
run_dir="${LEDGER%.ledger.tsv}-a3"
mkdir -p "$run_dir"
jq -n --arg before "$BEFORE" --arg after "$AFTER" '{engineExit:0,headBefore:$before,headAfter:$after}' > "$run_dir/meta.json"
git --git-dir="$REMOTE" update-ref refs/heads/review "$BEFORE"
fix_status NOOP "$BEFORE" "$BEFORE"
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage fix1 --role fix --provider claude
assert_exit "$?" 1 'status cannot hide unpublished wrapper HEAD'
assert_eq "$(wc -l < "$LEDGER" | tr -d ' ')" 2 'conflicting evidence writes no row'
git --git-dir="$REMOTE" update-ref refs/heads/review "$AFTER"
fix_status PUSHED "$BEFORE" "$AFTER"
jq 'del(.headBefore,.headAfter)' "$run_dir/status.json" > "$TMP/status.tmp"
mv "$TMP/status.tmp" "$run_dir/status.json"
cli record --ledger "$LEDGER" --run-dir "$run_dir" --stage fix1 --role fix --provider claude
assert_exit "$?" 0 'wrapper supplies hashes when status omits them'
test_summary
