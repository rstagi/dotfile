#!/bin/zsh
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
node --test "$HERE/jev-recovery.test.mjs"
