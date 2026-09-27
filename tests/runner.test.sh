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
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "uses GPT-6-Sol for the phase"
assert_contains "$INVOCATIONS" 'model_reasoning_effort="high"' "uses high Codex effort"
assert_contains "$INVOCATIONS" 'claude -p --model claude-sonnet-5' "falls back to Sonnet 5"
assert_contains "$INVOCATIONS" '--effort high' "uses high Claude effort"
assert_eq "$([[ "$INVOCATIONS" == *'--fallback-model'* ]] && echo yes || echo no)" "no" "uses the runner's cross-engine fallback"

echo "runner: phase deadline returns a checkpoint to the orchestrator"
mkdir -p "$TMP/slow-bin" "$TMP/checkpoint"
cat > "$TMP/slow-bin/codex" <<'EOF'
#!/bin/zsh
print -r -- "codex $*" >> "$FAKE_ENGINE_LOG"
print -r -- '{"type":"thread.started","thread_id":"checkpoint-session"}'
sleep 10
EOF
cat > "$TMP/slow-bin/claude" <<'EOF'
#!/bin/zsh
print -r -- "claude $*" >> "$FAKE_ENGINE_LOG"
sleep 10
EOF
chmod +x "$TMP/slow-bin/codex" "$TMP/slow-bin/claude"
export PATH="$TMP/slow-bin:$PATH"
export FAKE_ENGINE_LOG="$TMP/checkpoint-engines.log"
zsh "$ROOT/loop-runner.sh" \
  --worktree "$ROOT" \
  --run-dir "$TMP/checkpoint" \
  --prompt-file "$TMP/prompt.md" \
  --chain task \
  --timeout 1 > "$TMP/checkpoint-runner.out" 2>&1
RC=$?
assert_exit "$RC" "10" "deadline asks orchestrator for guidance"
assert_eq "$(jq -r '.outcome' "$TMP/checkpoint/status.json" 2>/dev/null)" "question" "writes question status"
assert_eq "$(jq -r '.checkpoint' "$TMP/checkpoint/status.json" 2>/dev/null)" "true" "marks timed checkpoint"
assert_eq "$(jq -r '(.summary | length > 0) and (.question | length > 0)' "$TMP/checkpoint/status.json" 2>/dev/null)" "true" "gives orchestrator a status and request"
assert_eq "$([[ -s "$TMP/checkpoint/checkpoint.md" ]] && echo yes || echo no)" "yes" "captures observable progress"
assert_eq "$(jq -r '.timedOut' "$TMP/checkpoint/meta.json" 2>/dev/null)" "true" "records watchdog cutoff"
assert_eq "$(jq -r '.engine' "$TMP/checkpoint/meta.json" 2>/dev/null)" "codex" "records engine for resume"
assert_eq "$(jq -r '.sessionId' "$TMP/checkpoint/meta.json" 2>/dev/null)" "checkpoint-session" "keeps session for resume"
assert_eq "$(rg -c '^claude ' "$FAKE_ENGINE_LOG" 2>/dev/null || echo 0)" "0" "does not start fallback after deadline"

echo "runner: review stage also checkpoints at its deadline"
mkdir -p "$TMP/review-checkpoint"
zsh "$ROOT/loop-runner.sh" \
  --worktree "$ROOT" \
  --run-dir "$TMP/review-checkpoint" \
  --prompt-file "$TMP/prompt.md" \
  --chain review-fix \
  --timeout 1 > "$TMP/review-checkpoint.out" 2>&1
RC=$?
assert_exit "$RC" "10" "review deadline asks orchestrator for guidance"
assert_eq "$(jq -r '.checkpoint' "$TMP/review-checkpoint/status.json" 2>/dev/null)" "true" "review writes a checkpoint"
export PATH="$HERE/fake:$PATH"

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
assert_eq "$([[ "$INVOCATIONS" == *'--agents'* ]] && echo yes || echo no)" "no" "remediation delegation belongs to the skill"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "remediation coordinator has no model fallback"

run_review review-final
assert_exit "$RC" "0" "Opus final review completes"
assert_contains "$INVOCATIONS" 'claude -p --model claude-opus-5' "pins final reviewer to Opus 5"
assert_eq "$([[ "$INVOCATIONS" == *'--agents'* ]] && echo yes || echo no)" "no" "final reviewer does not spawn remediation agents"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "final reviewer has no model fallback"

test_summary
