#!/bin/zsh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec node --test "$ROOT/tests/merge-risk.test.mjs"
