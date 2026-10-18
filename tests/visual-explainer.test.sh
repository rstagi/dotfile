#!/bin/zsh
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# The video test renders real MP4 files only when VE_VIDEO_E2E=1.
exec node --test "$ROOT/tests/visual-explainer.test.mjs" "$ROOT/tests/visual-explainer-video.test.mjs"
