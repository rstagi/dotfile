#!/bin/zsh
set -u -o pipefail

# Milestone merge-gate seam (human resume only; loop-execute applies the printed patch).
#   check   --state <state.json>
#     one "slug<TAB>url<TAB>open|merged|closed" row per state.awaitingMerge.prs entry.
#     exit 0 all merged, 3 any open, 4 any closed-unmerged (wins over open), 1 gh/state error.
#   advance --state <state.json> --next <N> --branches '<json slug→branch>'
#     prepares every next participant (loop-repo.sh prepare --milestone N) first, then retires
#     the old non-adopted worktrees (a non-participating runtime host keeps its own). Requires a
#     parked gate (awaitingMerge.prs) and N = parked + 1. Safe to re-run after a partial failure.
#     Prints the state patch JSON (milestone, repositories, runtimeRoot, integrationWorktree,
#     integrationBranch, prUrl/review/reviewPipeline/awaitingMerge: null, milestones history) to
#     apply via `loop-state.sh set ". + $patch"`; never writes state. Failures surface loop-repo's
#     exit code.

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
    if ! state="$("$LOOP_REPO" pr-state --pr "$url" </dev/null)"; then
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
  local milestone_re='^[1-9][0-9]*$' old slug branch run_id out rc host
  [[ "$NEXT" =~ $milestone_re ]] || die "advance requires --next <N>"
  jq -e 'type == "object" and length > 0 and all(.[]; type == "string" and length > 0)' <<< "${BRANCHES:-null}" >/dev/null 2>&1 \
    || die "advance requires --branches '{\"owner/repo\":\"branch\"}' (non-empty)"
  jq -e '.awaitingMerge.prs | type == "object" and length > 0' "$STATE" >/dev/null 2>&1 \
    || die "state is not parked at a merge gate (no awaitingMerge PRs)"
  old="$(jq -r '.awaitingMerge.milestone // .milestone // 1' "$STATE")"
  (( NEXT == old + 1 )) || die "next milestone $NEXT must be $(( old + 1 )) (parked at $old)"
  run_id="$(jq -er '.runId' "$STATE")" || die "state has no runId"

  # Prepare everything before touching anything old: a failure leaves the runtime intact.
  local -A prepared
  for slug in ${(f)"$(jq -r 'keys[]' <<< "$BRANCHES")"}; do
    branch="$(jq -r --arg s "$slug" '.[$s]' <<< "$BRANCHES")"
    out="$("$LOOP_REPO" prepare --repo "$slug" --run-id "$run_id" \
      --integration-branch "$branch" --milestone "$NEXT" </dev/null)"
    rc=$?
    (( rc == 0 )) || exit $rc
    prepared[$slug]="$out"
  done

  host="$(runtime_host)"
  retire_old "$run_id" "$host"
  print_patch "$old" "$host"
}

# Repository hosting the Loop runtime: state.runtimeRoot owner, else the worktree running this
# script (old or freshly prepared, so retries resolve the same host). Empty = installed runtime.
runtime_host() {
  local root slug wt
  root="$(jq -r '.runtimeRoot // ""' "$STATE")"
  while IFS=$'\t' read -r slug wt; do
    [[ -n "$wt" && -n "$root" && "${wt:A}" == "${root:A}" ]] && { print -r -- "$slug"; return; }
  done < <(old_worktrees)
  while IFS=$'\t' read -r slug wt; do
    in_tree "$HERE" "$wt" && { print -r -- "$slug"; return; }
  done < <(old_worktrees)
  for slug in ${(k)prepared}; do
    in_tree "$HERE" "$(jq -r .integrationWorktree <<< "${prepared[$slug]}")" && { print -r -- "$slug"; return; }
  done
}

# Retire old clean non-adopted worktrees; already-gone ones are a no-op (retry safe).
# A non-participating host keeps its worktree (it still runs the loop); the worktree holding
# this script retires last, through a surviving copy of loop-repo.sh.
retire_old() {
  local run_id="$1" host="$2" slug wt adopted rc self_slug="" self_wt="" tmp_repo
  local -a new_wts
  new_wts=(${(f)"$(for slug in ${(k)prepared}; do jq -r .integrationWorktree <<< "${prepared[$slug]}"; done)"})
  while IFS=$'\t' read -r slug wt adopted; do
    [[ -n "$wt" && "$adopted" != true ]] || continue
    (( ${new_wts[(Ie)$wt]} )) && continue
    [[ -n "$host" && "$slug" == "$host" && -z "${prepared[$slug]:-}" ]] && continue
    if in_tree "$HERE" "$wt"; then self_slug="$slug" self_wt="$wt"; continue; fi
    "$LOOP_REPO" retire --repo "$slug" --run-id "$run_id" --worktree "$wt" --adopted false </dev/null
    rc=$?
    (( rc == 0 )) || exit $rc
  done < <(jq -r '.repositories // {} | to_entries[] | "\(.key)\t\(.value.integrationWorktree // "")\t\(.value.adopted // false)"' "$STATE")
  [[ -n "$self_wt" ]] || return 0
  tmp_repo="$(mktemp -d)/loop-repo.sh" || die "cannot stage loop-repo.sh"
  cp "$LOOP_REPO" "$tmp_repo" || die "cannot stage loop-repo.sh"
  LOOP_REPO="$tmp_repo"
  "$LOOP_REPO" retire --repo "$self_slug" --run-id "$run_id" --worktree "$self_wt" --adopted false </dev/null
  rc=$?
  rm -rf "${tmp_repo:h}"
  (( rc == 0 )) || exit $rc
}

# State patch for `loop-state.sh set ". + $patch"`: top-level keys replace wholesale.
print_patch() {
  local old="$1" host="$2" repos slug root_slug root_wt="" first
  repos="$(for slug in ${(k)prepared}; do print -r -- "$slug"; print -r -- "${prepared[$slug]}"; done \
    | jq -Rn '[inputs] | [range(0; length; 2) as $i | {key: .[$i], value: (.[$i+1] | fromjson)}] | from_entries')"
  first="$(jq -r 'keys | sort | .[0]' <<< "$repos")"
  root_slug="$first"
  if [[ -n "$host" && -n "${prepared[$host]:-}" ]]; then
    root_slug="$host"
  elif [[ -n "$host" ]]; then
    # Host sits this milestone out: the runtime stays in its old worktree.
    root_wt="$(jq -r --arg h "$host" '.repositories[$h].integrationWorktree' "$STATE")"
  else
    # Host retained by an earlier milestone: runtimeRoot owned by no listed repository, still on disk.
    root_wt="$(jq -r '.runtimeRoot as $r | if ($r // "") != "" and ([.repositories // {} | .[] | .integrationWorktree] | index($r) | not) then $r else "" end' "$STATE")"
    [[ -n "$root_wt" && -d "$root_wt" ]] || root_wt=""
  fi
  jq -cn --slurpfile state "$STATE" --argjson repos "$repos" --argjson old "$old" \
    --argjson next "$NEXT" --arg root "$root_slug" --arg root_wt "$root_wt" '
    $state[0] as $s
    | {milestone: $next,
       runtimeRoot: (if $root_wt != "" then $root_wt else $repos[$root].integrationWorktree end),
       integrationWorktree: $repos[$root].integrationWorktree,
       integrationBranch: $repos[$root].integrationBranch,
       repositories: $repos,
       prUrl: null, review: null, reviewPipeline: null,
       awaitingMerge: null,
       milestones: (($s.milestones // {}) + {($old | tostring): {prs: ($s.awaitingMerge.prs // {})}})}'
}

old_worktrees() {
  jq -r '.repositories // {} | to_entries[] | "\(.key)\t\(.value.integrationWorktree // "")"' "$STATE"
}

# in_tree <path> <dir>: path is dir or inside it.
in_tree() {
  [[ -n "$2" ]] || return 1
  local p="${1:A}" d="${2:A}"
  [[ "$p" == "$d" || "$p" == "$d/"* ]]
}

load_state() {
  [[ -n "$STATE" ]] || die "command requires --state <state.json>"
  jq -e 'type == "object"' "$STATE" >/dev/null 2>&1 || die "cannot read state: $STATE"
}

die() { print -u2 "loop-milestone: $1"; exit "${2:-1}"; }

main "$@"
