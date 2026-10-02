#!/bin/zsh
# Throwaway demo test for the Loop showcase; delete after the demo.

showcase_greet() {
  printf 'hello, %s\n' "$1"
}

(( 1 + 1 == 2 )) || {
  print -u2 -- 'FAIL: 1 + 1 must equal 2'
  exit 1
}

[[ "$(showcase_greet 'Loop User')" == 'hello, Loop User' ]] || {
  print -u2 -- 'FAIL: showcase_greet must greet the supplied name'
  exit 1
}

print -- 'ok'
exit 0
