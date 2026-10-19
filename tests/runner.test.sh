#!/bin/zsh
# loop-runner.sh public CLI: configured phase models and effort reach each engine.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export LOOP_JEV_KEY_FILE="$TMP/missing-key"
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
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "resolves @sol from the codex catalog for the phase"
assert_contains "$INVOCATIONS" 'model_reasoning_effort="high"' "uses high Codex effort"
assert_contains "$INVOCATIONS" 'claude -p --model sonnet ' "falls back to the latest Sonnet"
assert_contains "$INVOCATIONS" '--effort high' "uses high Claude effort"
assert_eq "$([[ "$INVOCATIONS" == *'--fallback-model'* ]] && echo yes || echo no)" "no" "uses the runner's cross-engine fallback"

echo "runner: legacy custom model config still runs correlated tasks"
cat > "$TMP/legacy-models.conf" <<'EOF'
CHAIN_TASK=("codex:legacy-model")
LOOP_BUDGET_USD=1
LOOP_TIMEOUT_TASK=5
EOF
mkdir -p "$TMP/legacy-a1"
export FAKE_RUN_DIR="$TMP/legacy-a1"
export FAKE_ENGINE_LOG="$TMP/legacy-engines.log"
export FAKE_JEV_LOG="$TMP/legacy-jev.log"
export FAKE_JEV_RESPONSE='{"version":1,"status":"fallback","stage":"route","mode":"off","reason":"missing_credentials"}'
export LOOP_JEV_CLIENT="$HERE/fake/loop-jev.mjs"
export FAKE_CODEX_OUTCOME=done
zsh "$ROOT/loop-runner.sh" \
  --worktree "$ROOT" \
  --run-dir "$FAKE_RUN_DIR" \
  --prompt-file "$TMP/prompt.md" \
  --models-conf "$TMP/legacy-models.conf" \
  --run-id legacy --phase 1 --repository rstagi/dotfile \
  --chain task --timeout 5 > "$TMP/legacy.out" 2>&1
RC=$?
assert_exit "$RC" "0" "legacy config reaches the engine"
assert_contains "$(cat "$FAKE_ENGINE_LOG" 2>/dev/null)" '-m legacy-model' "uses legacy configured chain"
unset FAKE_JEV_LOG FAKE_JEV_RESPONSE LOOP_JEV_CLIENT FAKE_CODEX_OUTCOME

echo "runner: local key file enables shadow mode with a legacy model config"
print -r -- 'file-key' > "$TMP/typesafe-api-key"
chmod 600 "$TMP/typesafe-api-key"
(
  unset TYPESAFE_API_KEY LOOP_JEV_MODE LOOP_JEV_MODE_EXPLICIT
  export LOOP_JEV_KEY_FILE="$TMP/typesafe-api-key"
  export FAKE_RUN_DIR="$TMP/local-key-a1"
  export FAKE_ENGINE_LOG="$TMP/local-key-engines.log"
  export FAKE_JEV_LOG="$TMP/local-key-jev.log"
  export FAKE_JEV_RESPONSE='{"version":1,"status":"fallback","stage":"route","mode":"shadow","reason":"low_confidence"}'
  export LOOP_JEV_CLIENT="$HERE/fake/loop-jev.mjs"
  export FAKE_CODEX_OUTCOME=done
  mkdir -p "$FAKE_RUN_DIR"
  zsh "$ROOT/loop-runner.sh" \
    --worktree "$ROOT" --run-dir "$FAKE_RUN_DIR" --prompt-file "$TMP/prompt.md" \
    --models-conf "$TMP/legacy-models.conf" --run-id local-key --phase local-key \
    --repository rstagi/dotfile --chain task --timeout 5 > "$TMP/local-key.out" 2>&1
  assert_exit "$?" "0" "local key reaches the engine"
  assert_eq "$(jq -r '.mode' "$FAKE_RUN_DIR/route-decision.json")" "shadow" "local key chooses shadow mode"
)

cat > "$TMP/route-prompt.md" <<'EOF'
You are a loop-engineering task runner.
PHASE (from the shared plan):
### Phase 2 — Cache small reads [lane: A] [status: todo]
- **Done when:** Cached reads return the same value; rotate postgres://admin:hunter2@db/app and API_TOKEN = private-value-123 before release.
- **Estimate:** 10–15 min · high confidence · one module
CONTEXT: The plan goal and steering notes follow.
EOF

echo "runner: inherited implicit off retains missing-credentials reason"
mkdir -p "$TMP/implicit-a1"
(
  unset TYPESAFE_API_KEY LOOP_JEV_MODE LOOP_JEV_MODE_EXPLICIT LOOP_JEV_CLIENT
  source "$ROOT/loop-models.conf"
  export FAKE_RUN_DIR="$TMP/implicit-a1"
  export FAKE_ENGINE_LOG="$TMP/implicit-engines.log"
  export FAKE_CODEX_OUTCOME=done
  zsh "$ROOT/loop-runner.sh" \
    --worktree "$ROOT" \
    --run-dir "$FAKE_RUN_DIR" \
    --prompt-file "$TMP/route-prompt.md" \
    --run-id implicit --phase 1 --repository rstagi/dotfile \
    --chain task --timeout 5 > "$TMP/implicit.out" 2>&1
)
RC=$?
assert_exit "$RC" "0" "inherited off reaches the engine"
assert_eq "$(jq -r '.fallbackReason' "$TMP/implicit-a1/route-decision.json")" "missing_credentials" "persists missing credentials after double source"

run_routed_task() {
  local name="$1" response="$2" mode="$3" prompt="${4:-$TMP/route-prompt.md}"
  RUN_DIR="$TMP/$name-a1"
  mkdir -p "$RUN_DIR"
  export FAKE_RUN_DIR="$RUN_DIR"
  export FAKE_ENGINE_LOG="$TMP/$name-engines.log"
  export FAKE_JEV_LOG="$TMP/$name-jev.log"
  export FAKE_JEV_RESPONSE="$response"
  export LOOP_JEV_MODE="$mode"
  export LOOP_JEV_CLIENT="$HERE/fake/loop-jev.mjs"
  export FAKE_CODEX_OUTCOME=done
  export FAKE_LEG_SNAPSHOT="$TMP/leg-seen.json"
  rm -f "$FAKE_LEG_SNAPSHOT"
  : > "$FAKE_ENGINE_LOG"
  : > "$FAKE_JEV_LOG"
  zsh "$ROOT/loop-runner.sh" \
    --worktree "$ROOT" \
    --run-dir "$RUN_DIR" \
    --prompt-file "$prompt" \
    --chain task \
    --run-id loop-route \
    --phase "$name" \
    --repository rstagi/dotfile \
    --timeout 5 > "$TMP/$name.out" 2>&1
  RC=$?
  INVOCATIONS="$(cat "$FAKE_ENGINE_LOG")"
}

echo "runner: active high-confidence route uses configured light profile"
run_routed_task active '{"version":1,"status":"ok","stage":"route","mode":"active","model":"jev-test","answers":{"profile":{"type":"choice","choice":"light","probabilities":{"default":0.05,"light":0.95},"confidence":0.95}},"confidence":0.95,"usage":{"inputTokens":5,"outputTokens":1}}' active
assert_exit "$RC" "0" "active route completes"
assert_contains "$INVOCATIONS" '-m gpt-5.6-terra' "uses named light chain"
assert_eq "$(jq -r '.engine + ":" + .model' "$FAKE_LEG_SNAPSHOT" 2>/dev/null)" "codex:gpt-5.6-terra" "leg.json names the running leg before the attempt ends"
assert_eq "$(jq -r '.candidate + ":" + .appliedAction' "$RUN_DIR/route-decision.json")" "light:light" "persists proposed and actual profiles"
assert_eq "$(jq -r '.proposedProfile + ":" + .actualProfile' "$RUN_DIR/meta.json")" "light:light" "copies profiles into attempt metadata"
assert_eq "$(wc -l < "$FAKE_JEV_LOG" | tr -d ' ')" "1" "calls Jev once before spawn"
assert_eq "$(jq -r '.stage + ":" + .state.phase + ":" + .state.repository' "$FAKE_JEV_LOG")" "route:active:rstagi/dotfile" "sends bounded route context"
assert_eq "$(jq -r '.state.task.title' "$FAKE_JEV_LOG")" "Cache small reads" "sends phase title as task evidence"
assert_contains "$(jq -r '.state.task.doneWhen' "$FAKE_JEV_LOG")" "Cached reads return the same value" "sends acceptance criteria"
assert_eq "$(jq -r '.state.task.estimate' "$FAKE_JEV_LOG")" "10–15 min · high confidence · one module" "sends phase estimate"
assert_eq "$(rg -c 'hunter2' "$FAKE_JEV_LOG" 2>/dev/null || echo 0)" "0" "redacts credentials from route evidence"
assert_eq "$(rg -c 'private-value-123' "$FAKE_JEV_LOG" 2>/dev/null || echo 0)" "0" "redacts spaced credential assignments"
assert_eq "$(jq -r '(.state.task | tostring | length) < 1200' "$FAKE_JEV_LOG")" "true" "bounds task evidence"

echo "runner: active route without task evidence keeps default"
run_routed_task no-evidence '{"version":1,"status":"ok","stage":"route","mode":"active","model":"jev-test","answers":{"profile":{"type":"choice","choice":"light","probabilities":{"default":0.05,"light":0.95},"confidence":0.95}},"confidence":0.95}' active "$TMP/prompt.md"
assert_exit "$RC" "0" "runs even without a phase block"
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "keeps default chain without task evidence"
assert_eq "$(jq -r '.fallbackReason' "$RUN_DIR/route-decision.json")" "missing-task-evidence" "records why light advice was rejected"

echo "runner: shadow route advises light but keeps default profile"
run_routed_task shadow '{"version":1,"status":"ok","stage":"route","mode":"shadow","model":"jev-test","answers":{"profile":{"type":"choice","choice":"light","probabilities":{"default":0.1,"light":0.9},"confidence":0.9}},"confidence":0.9,"usage":{"inputTokens":5,"outputTokens":1}}' shadow
assert_exit "$RC" "0" "shadow route completes"
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "keeps default task chain"
assert_eq "$(jq -r '.candidate + ":" + .appliedAction + ":" + .fallbackReason' "$RUN_DIR/route-decision.json")" "light:default:shadow-mode" "records advice without applying it"

echo "runner: fallback route keeps default and retry/resume reuses the decision"
run_routed_task reuse '{"version":1,"status":"fallback","stage":"route","mode":"active","reason":"low_confidence","model":"jev-test","answers":{"profile":{"type":"choice","choice":"light","probabilities":{"default":0.4,"light":0.6},"confidence":0.6}},"confidence":0.6}' active
assert_exit "$RC" "0" "fallback route completes"
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "fallback uses default task chain"
RUN_DIR="$TMP/reuse-a2"
mkdir -p "$RUN_DIR"
export FAKE_RUN_DIR="$RUN_DIR"
zsh "$ROOT/loop-runner.sh" \
  --worktree "$ROOT" \
  --run-dir "$RUN_DIR" \
  --prompt-file "$TMP/prompt.md" \
  --chain task \
  --resume route-session \
  --engine codex \
  --run-id loop-route \
  --phase reuse \
  --repository rstagi/dotfile \
  --timeout 5 > "$TMP/reuse-resume.out" 2>&1
RC=$?
assert_exit "$RC" "0" "resume completes"
assert_eq "$(wc -l < "$FAKE_JEV_LOG" | tr -d ' ')" "1" "resume does not duplicate the Jev call"
assert_eq "$(jq -r '.fallbackReason + ":" + .appliedAction' "$RUN_DIR/route-decision.json")" "low_confidence:default" "persists fallback and actual profile"
assert_eq "$(jq -r '.attempt' "$RUN_DIR/route-decision.json")" "2" "correlates reused advice to the resumed attempt"
assert_eq "$([[ -e "$TMP/state.json" ]] && echo yes || echo no)" "no" "routing does not mutate phase state"

echo "runner: missing credentials, API errors, and unsupported profiles use default"
run_routed_task missing '{"version":1,"status":"fallback","stage":"route","mode":"active","reason":"missing_credentials"}' active
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "missing credentials use default"
assert_eq "$(jq -r '.fallbackReason' "$RUN_DIR/route-decision.json")" "missing_credentials" "records missing credentials"
run_routed_task apierror '{"version":1,"status":"fallback","stage":"route","mode":"active","reason":"api_error"}' active
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "API errors use default"
assert_eq "$(jq -r '.fallbackReason' "$RUN_DIR/route-decision.json")" "api_error" "records API error"
run_routed_task unsupported '{"version":1,"status":"ok","stage":"route","mode":"active","model":"jev-test","answers":{"profile":{"type":"choice","choice":"arbitrary-model","probabilities":{"arbitrary-model":0.99},"confidence":0.99}},"confidence":0.99}' active
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "unsupported profile uses default"
assert_eq "$(jq -r '.fallbackReason + ":" + .appliedAction' "$RUN_DIR/route-decision.json")" "unsupported-profile:default" "records rejected profile"

unset LOOP_JEV_MODE LOOP_JEV_CLIENT FAKE_JEV_LOG FAKE_JEV_RESPONSE FAKE_CODEX_OUTCOME
export FAKE_ENGINE_LOG="$TMP/engines.log"
export FAKE_RUN_DIR="$RUN_DIR"

# --- checkpoint readiness: observe session startup before awaiting the watchdog ---
echo "runner: phase deadline returns a checkpoint to the orchestrator"
mkdir -p "$TMP/slow-bin" "$TMP/checkpoint"
cat > "$TMP/slow-bin/codex" <<'EOF'
#!/bin/zsh
# Simulate slow model discovery before the engine emits its session.
[[ "$1" == debug ]] && { sleep 1; print -r -- '{"models":[{"slug":"gpt-6-sol","visibility":"list"}]}'; exit 0; }
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
  --timeout 5 > "$TMP/checkpoint-runner.out" 2>&1 &
checkpoint_pid=$!
checkpoint_ready=0
for checkpoint_poll in {1..60}; do
  if rg -qF '"thread_id":"checkpoint-session"' "$TMP/checkpoint/transcript.jsonl" 2>/dev/null; then
    checkpoint_ready=1
    break
  fi
  kill -0 "$checkpoint_pid" 2>/dev/null || break
  sleep 0.05
done
assert_eq "$checkpoint_ready" "1" "checkpoint engine publishes its session before cutoff"
wait "$checkpoint_pid"
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
# --- end checkpoint readiness ---

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

run_review() { # run_review <chain> [codex outcome] [extra runner args...]
  local chain="$1" codex_outcome="${2:-rate-limit}"
  shift; (( $# )) && shift
  RUN_DIR="$TMP/$chain-$RANDOM"
  mkdir -p "$RUN_DIR"
  export FAKE_RUN_DIR="$RUN_DIR"
  export FAKE_CODEX_OUTCOME="$codex_outcome"
  : > "$FAKE_ENGINE_LOG"
  zsh "$ROOT/loop-runner.sh" \
    --worktree "$ROOT" \
    --run-dir "$RUN_DIR" \
    --prompt-file "$TMP/prompt.md" \
    --chain "$chain" \
    --timeout 5 "$@" > "$RUN_DIR.out" 2>&1
  RC=$?
  INVOCATIONS="$(cat "$FAKE_ENGINE_LOG")"
}

echo "runner: review tiers pick per-role chains and efforts (default tier: standard)"
run_review review-gpt done
assert_exit "$RC" "0" "standard GPT reviewer completes"
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "standard GPT reviewer is Sol"
assert_contains "$INVOCATIONS" 'model_reasoning_effort="xhigh"' "standard GPT reviewer runs at xhigh"
assert_eq "$(jq -r '.effort' "$RUN_DIR/meta.json")" "xhigh" "meta records the effort passed"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "reviewer has no fallback reviewer"

run_review review-claude
assert_exit "$RC" "0" "standard Claude reviewer completes"
assert_contains "$INVOCATIONS" 'claude -p --model opus ' "standard Claude reviewer is Opus"
assert_contains "$INVOCATIONS" '--effort xhigh' "standard Claude reviewer runs at xhigh"

run_review review-grok
assert_exit "$RC" "0" "standard Grok reviewer completes"
assert_contains "$INVOCATIONS" 'grok --prompt-file ' "standard Grok reviewer is Grok (headless, prompt from file)"
assert_contains "$INVOCATIONS" '--always-approve' "grok skips approvals"
assert_contains "$INVOCATIONS" '--output-format streaming-messages-json' "grok streams a claude-shaped transcript"
assert_contains "$INVOCATIONS" '--effort xhigh' "standard Grok reviewer runs at xhigh"
assert_eq "$([[ "$INVOCATIONS" == *' -m '* ]] && echo yes || echo no)" "no" "grok:default runs the CLI's default (latest) model"
assert_eq "$(jq -r '.engine' "$RUN_DIR/meta.json")" "grok" "meta records the grok engine"
assert_eq "$(jq -r '.model' "$RUN_DIR/meta.json")" "grok-4.7" "meta records the concrete model from the init event"
assert_eq "$(jq -r '.sessionId' "$RUN_DIR/meta.json")" "s-grok" "meta keeps the grok session for resume"
assert_eq "$(cat "$RUN_DIR/last.md")" "grok done" "final message extracted to last.md"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "Grok reviewer has no fallback reviewer"

run_review review-fix
assert_exit "$RC" "0" "standard remediation coordinator completes"
assert_contains "$INVOCATIONS" 'claude -p --model opus ' "standard fixes run on Opus"
assert_contains "$INVOCATIONS" '--effort medium' "fixes run at medium effort"
assert_eq "$([[ "$INVOCATIONS" == *'--agents'* ]] && echo yes || echo no)" "no" "remediation delegation belongs to the skill"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "remediation coordinator has no model fallback"

run_review review-final
assert_exit "$RC" "0" "standard verdict completes"
assert_contains "$INVOCATIONS" 'claude -p --model opus ' "verdict runs on Opus"
assert_contains "$INVOCATIONS" '--effort xhigh' "standard verdict runs at xhigh"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "final reviewer has no model fallback"

echo "runner: max tier — Astra keeps codex ultra, Fable at max, verdict at max"
run_review review-gpt done --review-tier max
assert_exit "$RC" "0" "max review-gpt completes"
assert_contains "$INVOCATIONS" '-m gpt-6-astra' "max GPT reviewer is Astra"
assert_contains "$INVOCATIONS" 'model_reasoning_effort="ultra"' "codex keeps ultra"
run_review review-claude rate-limit --review-tier max
assert_exit "$RC" "0" "max review-claude completes"
assert_contains "$INVOCATIONS" 'claude -p --model fable ' "max Claude reviewer is Fable"
assert_contains "$INVOCATIONS" '--effort max' "Fable runs at max"
run_review review-final rate-limit --review-tier max
assert_exit "$RC" "0" "max review-final completes"
assert_contains "$INVOCATIONS" '--effort max' "max verdict runs at max"
run_review review-grok rate-limit --review-tier max
assert_exit "$RC" "0" "max review-grok completes"
assert_contains "$INVOCATIONS" 'grok --prompt-file ' "max Grok reviewer is Grok"
assert_contains "$INVOCATIONS" '--effort xhigh' "grok has no max: clamped to xhigh"
assert_eq "$(jq -r '.effort' "$RUN_DIR/meta.json")" "xhigh" "meta records the clamped effort"

run_review review-fix rate-limit --review-tier max
assert_exit "$RC" "0" "max remediation coordinator completes"
assert_contains "$INVOCATIONS" 'claude -p --model opus ' "max fixes run on Opus"
assert_contains "$INVOCATIONS" '--effort medium' "max fixes run at medium effort"

run_review review-gpt done --review-tier standard
assert_exit "$RC" "0" "explicit standard tier completes"
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "explicit standard GPT reviewer is Sol"

for tier in shallow medium huge; do
  run_review review-gpt done --review-tier "$tier"
  assert_exit "$RC" "1" "$tier tier is a usage error"
  assert_contains "$(cat "$RUN_DIR.out")" "review tier '$tier'" "identifies rejected tier $tier"
  assert_contains "$(cat "$RUN_DIR.out")" 'standard|max' "lists supported tiers for $tier"
  assert_eq "$INVOCATIONS" "" "rejected tier $tier never starts an engine"
done

echo "runner: review configuration defaults"
review_defaults="$(
  source "$ROOT/loop-models.conf"
  print -r -- "$LOOP_REVIEW_TIER_DEFAULT:$LOOP_REVIEW_ROUNDS_DEFAULT:$LOOP_REVIEW_INNER_DEFAULT:$LOOP_REVIEW_ROUNDS_CAP:${LOOP_PLAN_REVIEW_ROUNDS:-unset}"
)"
assert_eq "$review_defaults" "standard:1:2:5:1" "shared review defaults apply to both tiers and plan review"
legacy_rounds="$(
  unset LOOP_REVIEW_ROUNDS_SHALLOW LOOP_REVIEW_ROUNDS_MEDIUM LOOP_REVIEW_ROUNDS_MAX
  source "$ROOT/loop-models.conf"
  print -r -- "${LOOP_REVIEW_ROUNDS_SHALLOW-unset}:${LOOP_REVIEW_ROUNDS_MEDIUM-unset}:${LOOP_REVIEW_ROUNDS_MAX-unset}"
)"
assert_eq "$legacy_rounds" "unset:unset:unset" "per-tier round defaults are removed"

echo "runner: plan review — Sol + Opus + Grok at high, no fallback"
run_review plan-review-gpt done
assert_exit "$RC" "0" "plan GPT reviewer completes"
assert_contains "$INVOCATIONS" '-m gpt-6-sol' "plan GPT reviewer is Sol"
assert_contains "$INVOCATIONS" 'model_reasoning_effort="high"' "plan GPT reviewer runs at high"
assert_eq "$(print -r -- "$INVOCATIONS" | wc -l | tr -d ' ')" "1" "plan GPT reviewer has no fallback"
run_review plan-review-claude
assert_exit "$RC" "0" "plan Claude reviewer completes"
assert_contains "$INVOCATIONS" 'claude -p --model opus ' "plan Claude reviewer is Opus"
assert_contains "$INVOCATIONS" '--effort high' "plan Claude reviewer runs at high"
run_review plan-review-grok
assert_exit "$RC" "0" "plan Grok reviewer completes"
assert_contains "$INVOCATIONS" 'grok --prompt-file ' "plan Grok reviewer is Grok"
assert_contains "$INVOCATIONS" '--effort high' "plan Grok reviewer runs at high"
run_review plan-review-d
assert_exit "$RC" "1" "unknown plan reviewer is a usage error"

echo "runner: removed chain names are usage errors"
for chain in review-adv-a review-adv-b review-adv-c review-astra review-fable plan-review-a plan-review-b plan-review-c; do
  run_review "$chain" done
  assert_exit "$RC" "1" "removed $chain is rejected"
  assert_contains "$(cat "$RUN_DIR.out")" "removed chain '$chain'" "identifies removed $chain"
  assert_contains "$(cat "$RUN_DIR.out")" 'use ' "suggests replacement for $chain"
  assert_eq "$INVOCATIONS" "" "removed $chain never starts an engine"
done

echo "runner: legacy tier flags are rejected for every chain"
for chain in task plan-review-gpt; do
  for tier in shallow medium; do
    run_review "$chain" done --review-tier "$tier"
    assert_exit "$RC" "1" "$chain rejects explicit $tier tier"
    assert_contains "$(cat "$RUN_DIR.out")" "review tier '$tier'" "$chain identifies rejected $tier tier"
    assert_eq "$INVOCATIONS" "" "$chain never starts an engine with $tier tier"
  done
done

echo "runner: a claude leg configured at ultra is clamped to max"
{ cat "$ROOT/loop-models.conf"; print -r -- 'CHAIN_TASK=("claude:opus")'; print -r -- 'EFFORT_TASK=ultra'; } > "$TMP/ultra-models.conf"
run_review task rate-limit --models-conf "$TMP/ultra-models.conf"
assert_contains "$INVOCATIONS" '--effort max' "claude ultra → --effort max"
assert_eq "$(jq -r '.effort' "$RUN_DIR/meta.json")" "max" "meta records the clamped effort"
unset FAKE_CODEX_OUTCOME

# --- loop-top controls: pause, live note, model override (files under <loopDir>/control) ---
export LOOP_CONTROL_POLL_SEC=0.2

# ctl_run <name> [extra args...] — RUN_DIR under a .loop layout so loopDir = RUN_DIR:h:h
ctl_run() {
  local name="$1"; shift
  CTL="$TMP/ctl-$name/.loop"
  RUN_DIR="$CTL/runs/p1-a1"
  mkdir -p "$RUN_DIR" "$CTL/control" "$CTL/notes"
  export FAKE_RUN_DIR="$RUN_DIR"
  export FAKE_ENGINE_LOG="$TMP/ctl-$name.log"
  : > "$FAKE_ENGINE_LOG"
}
ctl_exec() {
  local start=$SECONDS
  zsh "$ROOT/loop-runner.sh" --worktree "$ROOT" --run-dir "$RUN_DIR" --prompt-file "$TMP/prompt.md" \
    --chain task --phase 1 --timeout 30 "$@" > "$RUN_DIR.out" 2>&1
  RC=$?
  ELAPSED=$(( SECONDS - start ))
  INVOCATIONS="$(cat "$FAKE_ENGINE_LOG")"
}

echo "runner: pause file mid-run halts the engine and exits 30 with a resumable session"
ctl_run pause-mid
export FAKE_CODEX_OUTCOME=done FAKE_CODEX_SLEEP=10 FAKE_CODEX_SESSION=sess-pause
( sleep 1; touch "$CTL/control/pause-1" ) &
ctl_exec
assert_exit "$RC" "30" "paused attempt exits 30"
assert_eq "$(( ELAPSED < 6 ))" "1" "halts within the poll window, not after the engine finishes (${ELAPSED}s)"
assert_eq "$(jq -r '.engineExit' "$RUN_DIR/meta.json")" "30" "meta records exit 30"
assert_eq "$(jq -r '.sessionId' "$RUN_DIR/meta.json")" "sess-pause" "meta keeps the session id for --resume"
assert_eq "$([[ -f "$RUN_DIR/status.json" ]] && echo yes || echo no)" "no" "no status.json from the killed engine"
unset FAKE_CODEX_SLEEP FAKE_CODEX_SESSION

echo "runner: loop-wide pause before spawn exits 30 without starting an engine"
ctl_run pause-before
touch "$CTL/control/pause"
ctl_exec
assert_exit "$RC" "30" "paused before start exits 30"
assert_eq "$INVOCATIONS" "" "no engine was spawned"
assert_eq "$(jq -r '.engineExit' "$RUN_DIR/meta.json")" "30" "meta records exit 30"
assert_eq "$(jq -r '.sessionId' "$RUN_DIR/meta.json")" "" "no session to resume"

echo "runner: editing the phase note mid-run resumes the same session with the note"
ctl_run note-live
print -r -- "old guidance" > "$CTL/notes/1.md"
export FAKE_CODEX_OUTCOME=done FAKE_CODEX_SLEEP=10 FAKE_CODEX_SESSION=sess-note
( sleep 1; print -r -- "focus on the cache tests" > "$CTL/notes/1.md" ) &
ctl_exec
assert_exit "$RC" "0" "attempt still completes"
assert_eq "$(( ELAPSED < 6 ))" "1" "the stalled run was interrupted (${ELAPSED}s)"
assert_eq "$(print -r -- "$INVOCATIONS" | grep -c '^codex ')" "2" "fresh run + one resume"
assert_contains "$INVOCATIONS" 'codex exec resume sess-note' "resumes the captured session"
assert_contains "$INVOCATIONS" 'Steering note updated by the user (notes/1.md)' "sends the note header"
assert_contains "$INVOCATIONS" 'focus on the cache tests' "sends the new note content"
assert_eq "$(jq -r '.sessionId' "$RUN_DIR/meta.json")" "sess-note" "meta keeps the session id"
unset FAKE_CODEX_SLEEP FAKE_CODEX_SESSION

echo "runner: an AR reviewer note edit restarts a fresh call, never resuming the session"
ctl_run note-ar
print -r -- "old guidance" > "$CTL/notes/1.md"
export FAKE_CODEX_OUTCOME=done FAKE_CODEX_SLEEP=10 FAKE_CODEX_SESSION=ar-must-be-fresh
( sleep 1; print -r -- "check the cache tests" > "$CTL/notes/1.md" ) &
ctl_exec --chain review-gpt --review-tier standard
assert_exit "$RC" "0" "AR attempt completes"
assert_eq "$(print -r -- "$INVOCATIONS" | grep -c '^codex exec --json ')" "2" "two fresh calls"
assert_eq "$(print -r -- "$INVOCATIONS" | grep -c 'exec resume')" "0" "never resumes the reviewer session"
assert_contains "$(cat "$RUN_DIR/note-1.md")" 'implement phase' "restart carries the full prompt"
assert_contains "$(cat "$RUN_DIR/note-1.md")" 'check the cache tests' "restart carries the updated note"
assert_eq "$(jq -r '.sessionId' "$RUN_DIR/meta.json")" "ar-must-be-fresh" "meta records the fresh call's session"
unset FAKE_CODEX_SLEEP FAKE_CODEX_SESSION

echo "runner: deleting an existing note mid-run tells the agent it was removed"
ctl_run note-removed
print -r -- "temporary guidance" > "$CTL/notes/1.md"
export FAKE_CODEX_OUTCOME=done FAKE_CODEX_SLEEP=10 FAKE_CODEX_SESSION=sess-rm
( sleep 1; rm -f "$CTL/notes/1.md" ) &
ctl_exec
assert_exit "$RC" "0" "attempt completes after removal"
assert_contains "$INVOCATIONS" 'note removed' "says the note was removed"
unset FAKE_CODEX_SLEEP FAKE_CODEX_SESSION

echo "runner: deleting a note mid AR reviewer call restarts fresh and says it was removed"
ctl_run note-ar-removed
print -r -- "temporary guidance" > "$CTL/notes/1.md"
export FAKE_CODEX_OUTCOME=done FAKE_CODEX_SLEEP=10 FAKE_CODEX_SESSION=ar-rm-fresh
( sleep 1; rm -f "$CTL/notes/1.md" ) &
ctl_exec --chain review-gpt --review-tier standard
assert_exit "$RC" "0" "AR attempt completes after removal"
assert_eq "$(print -r -- "$INVOCATIONS" | grep -c 'exec resume')" "0" "never resumes the reviewer session"
assert_contains "$(cat "$RUN_DIR/note-1.md")" 'implement phase' "restart carries the full prompt"
assert_contains "$(cat "$RUN_DIR/note-1.md")" 'note removed' "restart says the note was removed"
assert_contains "$(cat "$RUN_DIR/note-1.md")" 'Disregard the Repository steering notes section' "restart voids the embedded note"
unset FAKE_CODEX_SLEEP FAKE_CODEX_SESSION

echo "runner: control/model-<phase> overrides the first leg of the task chain"
ctl_run model-override
print -r -- "claude:claude-opus-5-5" > "$CTL/control/model-1"
export FAKE_CODEX_OUTCOME=done
ctl_exec
assert_exit "$RC" "0" "override run completes"
assert_contains "$(print -r -- "$INVOCATIONS" | head -1)" "claude -p --model claude-opus-5-5 " "first leg is the override"
assert_eq "$(jq -r '.modelOverride' "$RUN_DIR/meta.json")" "claude:claude-opus-5-5" "meta records the override"
assert_eq "$(jq -r '.routeFallbackReason' "$RUN_DIR/meta.json")" "user-override" "route decision skipped as user-override"

echo "runner: an invalid model override is ignored with a warning"
ctl_run model-bogus
print -r -- "gpt; rm -rf /" > "$CTL/control/model-1"
ctl_exec
assert_exit "$RC" "0" "default chain still runs"
assert_contains "$(print -r -- "$INVOCATIONS" | head -1)" "codex exec --json -C $ROOT -m gpt-6-sol" "first leg stays the default Codex leg"
assert_contains "$(cat "$RUN_DIR.out")" "ignoring invalid model override" "warns about the bad override"
assert_eq "$(jq -r '.modelOverride' "$RUN_DIR/meta.json")" "null" "no override recorded"
unset FAKE_CODEX_OUTCOME

# --- "latest of a family" legs: codex:@<family> resolves from `codex debug models`; claude
# aliases (opus/sonnet/fable) are native — meta.json records the model that actually ran ---
export FAKE_CODEX_MODELS='{"models":[{"slug":"gpt-6-sol","visibility":"list"},{"slug":"gpt-6.1-sol","visibility":"list"},{"slug":"gpt-5.6-sol","visibility":"list"},{"slug":"gpt-7-sol","visibility":"hide"},{"slug":"gpt-6-astra","visibility":"list"}]}'

echo "runner: codex:@sol runs the newest listed sol model and records it"
ctl_run family-sol
print -r -- "codex:@sol" > "$CTL/control/model-1"
export FAKE_CODEX_OUTCOME=done
ctl_exec
assert_exit "$RC" "0" "family leg completes"
assert_contains "$(print -r -- "$INVOCATIONS" | grep 'codex exec' | head -1)" "-m gpt-6.1-sol " "resolves @sol to the newest listed sol (hidden gpt-7-sol ignored)"
assert_eq "$(jq -r '.model' "$RUN_DIR/meta.json")" "gpt-6.1-sol" "meta records the resolved model"

echo "runner: an unknown codex family skips that leg and falls through the chain"
ctl_run family-unknown
print -r -- "codex:@nova" > "$CTL/control/model-1"
ctl_exec
assert_exit "$RC" "0" "falls through to the next leg"
assert_contains "$(cat "$RUN_DIR.out")" "no codex model for family @nova" "warns about the unresolvable family"
assert_eq "$(print -r -- "$INVOCATIONS" | grep -c -- '-m @nova')" "0" "never passes the raw @family to codex"
unset FAKE_CODEX_OUTCOME

echo "runner: a claude alias records the concrete model from the transcript init event"
ctl_run claude-alias
print -r -- "claude:opus" > "$CTL/control/model-1"
FAKE_CLAUDE_MODEL=claude-opus-5-5 ctl_exec
assert_exit "$RC" "0" "alias leg completes"
assert_contains "$(print -r -- "$INVOCATIONS" | head -1)" "claude -p --model opus " "alias passed through to claude"
assert_eq "$(jq -r '.model' "$RUN_DIR/meta.json")" "claude-opus-5-5" "meta records the concrete model"
unset FAKE_CODEX_MODELS

echo "runner: a grok session resumes with --engine grok"
run_review review-grok rate-limit --resume s-grok --engine grok
assert_exit "$RC" "0" "grok resume completes"
assert_contains "$INVOCATIONS" '--resume s-grok' "resumes the grok session by id"

echo "runner: a pinned grok model is passed with -m"
{ cat "$ROOT/loop-models.conf"; print -r -- 'CHAIN_TASK=("grok:grok-4.6")'; } > "$TMP/grok-models.conf"
run_review task rate-limit --models-conf "$TMP/grok-models.conf"
assert_contains "$(print -r -- "$INVOCATIONS" | grep '^grok ')" '-m grok-4.6' "pinned grok model reaches the grok CLI"

echo "runner: a claude plan-limit reply is a usage limit, not a finished leg"
for limit_rc in 0 1; do
  mkdir -p "$TMP/limit-$limit_rc"
  FAKE_RUN_DIR="$TMP/limit-$limit_rc" FAKE_ENGINE_LOG="$TMP/limit-$limit_rc.log" \
    FAKE_CLAUDE_OUTCOME=session-limit FAKE_CLAUDE_LIMIT_RC=$limit_rc \
    zsh "$ROOT/loop-runner.sh" \
      --worktree "$ROOT" \
      --run-dir "$TMP/limit-$limit_rc" \
      --prompt-file "$TMP/prompt.md" \
      --chain task \
      --timeout 5 > "$TMP/limit-$limit_rc.out" 2>&1
  RC=$?
  assert_exit "$RC" "40" "exhausts the chain instead of succeeding (claude rc=$limit_rc)"
  assert_contains "$(cat "$TMP/limit-$limit_rc.out")" "rate/usage limit on claude" "classifies the reply as a usage limit (claude rc=$limit_rc)"
  assert_eq "$(grep -c '^claude ' "$TMP/limit-$limit_rc.log")" "1" "does not retry the limited leg (claude rc=$limit_rc)"
done

# --- suite environment isolation: caller settings never reach suite fixtures ---
echo "suite: ignores inherited loop and color settings"
mkdir -p "$TMP/suite/tests"
cp "$HERE/run.sh" "$TMP/suite/tests/run.sh"
cat > "$TMP/suite/tests/environment.test.sh" <<'EOF'
#!/bin/zsh
for inherited in ${(k)parameters}; do
  case "$inherited" in
    LOOP_*|WS_*|CLICOLOR*|FORCE_COLOR|NO_COLOR|TYPESAFE_API_KEY) exit 1 ;;
  esac
done
[[ "${SUITE_SENTINEL:-}" == preserved ]]
EOF
env LOOP_JEV_MODE=shadow LOOP_JEV_MODE_EXPLICIT=0 LOOP_JEV_CUSTOM=inherited \
  LOOP_SUB=1 LOOP_SUB_DIR=/unused LOOP_SUB_CUSTOM=inherited LOOP_CONTROL_POLL_SEC=99 \
  LOOP_DAEMON_URL=http://127.0.0.1:9 CLICOLOR=1 CLICOLOR_FORCE=1 CLICOLOR_CUSTOM=1 \
  WS_SESSION=1 WS_WORKSPACE=x/y WS_CUSTOM=inherited \
  FORCE_COLOR=1 NO_COLOR=1 TYPESAFE_API_KEY=fake-key SUITE_SENTINEL=preserved \
  zsh "$TMP/suite/tests/run.sh" > "$TMP/suite.out" 2>&1
assert_exit "$?" "0" "suite strips loop, ws and color settings while preserving unrelated variables"
# --- end suite environment isolation ---

test_summary
