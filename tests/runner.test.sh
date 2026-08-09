#!/bin/zsh
# loop-runner.sh public CLI: configured phase models and effort reach each engine.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
RUN_DIR="$TMP/run"
mkdir -p "$RUN_DIR"
print -r -- "implement phase" > "$TMP/prompt.md"

export PATH="$HERE/fake:$PATH"
export FAKE_ENGINE_LOG="$TMP/engines.log"
export FAKE_RUN_DIR="$RUN_DIR"

zsh "$ROOT/loop-runner.sh" \
  --worktree "$ROOT" \
  --run-dir "$RUN_DIR" \
  --prompt-file "$TMP/prompt.md" \
  --chain task \
  --timeout 5 > "$TMP/runner.out" 2>&1
RC=$?
INVOCATIONS="$(cat "$FAKE_ENGINE_LOG")"

echo "runner: task phase model chain"
assert_exit "$RC" "0" "falls back from rate-limited Codex to Claude"
assert_contains "$INVOCATIONS" 'codex exec --json' "invokes Codex first"
assert_contains "$INVOCATIONS" '-m gpt-5.6-sol' "uses GPT-5.6-sol for the phase"
assert_contains "$INVOCATIONS" 'model_reasoning_effort="high"' "uses high Codex effort"
assert_contains "$INVOCATIONS" 'claude -p --model opus' "falls back to Opus"
assert_contains "$INVOCATIONS" '--effort high' "uses high Claude effort"
assert_eq "$([[ "$INVOCATIONS" == *'--fallback-model sonnet'* ]] && echo yes || echo no)" "no" "does not add a second Claude fallback"

test_summary
