#!/bin/zsh
# Runs the ws suite. Area files run in 4 independent groups, in parallel by default: each group
# is its own process with its own sandbox (helpers.sh: private HOME, tmux server, fake agents,
# real-HOME guard); its areas are sourced in order since workspaces, tabs and session ids carry
# over within a group. Non-core groups start from ws_tests_seed (api repo + session #1).
#   zsh ws/tests/run.sh           all groups in parallel, logs printed per group, then a TOTAL
#   zsh ws/tests/run.sh <group>   one group (core|send|triggers|misc)
#   WS_TESTS_JOBS=1               all areas sequentially in one sandbox (pre-split order)
#   WS_TESTS_EXPECT=<n>           also fail if fewer than n assertions ran
# Exits non-zero on any failure.
set -u
HERE="${0:A:h}"
typeset -A groups=(
  core "workspace hooks tabs picker setup notify rm"
  send "send"
  triggers "triggers inflight triggers-actions trigger-edges review-fixes"
  misc "lifecycle merge migrate"
)
order=(core send triggers misc)

ws_tests_areas() {
  local area
  for area in "$@"; do
    echo "== ws: $area =="
    source "$HERE/$area.test.sh"
  done
  test_summary
}

if [[ $# -gt 0 ]]; then
  [[ -n "${groups[$1]:-}" ]] || { print -u2 "usage: run.sh [${(j:|:)order}]"; exit 2; }
  source "$HERE/helpers.sh"
  [[ "$1" == core ]] || ws_tests_seed
  ws_tests_areas ${=groups[$1]}
fi
if [[ "${WS_TESTS_JOBS:-}" == 1 ]]; then
  source "$HERE/helpers.sh"
  ws_tests_areas workspace hooks tabs picker setup notify merge send lifecycle triggers inflight \
    triggers-actions trigger-edges review-fixes rm migrate
fi

logs="$(mktemp -d)"
trap 'rm -rf "$logs"' EXIT
typeset -A pids
for g in $order; do
  zsh "$HERE/run.sh" "$g" > "$logs/$g.log" 2>&1 &
  pids+=("$g" $!)
done
fail=0 total=0 failed=0
for g in $order; do
  wait "${pids[$g]}" || fail=1
  echo "== ws group: $g =="
  cat "$logs/$g.log"
  # test_summary's last line: "<n> passed" or "<f>/<n> failed" (colored)
  summary="$(tail -n 1 "$logs/$g.log" | sed $'s/\033\\[[0-9;]*m//g')"
  if [[ "$summary" =~ '^([0-9]+) passed$' ]]; then
    (( total += match[1] ))
  elif [[ "$summary" =~ '^([0-9]+)/([0-9]+) failed$' ]]; then
    (( failed += match[1], total += match[2] ))
  else
    print "  FAIL: group $g ended without a summary"; fail=1
  fi
  echo
done
if [[ -n "${WS_TESTS_EXPECT:-}" ]] && (( total < WS_TESTS_EXPECT )); then
  print "  FAIL: $total assertions ran, expected at least $WS_TESTS_EXPECT"; fail=1
fi
echo "== ws TOTAL =="
if (( fail == 0 && failed == 0 )); then
  printf '\033[32m%s\033[0m\n' "$total passed"; exit 0
fi
printf '\033[31m%s\033[0m\n' "$failed/$total failed"; exit 1
