#!/bin/zsh
# Throwaway lane-B demo for the Loop parallel-lane showcase.
word='loop'

if [[ ${#word} -ne 4 ]]; then
  print -u2 -- 'parallel: expected "loop" length 4'
  exit 1
fi

print -- 'parallel: ok'
exit 0
