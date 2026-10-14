#!/bin/zsh
# loop-plan-review.sh run — three independent plan reviewers in parallel → reviewer/exit/report TSV.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"; TMP="${TMP:A}"
trap 'rm -rf "$TMP"' EXIT
LOOP="$TMP/wt/.loop"
mkdir -p "$LOOP"
print -r -- "# Plan" > "$LOOP/plan.md"

# Fake runner: logs its args, writes report.md + status.json per FAKE_PR_<x> (done|fail|noreport).
cat > "$TMP/runner.sh" <<'RUNNER'
#!/bin/zsh
set -u
chain="" run_dir="" prompt=""
args="$*"
while [[ $# -gt 0 ]]; do
  case "$1" in
  --chain) chain="$2"; shift 2 ;;
  --run-dir) run_dir="$2"; shift 2 ;;
  --prompt-file) prompt="$2"; shift 2 ;;
  *) shift ;;
  esac
done
print -r -- "$args" >> "$FAKE_LOG"
x="${chain#plan-review-}" var="FAKE_PR_${(U)x}"
mode="${(P)var:-done}"
mkdir -p "$run_dir"
[[ "$mode" == noreport ]] || print -r -- "findings $x" > "$run_dir/report.md"
[[ "$mode" == fail ]] && exit 40
[[ "$mode" == noreport ]] && exit 0
jq -cn '{outcome:"done"}' > "$run_dir/status.json"
exit 0
RUNNER
chmod +x "$TMP/runner.sh"
export LOOP_PLAN_REVIEW_RUNNER="$TMP/runner.sh" FAKE_LOG="$TMP/runner.log"

review() { : > "$FAKE_LOG"; (cd "$TMP/wt" && zsh "$ROOT/loop-plan-review.sh" run "$@") > "$TMP/out" 2>"$TMP/err"; RC=$?; OUT="$(cat "$TMP/out")"; }
T=$'\t'

echo "plan-review: all three reviewers run and report"
review
assert_exit "$RC" "0" "all reviewers done"
assert_eq "$(cut -f1,2 <<<"$OUT" | tr '\t\n' ' |')" "a 0|b 0|c 0|" "one TSV row per reviewer"
report_a="$(awk -F'\t' '$1=="a"{print $3}' <<<"$OUT")"
assert_eq "$(cat "$report_a")" "findings a" "report path points at reviewer A's report"
assert_contains "$report_a" "$LOOP/runs/plan-review-" "runs live under the loop dir"
LOG="$(cat "$FAKE_LOG")"
assert_contains "$LOG" "--chain plan-review-a" "spawns reviewer A"
assert_contains "$LOG" "--chain plan-review-b" "spawns reviewer B"
assert_contains "$LOG" "--chain plan-review-c" "spawns reviewer C"
assert_contains "$LOG" "--worktree $TMP/wt" "reviews the current worktree"
prompt="$(cat "${report_a:h}/prompt.md")"
assert_contains "$prompt" "$LOOP/plan.md" "prompt names the plan file"
assert_contains "$prompt" "${report_a:h}/report.md" "prompt names the reviewer's own report path"
assert_contains "$prompt" "Do not edit" "reviewers are read-only"
assert_contains "$prompt" "Independence" "prompt carries the plan checklist"

echo "plan-review: a failed reviewer is reported, the rest still count"
FAKE_PR_C=fail review
assert_exit "$RC" "0" "two of three reviewers are enough"
assert_eq "$(cut -f1,2 <<<"$OUT" | tr '\t\n' ' |')" "a 0|b 0|c 40|" "failure exit surfaced"
assert_eq "$(awk -F'\t' '$1=="c"{print $3}' <<<"$OUT")" "-" "no report for the failed reviewer"

echo "plan-review: a reviewer without status.json counts as failed"
FAKE_PR_A=noreport review
assert_eq "$(awk -F'\t' '$1=="a"{print $3}' <<<"$OUT")" "-" "missing report → '-'"

echo "plan-review: no usable report → exit 2"
FAKE_PR_A=fail FAKE_PR_B=fail FAKE_PR_C=fail review
assert_exit "$RC" "2" "all reviewers failed"

echo "plan-review: validation"
review --plan-file "$TMP/missing.md"
assert_exit "$RC" "1" "missing plan file rejected"
(cd "$TMP/wt" && zsh "$ROOT/loop-plan-review.sh" bogus) >/dev/null 2>&1
assert_exit "$?" "1" "unknown command rejected"

test_summary
