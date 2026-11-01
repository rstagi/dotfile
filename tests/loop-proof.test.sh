#!/bin/zsh
set -eu
HERE="${0:A:h}"
exec node --test "$HERE/loop-proof.test.mjs"
