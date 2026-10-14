#!/bin/zsh
set -u -o pipefail

# Milestone merge-gate seam (human resume only; loop-execute applies the printed patch).
#   check   --state <state.json>
#     one "slug<TAB>url<TAB>open|merged|closed" row per state.awaitingMerge.prs entry.
#     exit 0 all merged, 3 any open, 4 any closed-unmerged (wins over open), 1 gh/state error.
#   advance --state <state.json> --next <N> --branches '<json slug→branch>'
#     prepares every next participant (loop-repo.sh prepare --milestone N) first, then retires
#     the old non-adopted worktrees. Safe to re-run after a partial failure. Prints the state
#     patch JSON (milestone, repositories, runtimeRoot, integrationBranch, awaitingMerge:null,
#     milestones history) and never writes state. Failures surface loop-repo's exit code.

HERE="${0:A:h}"
LOOP_REPO="$HERE/loop-repo.sh"
STATE="" NEXT="" BRANCHES=""

main() {
  local cmd="${1:-}"
  [[ -n "$cmd" ]] && shift
  while [[ $# -gt 0 ]]; do
    case "$1" in
    --state) STATE="${2:-}"; shift 2 ;;
    --next) NEXT="${2:-}"; shift 2 ;;
    --branches) BRANCHES="${2:-}"; shift 2 ;;
    *) die "unknown argument '$1'" ;;
    esac
  done
  case "$cmd" in
  check) check_gate ;;
  advance) advance_milestone ;;
  *) die "unknown command '$cmd' (check|advance)" ;;
  esac
}

check_gate() {
  load_state
  local prs slug url state rc=0 open=false closed=false failed=false
  prs="$(jq -r '.awaitingMerge.prs // {} | to_entries | sort_by(.key)[] | "\(.key)\t\(.value)"' "$STATE")" \
    || die "cannot read $STATE"
  [[ -n "$prs" ]] || die "state has no awaitingMerge PRs"
  while IFS=$'\t' read -r slug url; do
    if ! state="$("$LOOP_REPO" pr-state --pr "$url")"; then
      print -u2 "loop-milestone: cannot read PR state: $url"
      failed=true
      continue
    fi
    print -r -- "$slug"$'\t'"$url"$'\t'"$state"
    [[ "$state" == open ]] && open=true
    [[ "$state" == closed ]] && closed=true
  done <<< "$prs"
  $failed && rc=1
  (( rc == 0 )) && $closed && rc=4
  (( rc == 0 )) && $open && rc=3
  return $rc
}

advance_milestone() {
  load_state
  local milestone_re='^[1-9][0-9]*$' old slug branch run_id out rc
  [[ "$NEXT" =~ $milestone_re ]] || die "advance requires --next <N>"
  jq -e 'type == "object" and length > 0 and all(.[]; type == "string" and length > 0)' <<< "${BRANCHES:-null}" >/dev/null 2>&1 \
    || die "advance requires --branches '{\"owner/repo\":\"branch\"}' (non-empty)"
  old="$(jq -r '.awaitingMerge.milestone // .milestone // 1' "$STATE")"
  (( NEXT > old )) || die "next milestone $NEXT must follow current milestone $old"
  run_id="$(jq -er '.runId' "$STATE")" || die "state has no runId"

  # Prepare everything before touching anything old: a failure leaves the runtime intact.
  local -A prepared
  for slug in ${(f)"$(jq -r 'keys[]' <<< "$BRANCHES")"}; do
    branch="$(jq -r --arg s "$slug" '.[$s]' <<< "$BRANCHES")"
    out="$("$LOOP_REPO" prepare --repo "$slug" --run-id "$run_id" \
      --integration-branch "$branch" --milestone "$NEXT")"
    rc=$?
    (( rc == 0 )) || exit $rc
    prepared[$slug]="$out"
  done

  retire_old "$run_id"
  print_patch "$old"
}

# Retire old clean non-adopted worktrees; already-gone ones are a no-op (retry safe).
retire_old() {
  local run_id="$1" slug wt adopted rc
  local -a new_wts
  new_wts=(${(f)"$(for slug in ${(k)prepared}; do jq -r .integrationWorktree <<< "${prepared[$slug]}"; done)"})
  while IFS=$'\t' read -r slug wt adopted; do
    [[ -n "$wt" && "$adopted" != true ]] || continue
    (( ${new_wts[(Ie)$wt]} )) && continue
    "$LOOP_REPO" retire --repo "$slug" --run-id "$run_id" --worktree "$wt" --adopted false
    rc=$?
    (( rc == 0 )) || exit $rc
  done < <(jq -r '.repositories // {} | to_entries[] | "\(.key)\t\(.value.integrationWorktree // "")\t\(.value.adopted // false)"' "$STATE")
}

print_patch() {
  local old="$1" repos slug root_slug root_wt
  repos="$(for slug in ${(k)prepared}; do print -r -- "$slug"; print -r -- "${prepared[$slug]}"; done \
    | jq -Rn '[inputs] | [range(0; length; 2) as $i | {key: .[$i], value: (.[$i+1] | fromjson)}] | from_entries')"
  # Runtime root stays with the repository that hosted it, else the first participant.
  root_wt="$(jq -r '.runtimeRoot // ""' "$STATE")"
  root_slug="$(jq -r --arg wt "$root_wt" \
    '(.repositories // {} | to_entries | map(select(.value.integrationWorktree == $wt)) | .[0].key) // ""' "$STATE")"
  [[ -n "$root_slug" && -n "${prepared[$root_slug]:-}" ]] || root_slug="$(jq -r 'keys | sort | .[0]' <<< "$repos")"
  jq -cn --slurpfile state "$STATE" --argjson repos "$repos" --argjson old "$old" \
    --argjson next "$NEXT" --arg root "$root_slug" '
    $state[0] as $s
    | {milestone: $next,
       runtimeRoot: $repos[$root].integrationWorktree,
       integrationBranch: $repos[$root].integrationBranch,
       repositories: $repos,
       awaitingMerge: null,
       milestones: (($s.milestones // {}) + {($old | tostring): {prs: ($s.awaitingMerge.prs // {})}})}'
}

load_state() {
  [[ -n "$STATE" ]] || die "command requires --state <state.json>"
  jq -e 'type == "object"' "$STATE" >/dev/null 2>&1 || die "cannot read state: $STATE"
}

die() { print -u2 "loop-milestone: $1"; exit "${2:-1}"; }

main "$@"
