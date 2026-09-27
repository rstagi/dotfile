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

test_summary
