#!/bin/zsh
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec node --test "$ROOT/tests/question-triage.test.mjs"
