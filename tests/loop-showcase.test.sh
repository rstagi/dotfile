#!/bin/zsh
# Throwaway demo test for the Loop showcase; delete after the demo.

(( 1 + 1 == 2 )) || {
  print -u2 -- 'FAIL: 1 + 1 must equal 2'
  exit 1
}

print -- 'ok'
exit 0
