#!/bin/zsh
set -u -o pipefail

# Loop Engineering — adversarial plan review (multiphase-plan, skipped by `light`). Spawns three
# independent, read-only reviewers in parallel (loop-runner chains plan-review-a/b/c: one per
# provider, see loop-models.conf) against a drafted plan. Mechanism only: the planner chat
# reconciles the reports and edits the plan.
#
# Usage:
#   loop-plan-review.sh run [--dir .loop] [--plan-file <dir>/plan.md] [--worktree .]
#     [--models-conf <f>]
#     → one TSV line per reviewer: <a|b|c> \t <runner exit> \t <report.md path, or "-">
#
# Exit: 0 at least one usable report · 2 no usable report · 1 usage
# Env: LOOP_PLAN_REVIEW_RUNNER overrides the runner (tests).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RUNNER="${LOOP_PLAN_REVIEW_RUNNER:-$SCRIPT_DIR/loop-runner.sh}"
PLAN_FORMAT="$SCRIPT_DIR/.claude/skills/multiphase-plan/references/plan-format.md"
REVIEWERS=(a b c)
DIR=".loop" PLAN_FILE="" WT="$PWD" MODELS_CONF=""

die() { echo "loop-plan-review: $*" >&2; exit 1; }

CMD="${1:-}"; shift 2>/dev/null || true
[[ "$CMD" == run ]] || die "usage: loop-plan-review.sh run [--dir D] [--plan-file F] [--worktree W] [--models-conf F]"
while [[ $# -gt 0 ]]; do
  case "$1" in
  --dir) DIR="$2"; shift 2 ;;
  --plan-file) PLAN_FILE="$2"; shift 2 ;;
  --worktree) WT="$2"; shift 2 ;;
  --models-conf) MODELS_CONF="$2"; shift 2 ;;
  *) die "unknown arg $1" ;;
  esac
done
PLAN_FILE="${PLAN_FILE:-$DIR/plan.md}"
[[ -s "$PLAN_FILE" ]] || die "plan file '$PLAN_FILE' missing or empty"
mkdir -p "$DIR" || die "cannot create '$DIR'"
DIR="${DIR:A}" PLAN_FILE="${PLAN_FILE:A}" WT="${WT:A}"

main() {
  local stamp x run_dir pid usable=0
  local -A pids
  stamp="$(date +%Y%m%d-%H%M%S)-$$"
  for x in $REVIEWERS; do
    run_dir="$DIR/runs/plan-review-$stamp-$x"
    mkdir -p "$run_dir"
    write_prompt "$run_dir" > "$run_dir/prompt.md"
    zsh "$RUNNER" --worktree "$WT" --run-dir "$run_dir" --prompt-file "$run_dir/prompt.md" \
      --chain "plan-review-$x" --loop-dir "$DIR" ${MODELS_CONF:+--models-conf "$MODELS_CONF"} \
      > "$run_dir/spawn.log" 2>&1 &
    pid=$!; pids[$x]=$pid
  done
  for x in $REVIEWERS; do
    run_dir="$DIR/runs/plan-review-$stamp-$x"
    wait "${pids[$x]}"
    report_row "$x" "$?" "$run_dir" && usable=1
  done
  (( usable )) || exit 2
}

# The reviewer prompt: plan-specific adversarial checklist, read-only, report + status.json.
write_prompt() {
  local run_dir="$1"
  cat <<EOF
You are an adversarial reviewer of a DRAFTED multi-phase implementation plan, before any code
is written. Find what will make this plan fail when headless agents execute it phase by phase.

- Plan: $PLAN_FILE
- Codebase: $WT (current worktree; other repositories in the plan may be mapped in
  ~/.loop/repos.json — read them only if present)
EOF
  [[ -r "$PLAN_FORMAT" ]] && print -r -- "- Plan format contract: $PLAN_FORMAT"
  cat <<EOF

Do not edit, create, or delete any file except your two outputs below; do not commit, push,
or call any network mutation. Review independently: do not read other plan-review runs under
$DIR/runs. Verify every claim against the code — cite file paths (and lines) as evidence.

Checklist:
1. Codebase assumptions — files, modules, APIs, commands the plan relies on actually exist and
   behave as the plan assumes.
2. Completeness — missing work: migrations, config, docs/READMEs, tests, cleanup, rollout.
3. Dependencies — every Depends on edge is real and none is missing; ordering is executable.
4. Independence — phases in different lanes pass the plan's Independence test (no dependency,
   low file contention, no coordination tax); flag lanes that will fight over the same files.
5. Sizing — each phase fits its Estimate and one 30-minute attempt; flag phases that won't.
6. Done when / Verify — acceptance criteria are concrete and checkable; Verify commands exist,
   are runnable from the repository root, and actually check the phase.
7. Slices & branches — vertical slices, descriptive branch names, repository declarations.
8. Review tier — the terminal review tier matches the effort's risk.
9. Ambiguity — anything a headless agent would have to guess or ask about.

Report at most 10 findings, most severe first. Each: severity (critical|major|minor),
the phase(s) affected, the problem, evidence, and a concrete plan edit. Skip style nits.
If the plan is sound, say so — do not invent findings.

Write the complete findings to $run_dir/report.md, then write $run_dir/status.json with
{"outcome":"done","summary":"<n findings, top severity>"}.
EOF
}

# One TSV row; returns 0 when the reviewer left a usable report (runner exit 0 + report.md).
report_row() {
  local x="$1" rc="$2" run_dir="$3" report="-"
  (( rc == 0 )) && [[ -s "$run_dir/report.md" ]] && report="$run_dir/report.md"
  printf '%s\t%s\t%s\n' "$x" "$rc" "$report"
  [[ "$report" != - ]]
}

main
