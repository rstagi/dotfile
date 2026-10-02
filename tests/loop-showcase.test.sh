#!/bin/zsh
# Throwaway demo test for the Loop showcase; delete after the demo.

typeset -i passed=0

showcase_greet() {
  printf 'hello, %s\n' "$1"
}

assert_eq() {
  [[ "$1" == "$2" ]] || {
    print -u2 -- "FAIL: $3 (expected '$2', got '$1')"
    exit 1
  }
  (( ++passed ))
}

assert_eq "$(( 1 + 1 ))" '2' '1 + 1 must equal 2'
assert_eq "$(showcase_greet 'Loop User')" 'hello, Loop User' \
  'showcase_greet must greet the supplied name'

print -- "showcase: $passed passed"
exit 0
