#!/bin/zsh
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec node --test "$ROOT/tests/loop-top.test.mjs"
