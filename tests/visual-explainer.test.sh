#!/bin/zsh
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec node --test "$ROOT/tests/visual-explainer-graph.test.mjs" "$ROOT/tests/visual-explainer.test.mjs" "$ROOT/tests/visual-explainer-narration.test.mjs" "$ROOT/tests/visual-explainer-explore.test.mjs" "$ROOT/tests/visual-explainer-run.test.mjs"
