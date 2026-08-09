#!/bin/zsh
set -u -o pipefail

# Loop Engineering — run ONE phase attempt headlessly with a model-fallback chain.
# Mechanism only: spawn engine, watchdog, classify API failures, validate the
# runner's status.json, run the verify command. The orchestrator switches on the
# exit code (see .claude/skills/loop-execute/references/loop-protocol.md).
#
# Usage:
#   loop-runner.sh --worktree <path> --run-dir <abs path> --prompt-file <f>
#     [--chain task|escalate|review] [--timeout <s>] [--verify-cmd <cmd>]
#     [--resume <sessionId> --engine codex|claude] [--models-conf <f>] [--budget <usd>]
#
# Exit: 0 done+verified · 10 question · 12 verify failed · 20 blocked
#       40 chain exhausted (API) · 50 crash (no valid status.json) · 124 timeout · 1 usage

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Best-effort daemon emission (sourced; no-op if loop-emit.sh is missing).
if [[ -r "$SCRIPT_DIR/loop-emit.sh" ]]; then
  source "$SCRIPT_DIR/loop-emit.sh"
else
  loop_emit() { :; }
fi

WT="" RUN_DIR="" PROMPT_FILE="" CHAIN_NAME="task" TIMEOUT="" VERIFY=""
RESUME_SID="" RESUME_ENGINE="" MODELS_CONF="$SCRIPT_DIR/loop-models.conf" BUDGET=""
RUN_ID="" PHASE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
  --worktree) WT="$2"; shift 2 ;;
  --run-dir) RUN_DIR="$2"; shift 2 ;;
  --prompt-file) PROMPT_FILE="$2"; shift 2 ;;
  --chain) CHAIN_NAME="$2"; shift 2 ;;
  --timeout) TIMEOUT="$2"; shift 2 ;;
  --verify-cmd) VERIFY="$2"; shift 2 ;;
  --resume) RESUME_SID="$2"; shift 2 ;;
  --engine) RESUME_ENGINE="$2"; shift 2 ;;
  --models-conf) MODELS_CONF="$2"; shift 2 ;;
  --budget) BUDGET="$2"; shift 2 ;;
  --run-id) RUN_ID="$2"; shift 2 ;;
  --phase) PHASE="$2"; shift 2 ;;
  *) echo "loop-runner: unknown arg $1" >&2; exit 1 ;;
  esac
done

# Emit phase.attempt.finish on ANY exit path (map the runner exit code → outcome). The
# daemon's lattice promotes the phase to done on {done,exit0} even if state.json lags.
loop_runner_finish() {
  local rc="$1" outcome
  [[ -n "${RUN_ID:-}" && -n "${PHASE:-}" ]] || return 0
  case "$rc" in
  0) outcome=done ;;
  10) outcome=question ;;
  12) outcome=verify-fail ;;
  20) outcome=blocked ;;
  40) outcome=chain-exhausted ;;
  50) outcome=crash ;;
  124) outcome=timeout ;;
  *) outcome=error ;;
  esac
  jq -cn --arg event phase.attempt.finish --arg phase "$PHASE" --arg outcome "$outcome" \
    --argjson exitCode "$rc" --arg engine "${CUR_ENGINE:-}" --arg model "${CUR_MODEL:-}" \
    --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{event:$event, phase:$phase, outcome:$outcome, exitCode:$exitCode, engine:$engine, model:$model, ts:$ts}' |
    loop_emit "$RUN_ID" event
}
trap 'loop_runner_finish $?' EXIT

[[ -d "$WT" && -n "$RUN_DIR" && -f "$PROMPT_FILE" ]] || {
  echo "loop-runner: --worktree, --run-dir, --prompt-file are required" >&2; exit 1
}
source "$MODELS_CONF" || { echo "loop-runner: cannot source $MODELS_CONF" >&2; exit 1; }
(( ${+CODEX_EXTRA_ARGS} )) || CODEX_EXTRA_ARGS=()
(( ${+CLAUDE_EXTRA_ARGS} )) || CLAUDE_EXTRA_ARGS=()
BUDGET="${BUDGET:-$LOOP_BUDGET_USD}"

case "$CHAIN_NAME" in
task) chain=("${CHAIN_TASK[@]}"); TIMEOUT="${TIMEOUT:-$LOOP_TIMEOUT_TASK}" ;;
escalate) chain=("${CHAIN_ESCALATE[@]}"); TIMEOUT="${TIMEOUT:-$LOOP_TIMEOUT_ESCALATE}" ;;
review) chain=("${CHAIN_REVIEW[@]}"); TIMEOUT="${TIMEOUT:-$LOOP_TIMEOUT_REVIEW}" ;;
*) echo "loop-runner: unknown chain '$CHAIN_NAME'" >&2; exit 1 ;;
esac
[[ "$TIMEOUT" == <-> ]] || { echo "loop-runner: --timeout must be integer seconds" >&2; exit 1; }

mkdir -p "$RUN_DIR"
[[ -n "${RUN_ID:-}" && -n "${PHASE:-}" ]] && jq -cn --arg event phase.attempt.start \
  --arg phase "$PHASE" --arg detail "$CHAIN_NAME" --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{event:$event, phase:$phase, detail:$detail, ts:$ts}' | loop_emit "$RUN_ID" event
[[ "$PROMPT_FILE" -ef "$RUN_DIR/prompt.md" ]] || cp "$PROMPT_FILE" "$RUN_DIR/prompt.md"
TRANSCRIPT="$RUN_DIR/transcript.jsonl" STDERR="$RUN_DIR/stderr.log"
STATUS="$RUN_DIR/status.json" LAST="$RUN_DIR/last.md"
HEAD_BEFORE="$(git -C "$WT" rev-parse HEAD 2>/dev/null || echo unknown)"
CUR_ENGINE="" CUR_MODEL="" TIMED_OUT=0

write_meta() { # $1 = engine exit code
  local sid
  sid="$(jq -r '.session_id // .sessionId // .thread_id // (.msg.session_id? // empty) // empty' \
    "$TRANSCRIPT" 2>/dev/null | head -1)"
  jq -n --arg engine "$CUR_ENGINE" --arg model "$CUR_MODEL" --arg sid "${sid:-}" \
    --arg before "$HEAD_BEFORE" --arg after "$(git -C "$WT" rev-parse HEAD 2>/dev/null || echo unknown)" \
    --argjson rc "${1:-0}" --argjson timedOut "$TIMED_OUT" \
    '{engine: $engine, model: $model, sessionId: $sid, headBefore: $before,
      headAfter: $after, engineExit: $rc, timedOut: ($timedOut == 1)}' > "$RUN_DIR/meta.json"
}

# --- engine launchers (backgrounded by run_leg; cwd/-C = the worktree) ---

launch_claude() { # $1 model, $2 fallback (may be empty)
  local fb_args=()
  [[ -n "$2" ]] && fb_args=(--fallback-model "$2")
  if [[ -n "$RESUME_SID" ]]; then
    ( cd "$WT" && command claude -p --resume "$RESUME_SID" "${fb_args[@]}" "${CLAUDE_EXTRA_ARGS[@]}" \
        --output-format stream-json --verbose \
        --allow-dangerously-skip-permissions --permission-mode bypassPermissions \
        --max-budget-usd "$BUDGET" < "$RUN_DIR/prompt.md" > "$TRANSCRIPT" 2> "$STDERR" )
  else
    ( cd "$WT" && command claude -p --model "$1" "${fb_args[@]}" "${CLAUDE_EXTRA_ARGS[@]}" \
        --output-format stream-json --verbose \
        --allow-dangerously-skip-permissions --permission-mode bypassPermissions \
        --max-budget-usd "$BUDGET" < "$RUN_DIR/prompt.md" > "$TRANSCRIPT" 2> "$STDERR" )
  fi
}

launch_codex() { # $1 model
  # `codex exec resume` has no -C flag and filters sessions by cwd — cd is load-bearing
  if [[ -n "$RESUME_SID" ]]; then
    ( cd "$WT" && codex exec resume "$RESUME_SID" --json "${CODEX_EXTRA_ARGS[@]}" \
        --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check \
        -o "$LAST" "$(cat "$RUN_DIR/prompt.md")" > "$TRANSCRIPT" 2> "$STDERR" )
  else
    codex exec --json -C "$WT" -m "$1" "${CODEX_EXTRA_ARGS[@]}" \
      --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check \
      -o "$LAST" - < "$RUN_DIR/prompt.md" > "$TRANSCRIPT" 2> "$STDERR"
  fi
}

descendants() { # print pids of the full process tree under $1 (depth-first)
  local p
  for p in $(pgrep -P "$1" 2>/dev/null); do
    echo "$p"
    descendants "$p"
  done
}

run_leg() { # runs current engine with watchdog; returns engine exit code (200 = timeout)
  rm -f "$RUN_DIR/.timeout" "$RUN_DIR/.victims"
  if [[ "$CUR_ENGINE" == "claude" ]]; then
    launch_claude "$CUR_MODEL" "$CUR_FALLBACK" &
  else
    launch_codex "$CUR_MODEL" &
  fi
  local child=$!
  # watchdog only marks + TERMs the whole tree; the KILL follow-through happens in the
  # main flow after wait (the watchdog dies with its TERM'd parent otherwise)
  ( sleep "$TIMEOUT"
    kill -0 "$child" 2>/dev/null || exit 0
    { echo "$child"; descendants "$child"; } > "$RUN_DIR/.victims"
    touch "$RUN_DIR/.timeout"
    xargs kill -TERM 2>/dev/null < "$RUN_DIR/.victims"
  ) &
  local watchdog=$!
  wait "$child"; local rc=$?
  kill "$watchdog" 2>/dev/null; wait "$watchdog" 2>/dev/null
  if [[ -f "$RUN_DIR/.timeout" ]]; then
    # engine finished successfully right at the boundary — prefer its real result
    if [[ $rc -eq 0 ]] && jq -e '.outcome' "$STATUS" >/dev/null 2>&1; then
      rm -f "$RUN_DIR/.timeout"
    else
      TIMED_OUT=1
      local grace=0 survivors
      while [[ $grace -lt 30 ]]; do
        survivors="$(xargs -n1 sh -c 'kill -0 "$0" 2>/dev/null && echo "$0"' < "$RUN_DIR/.victims" 2>/dev/null)"
        [[ -z "$survivors" ]] && break
        sleep 5; grace=$((grace + 5))
      done
      [[ -n "${survivors:-}" ]] && echo "$survivors" | xargs kill -KILL 2>/dev/null
      return 200
    fi
  fi
  return $rc
}

# error classification reads stderr + error-shaped transcript events only (never the
# whole transcript — code diffs would false-positive the regexes). Materialized to a
# file: grep -q on a pipe + pipefail returns 141 on match (SIGPIPE upstream).
collect_error_text() {
  { cat "$STDERR" 2>/dev/null
    jq -c 'select((.type == "error") or (.is_error? == true) or (has("error")))' \
      "$TRANSCRIPT" 2>/dev/null | tail -20
  } > "$RUN_DIR/.errtext" 2>/dev/null || true
}
is_rate_limited() { grep -qiE '(^|[^0-9])429([^0-9]|$)|rate.?limit|usage limit|quota exceeded' "$RUN_DIR/.errtext" 2>/dev/null; }
is_transient() { grep -qiE 'overloaded|"5[0-9][0-9]"|status.?5[0-9][0-9]|ECONNRESET|ETIMEDOUT|internal server error' "$RUN_DIR/.errtext" 2>/dev/null; }

# --- chain loop: every attempt starts at the top of the chain ---

if [[ -n "$RESUME_SID" ]]; then
  [[ "$RESUME_ENGINE" == "claude" || "$RESUME_ENGINE" == "codex" ]] || {
    echo "loop-runner: --resume requires --engine claude|codex" >&2; exit 1
  }
  chain=("${RESUME_ENGINE}:resume")
fi

final_rc=40
for leg in "${chain[@]}"; do
  CUR_ENGINE="${leg%%:*}"
  rest="${leg#*:}"
  CUR_MODEL="${rest%%+*}"
  CUR_FALLBACK=""
  [[ "$rest" == *"+"* ]] && CUR_FALLBACK="${rest#*+}"

  retries=0 delay=10 leg_done=0
  while [[ $retries -lt 3 ]]; do
    rm -f "$STATUS"
    run_leg; rc=$?
    if [[ $rc -eq 200 ]]; then
      write_meta 124
      echo "loop-runner: timeout on $leg after ${TIMEOUT}s" >&2
      exit 124
    fi
    if [[ $rc -eq 0 && ( -s "$TRANSCRIPT" || -s "$STATUS" ) ]]; then
      leg_done=1; break
    fi
    collect_error_text
    if is_rate_limited; then
      echo "loop-runner: rate/usage limit on $leg — next leg" >&2
      break
    fi
    if is_transient || [[ $rc -eq 0 ]]; then
      retries=$((retries + 1))
      echo "loop-runner: transient failure on $leg (rc=$rc), retry $retries" >&2
      [[ $retries -lt 3 ]] && { sleep "$delay"; delay=$((delay * 3)); }
      continue
    fi
    # unknown failure: one retry, then next leg
    if [[ $retries -eq 0 ]]; then
      retries=1
      echo "loop-runner: unknown failure on $leg (rc=$rc), one retry" >&2
      sleep "$delay"
      continue
    fi
    echo "loop-runner: giving up on $leg (rc=$rc)" >&2
    break
  done
  [[ $leg_done -eq 1 ]] && { final_rc=0; break; }
done

if [[ $final_rc -eq 40 ]]; then
  write_meta 40
  echo "loop-runner: model chain exhausted" >&2
  exit 40
fi

# claude writes no -o file; extract the final message for humans/post-mortems
if [[ ! -s "$LAST" ]]; then
  jq -r 'select(.type == "result") | .result // empty' "$TRANSCRIPT" 2>/dev/null > "$LAST" || true
fi
write_meta 0

# --- validate the runner's status.json (the actual contract) ---

outcome="$(jq -r '.outcome // empty' "$STATUS" 2>/dev/null)"
case "$outcome" in
question) exit 10 ;;
blocked) exit 20 ;;
done)
  if [[ -n "$VERIFY" ]]; then
    if ! ( cd "$WT" && eval "$VERIFY" ) > "$RUN_DIR/verify.log" 2>&1; then
      echo "loop-runner: claimed done but verify failed (see verify.log)" >&2
      exit 12
    fi
  fi
  exit 0
  ;;
*)
  echo "loop-runner: no valid status.json (outcome='$outcome') — crash" >&2
  exit 50
  ;;
esac
