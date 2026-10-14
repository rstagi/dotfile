#!/bin/zsh
# Run every tests/*.test.sh; exit non-zero if any file fails. Also syntax-checks the
# loop-*.sh + ws.sh scripts (zsh -n) first. Companion to `cd loop-web && npm test` for JS.
# Discard caller loop/color settings; each suite configures its own environment.
set -u
for inherited in ${(k)parameters}; do
  case "$inherited" in
    LOOP_*|WS_*|CLICOLOR*|FORCE_COLOR|NO_COLOR|TYPESAFE_API_KEY) unset "$inherited" ;;
  esac
done
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
fail=0
# Every suite runs under a private HOME; the real agent hooks/config must stay byte-identical.
TEST_HOME_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_HOME_DIR"' EXIT
REAL_HOME="${TESTS_REAL_HOME:-$HOME}"
if [[ -f "$HERE/home-guard.sh" ]]; then
  source "$HERE/home-guard.sh"
  home_guard_sandbox "$TEST_HOME_DIR/home"
  real_home_snapshot="$(home_guard_snapshot)"
else # copied-runner fixtures (runner.test.sh) ship run.sh alone: sandbox HOME, skip the guard
  export TESTS_REAL_HOME="$REAL_HOME" HOME="$TEST_HOME_DIR"
fi

echo "== zsh -n (syntax) =="
for s in loop-state.sh loop-orchestrator.sh loop-runner.sh loop-merge.sh loop-notify.sh loop-emit.sh loop-plan.sh loop-repo.sh loop-review.sh ws.sh ws-bus.sh; do
  [[ -f "$ROOT/$s" ]] || continue
  if zsh -n "$ROOT/$s"; then echo "  ok: $s"; else echo "  FAIL: $s"; fail=1; fi
done
echo

for t in "$HERE"/*.test.sh; do
  echo "== ${t:t} =="
  zsh "$t" || fail=1
  echo
done

if [[ -n "${real_home_snapshot+set}" ]]; then
  echo "== real HOME guard ($REAL_HOME) =="
  if home_guard_check "$real_home_snapshot"; then echo "  ok: agent hooks/config/settings untouched"; else fail=1; fi
  echo
fi

if [[ $fail -eq 0 ]]; then
  echo "ALL SHELL TESTS PASSED"; exit 0
fi
echo "SHELL TESTS FAILED"; exit 1
