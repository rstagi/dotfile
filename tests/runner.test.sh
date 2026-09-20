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

run_review() {
  local chain="$1" codex_outcome="${2:-rate-limit}"
  RUN_DIR="$TMP/$chain"
  mkdir -p "$RUN_DIR"
  export FAKE_RUN_DIR="$RUN_DIR"
  export FAKE_CODEX_OUTCOME="$codex_outcome"
  : > "$FAKE_ENGINE_LOG"
  zsh "$ROOT/loop-runner.sh" \
    --worktree "$ROOT" \
    --run-dir "$RUN_DIR" \
    --prompt-file "$TMP/prompt.md" \
    --chain "$chain" \
    --timeout 5 > "$TMP/$chain.out" 2>&1
  RC=$?
  INVOCATIONS="$(cat "$FAKE_ENGINE_LOG")"
}

echo "runner: staged adversarial review model chains"
run_review review-fable
assert_exit "$RC" "0" "Fable review completes"
assert_contains "$INVOCATIONS" 'claude -p --model claude-fable-5-1' "pins Fable 5.1"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "Fable review has no fallback reviewer"

run_review review-astra done
assert_exit "$RC" "0" "Astra review completes"
assert_contains "$INVOCATIONS" 'codex exec --json' "runs Astra through Codex"
assert_contains "$INVOCATIONS" '-m gpt-6-astra' "pins Astra 6"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "Astra review has no fallback reviewer"

run_review review-fix
assert_exit "$RC" "0" "Opus remediation coordinator completes"
assert_contains "$INVOCATIONS" 'claude -p --model claude-opus-5' "pins remediation coordinator to Opus 5"
assert_contains "$INVOCATIONS" '--agents' "registers the remediation fixer subagent"
assert_contains "$INVOCATIONS" '"model":"claude-opus-5"' "pins fixer subagents to Opus 5"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "remediation coordinator has no model fallback"

run_review review-final
assert_exit "$RC" "0" "Opus final review completes"
assert_contains "$INVOCATIONS" 'claude -p --model claude-opus-5' "pins final reviewer to Opus 5"
assert_eq "$([[ "$INVOCATIONS" == *'--agents'* ]] && echo yes || echo no)" "no" "final reviewer does not spawn remediation agents"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "final reviewer has no model fallback"

test_summary
