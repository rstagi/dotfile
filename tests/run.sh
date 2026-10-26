#!/bin/zsh
# Run every shell suite and the picker smoke test. Fail if any suite fails.
# Check the Loop scripts, launcher and picker syntax first.
# Discard caller loop/color settings; each suite configures its own environment.
set -u
for inherited in ${(k)parameters}; do
  case "$inherited" in
    LOOP_*|WS_*|CLICOLOR*|FORCE_COLOR|NO_COLOR|TYPESAFE_API_KEY) unset "$inherited" ;;
  esac
done
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
failed_suites=()
# Every suite runs under a private HOME; the real agent hooks/config must stay byte-identical.
TEST_HOME_DIR="$(mktemp -d)"
ws_pid="" suite_pid=""
runner_cleanup() {
  trap '' INT TERM HUP
  [[ -z "$suite_pid" ]] || kill -TERM "$suite_pid" 2>/dev/null
  [[ -z "$ws_pid" ]] || kill -TERM "$ws_pid" 2>/dev/null
  [[ -z "$suite_pid" ]] || wait "$suite_pid" 2>/dev/null
  [[ -z "$ws_pid" ]] || wait "$ws_pid" 2>/dev/null
  rm -rf "$TEST_HOME_DIR"
}
trap runner_cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

run_suite() {
  zsh "$@" &
  suite_pid=$!
  wait "$suite_pid"
  local result=$?
  suite_pid=""
  return "$result"
}
REAL_HOME="${TESTS_REAL_HOME:-$HOME}"
if [[ -f "$HERE/home-guard.sh" ]]; then
  source "$HERE/home-guard.sh"
  home_guard_sandbox "$TEST_HOME_DIR/home"
  real_home_snapshot="$(home_guard_snapshot)"
else # copied-runner fixtures (runner.test.sh) ship run.sh alone: sandbox HOME, skip the guard
  export TESTS_REAL_HOME="$REAL_HOME" HOME="$TEST_HOME_DIR"
fi

echo "== zsh -n (syntax) =="
ws_scripts=(${(f)"$(cd "$ROOT" && print -l ws/ws.sh wsd/crates/ws/src/cmd/picker.zsh ws/tests/*.sh(N) tests/loop-e2e/*.sh(N))"}) # relative to $ROOT
for s in loop-state.sh loop-orchestrator.sh loop-runner.sh loop-merge.sh loop-notify.sh loop-emit.sh loop-plan.sh loop-plan-review.sh loop-repo.sh loop-milestone.sh loop-review.sh loop-ar.sh $ws_scripts; do
  [[ -f "$ROOT/$s" ]] || continue
  if zsh -n "$ROOT/$s"; then echo "  ok: $s"; else echo "  FAIL: $s"; failed_suites+=("zsh -n: $s"); fi
done
echo

# The picker smoke test uses its own sandbox and runs with the Loop suites.
ws_pid=""
if [[ -f "$ROOT/ws/tests/run.sh" ]]; then # needs the repo's ws/; absent from copied-runner fixtures
  zsh "$ROOT/ws/tests/run.sh" > "$TEST_HOME_DIR/ws.log" 2>&1 &
  ws_pid=$!
fi

for t in "$HERE"/*.test.sh; do
  echo "== ${t:t} =="
  run_suite "$t" || failed_suites+=("${t:t}")
  echo
done

if [[ -n "$ws_pid" ]]; then
  echo "== ws/tests/run.sh =="
  wait "$ws_pid" || failed_suites+=("ws/tests/run.sh")
  ws_pid=""
  cat "$TEST_HOME_DIR/ws.log"
  echo
fi

if [[ -n "${real_home_snapshot+set}" ]]; then
  echo "== real HOME guard ($REAL_HOME) =="
  if home_guard_check "$real_home_snapshot"; then echo "  ok: agent hooks/config/settings untouched"; else failed_suites+=("real HOME guard"); fi
  echo
fi

if [[ ${#failed_suites} -eq 0 ]]; then
  echo "ALL SHELL TESTS PASSED"; exit 0
fi
echo "SHELL TESTS FAILED: ${(j:, :)failed_suites}"; exit 1
