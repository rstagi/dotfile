#!/bin/zsh
# Minimal shell test lib for the loop-engineering scripts. Source it, run assert_*,
# end with test_summary (exits 0 all-pass / 1 any-fail). tests/run.sh aggregates files.
set -u

TESTS_RUN=0
TESTS_FAIL=0

_red() { printf '\033[31m%s\033[0m\n' "$1"; }
_grn() { printf '\033[32m%s\033[0m\n' "$1"; }

# assert_eq <actual> <expected> <msg>
assert_eq() {
  TESTS_RUN=$((TESTS_RUN + 1))
  if [[ "$1" == "$2" ]]; then
    _grn "  ok: $3"
  else
    TESTS_FAIL=$((TESTS_FAIL + 1))
    _red "  FAIL: $3"
    _red "    expected: [$2]"
    _red "    actual:   [$1]"
  fi
}

# assert_exit <actual-code> <expected-code> <msg>
assert_exit() {
  TESTS_RUN=$((TESTS_RUN + 1))
  if [[ "$1" == "$2" ]]; then
    _grn "  ok: $3 (exit $1)"
  else
    TESTS_FAIL=$((TESTS_FAIL + 1))
    _red "  FAIL: $3 (expected exit $2, got $1)"
  fi
}

# assert_contains <haystack> <needle> <msg>
assert_contains() {
  TESTS_RUN=$((TESTS_RUN + 1))
  if [[ "$1" == *"$2"* ]]; then
    _grn "  ok: $3"
  else
    TESTS_FAIL=$((TESTS_FAIL + 1))
    _red "  FAIL: $3"
    _red "    [$1] does not contain [$2]"
  fi
}

test_summary() {
  echo "-----"
  if [[ $TESTS_FAIL -eq 0 ]]; then
    _grn "$TESTS_RUN passed"
    exit 0
  fi
  _red "$TESTS_FAIL/$TESTS_RUN failed"
  exit 1
}
