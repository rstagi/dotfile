#!/bin/zsh
# Plan-review CLI: one independent rotating R/C round over scripted verdicts.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"; TMP="${TMP:A}"
trap 'rm -rf "$TMP"' EXIT
LOOP="$TMP/wt/.loop"
mkdir -p "$LOOP"
print -r -- "# Plan" > "$LOOP/plan.md"
cat > "$TMP/runner.sh" <<'RUNNER'
#!/bin/zsh
set -u
chain="" run_dir="" prompt=""
while (( $# )); do
  case "$1" in
    --chain) chain="$2"; shift 2 ;;
    --run-dir) run_dir="$2"; shift 2 ;;
    --prompt-file) prompt="$2"; shift 2 ;;
    *) shift ;;
  esac
done
n=$(( $(wc -l < "$FAKE_LOG") + 1 ))
printf '%s\t%s\t%s\n' "$chain" "$run_dir" "$prompt" >> "$FAKE_LOG"
mode="$(sed -n "${n}p" "$FAKE_SCRIPT")"
[[ -n "$mode" ]] || exit 91
[[ "$mode" == fail ]] && exit 40
[[ "$mode" == noreport ]] && exit 0
printf 'Scripted report %s\n%s\n' "$n" "$mode" > "$run_dir/report.md"
print -r -- '{"outcome":"done"}' > "$run_dir/status.json"
RUNNER
export LOOP_PLAN_REVIEW_RUNNER="$TMP/runner.sh" FAKE_LOG="$TMP/runner.log" FAKE_SCRIPT="$TMP/verdicts"

review() {
  : > "$FAKE_LOG"
  (cd "$TMP/wt" && zsh "$ROOT/loop-plan-review.sh" run "$@") > "$TMP/out" 2> "$TMP/err"
  RC=$?; OUT="$(cat "$TMP/out")"
}

printf '%s\n' APPROVE AGREE AGREE > "$FAKE_SCRIPT"
echo 'plan-review: clean stop requires both critics'
review --round 1
assert_exit "$RC" 0 'clean review succeeds'
assert_eq "$(cut -f1 <<< "$OUT")" clean 'clean status'
report="$(cut -f2 <<< "$OUT")"; ledger="$(cut -f3 <<< "$OUT")"
assert_eq "$(tail -n 1 "$report" 2>/dev/null)" APPROVE 'result points at reviewer report'
assert_eq "$(cut -f1 "$FAKE_LOG" | tr '\n' ' ')" 'plan-review-gpt plan-review-claude plan-review-grok ' 'review, critic, third-provider confirmation'
assert_eq "$(cut -f2-5 "$ledger" 2>/dev/null | tr '\t\n' ' |')" 'round1 review gpt APPROVE|round1 critic claude AGREE|round1 confirm grok AGREE|' 'six-column ledger records protocol order'
assert_contains "$report" "$LOOP/runs/plan-review-" 'runs live beside ledger'
assert_contains "$report" '-r1-1/report.md' 'run path carries round and attempt'
prompt="$(cat "${report:h}/prompt.md" 2>/dev/null)"
assert_contains "$prompt" "$LOOP/plan.md" 'prompt names the drafted plan'
assert_contains "$prompt" 'Independence' 'reviewer keeps plan checklist'
assert_contains "$prompt" 'Role: review (A.2.3)' 'shared AR reviewer prompt'
critic_prompt="$(sed -n '2p' "$FAKE_LOG" | cut -f3)"
assert_contains "$(cat "$critic_prompt" 2>/dev/null)" 'Scripted report 1' 'critic receives prior review'

echo 'plan-review: findings converge without a third-provider confirmation'
printf '%s\n' 'NEEDS_CHANGES: missing command' AGREE > "$FAKE_SCRIPT"
review --round 1
assert_exit "$RC" 0 'findings round succeeds'
assert_eq "$(cut -f1 <<< "$OUT")" findings 'findings status'
assert_eq "$(wc -l < "$FAKE_LOG" | tr -d ' ')" 2 'findings need only reviewer and critic'
assert_eq "$(tail -n 1 "$(cut -f2 <<< "$OUT")")" 'NEEDS_CHANGES: missing command' 'returns converged findings'

echo 'plan-review: evidence disagreement gets a fresh reviewer response'
printf '%s\n' 'NEEDS_CHANGES: wrong command' 'DISAGREE_EVIDENCE: F1 is refuted at loop-plan.sh:1' APPROVE AGREE > "$FAKE_SCRIPT"
review --round 1
assert_exit "$RC" 0 'response converges'
assert_eq "$(cut -f1 <<< "$OUT")" clean 'code-backed dropped finding can converge clean'
assert_contains "$(cut -f2 <<< "$OUT")" '-r1-3/report.md' 'returns response instead of opening report'
assert_eq "$(cut -f1 "$FAKE_LOG" | tr '\n' ' ')" 'plan-review-gpt plan-review-claude plan-review-gpt plan-review-claude ' 'response uses original reviewer'
response_prompt="$(sed -n '3p' "$FAKE_LOG" | cut -f3)"
assert_contains "$(cat "$response_prompt")" 'DISAGREE_EVIDENCE: F1' 'response includes critic evidence'
assert_contains "$(cat "$response_prompt")" 'Role: respond (A.2.3)' 'response uses shared response binding'

echo 'plan-review: third-provider veto takes over critic slot'
printf '%s\n' APPROVE AGREE 'DISAGREE_CONCERN: F2 missing verification at tests/run.sh:1' 'NEEDS_CHANGES: F2 missing verification' AGREE > "$FAKE_SCRIPT"
review --round 1
assert_exit "$RC" 0 'veto round converges'
assert_eq "$(cut -f1 <<< "$OUT")" findings 'third-provider veto prevents clean result'
assert_eq "$(cut -f1 "$FAKE_LOG" | tr '\n' ' ')" 'plan-review-gpt plan-review-claude plan-review-grok plan-review-gpt plan-review-grok ' 'third provider remains critic'
ledger="$(cut -f3 <<< "$OUT")"
assert_eq "$(cut -f3 "$ledger" | tr '\n' ' ')" 'review critic confirm respond critic ' 'sequential inner exchange'

echo 'plan-review: unresolved critic-only findings produce disputed evidence'
printf '%s\n' APPROVE 'DISAGREE_CONCERN: F9 missing test at tests/run.sh:1' APPROVE 'DISAGREE_EVIDENCE: F9 remains at tests/run.sh:1' > "$FAKE_SCRIPT"
review --round 1
assert_exit "$RC" 0 'disputed review is usable'
assert_eq "$(cut -f1 <<< "$OUT")" disputed 'disputed outranks reviewer approval'
ledger="$(cut -f3 <<< "$OUT")"; disputed="${ledger%.tsv}.disputed.md"
assert_eq "$(tail -n 1 "$ledger" | cut -f2-5)" $'disputed\tdisputed\t-\tDISPUTED' 'disputed marker recorded'
assert_contains "$(cat "$disputed")" '- F9' 'sidecar includes critic-only finding'
assert_contains "$(cat "$disputed")" 'Scripted report 3' 'sidecar preserves last reviewer report'
assert_contains "$(cat "$disputed")" 'Scripted report 4' 'sidecar preserves last critic report'
assert_contains "$(cut -f2 <<< "$OUT")" '-r1-3/report.md' 'disputed result still points at last reviewer'

echo 'plan-review: explicit inner cap stops at first disagreement'
printf '%s\n' 'NEEDS_CHANGES: F1 broken' 'DISAGREE_CONCERN: F1 questionable' > "$FAKE_SCRIPT"
review --round 1 --inner 1
assert_exit "$RC" 0 'inner 1 accepted'
assert_eq "$(cut -f1 <<< "$OUT")" disputed 'disputed outranks findings'
assert_eq "$(wc -l < "$FAKE_LOG" | tr -d ' ')" 2 'inner cap prevents response'

echo 'plan-review: round 2 starts independently with rotated providers'
printf '%s\n' APPROVE AGREE AGREE > "$FAKE_SCRIPT"
review --round 2
assert_exit "$RC" 0 'independent round 2 succeeds'
assert_eq "$(cut -f1 "$FAKE_LOG" | tr '\n' ' ')" 'plan-review-claude plan-review-grok plan-review-gpt ' 'round 2 rotation'
assert_contains "$(cut -f2 <<< "$OUT")" '-r2-1/report.md' 'round 2 path'
ledger="$(cut -f3 <<< "$OUT")"
assert_eq "$(cut -f2 "$ledger" | sort -u)" round2 'independent ledger contains only round 2'
assert_eq "$(wc -l < "$ledger" | tr -d ' ')" 3 'no fix or final in a plan round'
opening_prompt="$(head -n 1 "$FAKE_LOG" | cut -f3)"
assert_eq "$(rg -c 'Scripted report' "$opening_prompt" || print 0)" 0 'opening review excludes previous rounds'

echo 'plan-review: provider failure aborts instead of substituting'
printf '%s\n' APPROVE fail > "$FAKE_SCRIPT"
review --round 1
assert_exit "$RC" 2 'critic failure makes round unusable'
assert_eq "$OUT" '' 'partial review is not advertised as converged'
assert_contains "$(cat "$TMP/err")" 'claude critic failed (exit 40)' 'provider failure reported'
assert_eq "$(wc -l < "$FAKE_LOG" | tr -d ' ')" 2 'no substitute provider'
printf '%s\n' fail > "$FAKE_SCRIPT"
review --round 2
assert_exit "$RC" 2 'opening reviewer failure is unusable'
assert_contains "$(cat "$TMP/err")" 'claude review failed (exit 40)' 'rotated reviewer failure reported'
printf '%s\n' APPROVE AGREE fail > "$FAKE_SCRIPT"
review --round 1
assert_exit "$RC" 2 'confirmation failure cannot pass as clean'
assert_contains "$(cat "$TMP/err")" 'grok confirm failed (exit 40)' 'confirmation failure reported'

echo 'plan-review: invalid verdict gets one format retry'
printf '%s\n' 'No valid verdict' APPROVE AGREE AGREE > "$FAKE_SCRIPT"
review --round 1
assert_exit "$RC" 0 'one retry can recover format'
assert_eq "$(wc -l < "$FAKE_LOG" | tr -d ' ')" 4 'retry is a fresh runner call'
retry_prompt="$(sed -n '2p' "$FAKE_LOG" | cut -f3)"
assert_contains "$(cat "$retry_prompt")" 'FORMAT REMINDER' 'retry explains required output'
ledger="$(cut -f3 <<< "$OUT")"
assert_eq "$(head -n 1 "$ledger" | cut -f1)" a2 'invalid attempt never enters ledger'
printf '%s\n' noreport noreport > "$FAKE_SCRIPT"
review --round 1
assert_exit "$RC" 2 'missing reports are unusable after one retry'
assert_eq "$(wc -l < "$FAKE_LOG" | tr -d ' ')" 2 'retry bound enforced'
assert_contains "$(cat "$TMP/err")" 'no usable verdict after retry' 'unusable report reported'

echo 'plan-review: retained path and models-conf flags'
mkdir -p "$TMP/custom/runs"
print -r -- '# Custom plan' > "$TMP/custom-plan.md"
print -r -- 'LOOP_REVIEW_INNER_DEFAULT=1' > "$TMP/models.conf"
printf '%s\n' APPROVE 'DISAGREE_CONCERN: F1 missing check' > "$FAKE_SCRIPT"
review --round 3 --dir "$TMP/custom" --plan-file "$TMP/custom-plan.md" --worktree "$TMP/wt" --models-conf "$TMP/models.conf"
assert_exit "$RC" 0 'existing flags retained'
assert_eq "$(cut -f1 <<< "$OUT")" disputed 'inner default loaded from custom models-conf'
assert_contains "$(cut -f3 <<< "$OUT")" "$TMP/custom/runs/" 'custom loop dir receives ledger'
prompt="$(head -n 1 "$FAKE_LOG" | cut -f3)"
assert_contains "$(cat "$prompt")" "$TMP/custom-plan.md" 'custom drafted plan in framing'
assert_eq "$(cut -f1 "$FAKE_LOG" | tr '\n' ' ')" 'plan-review-grok plan-review-gpt ' 'round 3 rotation'

echo 'plan-review: validation'
for args in '' '--round 0' '--round 6' '--round nope' '--round' '--round 1 --inner 0' '--round 1 --inner 6' '--round 1 --dir' '--round 1 --unknown value'; do
  review ${=args}
  assert_exit "$RC" 1 "reject invalid args: $args"
done
review --round 1 --plan-file "$TMP/missing.md"
assert_exit "$RC" 1 'missing plan file rejected'
review --round 1 --models-conf "$TMP/missing.conf"
assert_exit "$RC" 1 'missing config rejected'
(cd "$TMP/wt" && zsh "$ROOT/loop-plan-review.sh" bogus) >/dev/null 2>&1
assert_exit "$?" 1 'unknown command rejected'

test_summary
