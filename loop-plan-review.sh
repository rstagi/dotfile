#!/bin/zsh
set -u -o pipefail

# One independent AR plan-review round. Only the planner edits the plan, between rounds.
# Usage: loop-plan-review.sh run --round r [--inner M] [--dir .loop]
#   [--plan-file <dir>/plan.md] [--worktree .] [--models-conf F]
# Output: clean|findings|disputed<TAB>last reviewer report<TAB>ledger
# Exit: 0 usable round; 2 provider failure / unusable verdict; 1 usage or local error.
# Env: LOOP_PLAN_REVIEW_RUNNER overrides the runner (tests).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RUNNER="${LOOP_PLAN_REVIEW_RUNNER:-$SCRIPT_DIR/loop-runner.sh}"
PLAN_FORMAT="$SCRIPT_DIR/.claude/skills/multiphase-plan/references/plan-format.md"
DIR=".loop" PLAN_FILE="" WT="$PWD" MODELS_CONF="$SCRIPT_DIR/loop-models.conf"
ROUND="" INNER=""
source "$SCRIPT_DIR/loop-ar.sh"

main() {
  parse_args "$@"
  source "$MODELS_CONF" || die "cannot source '$MODELS_CONF'"
  INNER="${INNER:-${LOOP_REVIEW_INNER_DEFAULT:-2}}"
  [[ "$ROUND" == [1-5] ]] || die "--round must be 1..5"
  [[ "$INNER" == [1-5] ]] || die "--inner must be 1..5"
  PLAN_FILE="${PLAN_FILE:-$DIR/plan.md}"
  [[ -s "$PLAN_FILE" ]] || die "plan file '$PLAN_FILE' missing or empty"
  [[ -d "$WT" ]] || die "worktree '$WT' missing"
  mkdir -p "$DIR/runs" || die "cannot create '$DIR/runs'"
  DIR="${DIR:A}" PLAN_FILE="${PLAN_FILE:A}" WT="${WT:A}" MODELS_CONF="${MODELS_CONF:A}"

  local stamp ledger framing next stage role provider run_dir verdict kind rc report="" k=0 retry
  local -a step
  stamp="$(date +%Y%m%d-%H%M%S)-$$"
  ledger="$DIR/runs/plan-review-$stamp-r$ROUND.ledger.tsv"
  framing="${ledger%.ledger.tsv}.framing.md"
  : > "$ledger" || die "cannot create ledger"
  write_framing > "$framing" || die "cannot write framing"
  while true; do
    next="$(ar_next --rounds "$ROUND" --inner "$INNER" --ledger "$ledger" \
      --start-round "$ROUND" --single-round)" || die "cannot replay '$ledger'"
    [[ "$next" == done ]] && break
    step=(${(ps:\t:)next})
    stage="$step[1]" role="$step[2]" provider="$step[3]"
    if [[ "$role" == disputed ]]; then
      ar_write_disputed --ledger "$ledger" > /dev/null || die "cannot write disputed items"
      continue
    fi
    kind=critic
    [[ "$role" == review || "$role" == respond ]] && kind=reviewer
    for retry in 0 1; do
      (( k++ ))
      run_dir="$DIR/runs/plan-review-$stamp-r$ROUND-$k"
      mkdir -p "$run_dir" || die "cannot create '$run_dir'"
      ar_prompt --role "$role" --framing "$framing" --ledger "$ledger" --run-dir "$run_dir" \
        > "$run_dir/prompt.md" || die "cannot render $role prompt"
      if (( retry )); then
        print -r -- "FORMAT REMINDER: End report.md with exactly one valid $kind verdict line as specified above." \
          >> "$run_dir/prompt.md"
      fi
      zsh "$RUNNER" --worktree "$WT" --run-dir "$run_dir" --prompt-file "$run_dir/prompt.md" \
        --chain "plan-review-$provider" --loop-dir "$DIR" --models-conf "$MODELS_CONF" \
        > "$run_dir/spawn.log" 2>&1
      rc=$?
      if (( rc != 0 )); then
        echo "loop-plan-review: $provider $role failed (exit $rc); see $run_dir/spawn.log" >&2
        return 2
      fi
      verdict="$(ar_parse_verdict "$run_dir/report.md" "$kind")" && break
      if (( retry )); then
        echo "loop-plan-review: $provider $role has no usable verdict after retry; see $run_dir" >&2
        return 2
      fi
      echo "loop-plan-review: $provider $role invalid verdict; retrying once" >&2
    done
    record_run "$ledger" "$k" "$stage" "$role" "$provider" "$verdict" "${run_dir#$DIR/}" \
      || die "cannot record $role verdict"
    [[ "$kind" == reviewer ]] && report="$run_dir/report.md"
  done
  local round_status
  round_status="$(ar_round_status --ledger "$ledger" --round "$ROUND")" || die "cannot read round status"
  printf '%s\t%s\t%s\n' "$round_status" "$report" "$ledger"
}

parse_args() {
  [[ "${1:-}" == run ]] || die "usage: loop-plan-review.sh run --round r [--inner M] [--dir D] [--plan-file F] [--worktree W] [--models-conf F]"
  shift
  while (( $# )); do
    (( $# >= 2 )) || die "missing value for $1"
    [[ -n "$2" && "$2" != --* ]] || die "missing value for $1"
    case "$1" in
      --round) ROUND="$2" ;; --inner) INNER="$2" ;;
      --dir) DIR="$2" ;; --plan-file) PLAN_FILE="$2" ;;
      --worktree) WT="$2" ;; --models-conf) MODELS_CONF="$2" ;;
      *) die "unknown arg $1" ;;
    esac
    shift 2
  done
}

# The planner's artifact-specific framing; role rules and prior exchange come from ar_prompt.
write_framing() {
  cat <<EOF
Review a DRAFTED multi-phase implementation plan, before any code is written. Find what will
make this plan fail when headless agents execute it phase by phase.

- Plan: $PLAN_FILE
- Codebase: $WT (current worktree; other repositories in the plan may be mapped in
  ~/.loop/repos.json — read them only if present)
- Plan format contract: $PLAN_FORMAT

Do not edit, create, or delete any file except RUN_DIR/report.md and RUN_DIR/status.json;
do not commit, push, or call any network mutation. The plan and code are frozen for this
round. Use only the prior exchange in this prompt; do not read other plan-review runs under
$DIR/runs. Verify every claim against the code — cite file paths and lines as evidence.

Archived milestones: An \`[archived]\` milestone stub lists shipped phases. Each listed phase
exists and its status is done. Its full block and its progress-log entries are in
${PLAN_FILE:h}/milestones/m<N>.md. Do not read these files unless a finding needs them. Do not
report a missing Done when, Repository or Estimate for an archived phase. A Depends on edge to
an archived phase is satisfied. Do not propose edits to archived phases.

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

Each numbered finding must identify the phase(s) affected, its class, the problem, code
evidence, and a concrete plan edit. A plan finding is blocking only in these categories:
- security: the plan ships a security or safety problem.
- unprotected-breakage: the plan ships a breaking change to existing behavior with
  no feature flag or off switch, or no wiring point for that flag.
- failing-check: a phase cannot pass a deterministic check, for example its Verify command.
All other findings are deferred. They never block the plan. The planner puts them in
the "Deferred / Not now" section of the plan. Skip style nits.
If the plan is sound, say so — do not invent findings.
EOF
}

# Atomic six-column ledger append. Each invocation owns an independent ledger.
record_run() {
  local ledger="$1" tmp
  shift
  tmp="$(mktemp "$ledger.tmp.XXXXXX")" || return 1
  {
    cat -- "$ledger" > "$tmp" && printf 'a%s\t%s\t%s\t%s\t%s\t%s\n' "$@" >> "$tmp" \
      && mv -f -- "$tmp" "$ledger"
  } always {
    rm -f -- "$tmp"
  }
}

die() { echo "loop-plan-review: $*" >&2; exit 1; }

main "$@"
