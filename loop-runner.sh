#!/bin/zsh
set -u -o pipefail

# Loop Engineering — run ONE phase attempt headlessly with a model-fallback chain.
# Mechanism only: spawn engine, watchdog, classify API failures, validate the
# runner's status.json, run the verify command. The orchestrator switches on the
# exit code (see .claude/skills/loop-execute/references/loop-protocol.md).
#
# Usage:
#   loop-runner.sh --worktree <path> --run-dir <abs path> --prompt-file <f>
#     [--chain task|escalate|review-adv-a|review-adv-b|review-fix|review-final]
#     [--review-tier shallow|medium|max]   (review chains; default LOOP_REVIEW_TIER_DEFAULT)
#     [--timeout <s>] [--verify-cmd <cmd>]
#     [--resume <sessionId> --engine codex|claude] [--models-conf <f>] [--budget <usd>]
#     [--loop-dir <d>]   (default: RUN_DIR/../.. — the coordinator .loop/)
#
# Live controls (polled every LOOP_CONTROL_POLL_SEC, default 2s, while the engine runs):
#   <loopDir>/control/pause | control/pause-<phase>  → kill the engine, exit 30 (resumable)
#   <loopDir>/control/model-<phase> (read once, task chain, non-resume) → that leg runs first
#   <loopDir>/notes/<phase>.md changed                → kill the engine, resume the same
#                                                       session with the new note (same attempt)
#
# Exit: 0 done+verified · 10 question · 12 verify failed · 20 blocked · 30 paused
#       40 chain exhausted (API) · 50 crash (no valid status.json) · 1 usage

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Best-effort daemon emission (sourced; no-op if loop-emit.sh is missing).
if [[ -r "$SCRIPT_DIR/loop-emit.sh" ]]; then
  source "$SCRIPT_DIR/loop-emit.sh"
else
  loop_emit() { :; }
  loop_emit_jev_decision() { cat >/dev/null; }
fi

WT="" RUN_DIR="" PROMPT_FILE="" CHAIN_NAME="task" TIMEOUT="" VERIFY=""
RESUME_SID="" RESUME_ENGINE="" MODELS_CONF="$SCRIPT_DIR/loop-models.conf" BUDGET=""
RUN_ID="" PHASE="" REPOSITORY="" LOOP_DIR="" REVIEW_TIER=""
ROUTE_PROPOSED_PROFILE="" ROUTE_ACTUAL_PROFILE="" ROUTE_FALLBACK_REASON=""

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
  --repository) REPOSITORY="$2"; shift 2 ;;
  --loop-dir) LOOP_DIR="$2"; shift 2 ;;
  --review-tier) REVIEW_TIER="$2"; shift 2 ;;
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
  30) outcome=paused ;;
  40) outcome=chain-exhausted ;;
  50) outcome=crash ;;
  124) outcome=timeout ;;
  *) outcome=error ;;
  esac
  jq -cn --arg event phase.attempt.finish --arg phase "$PHASE" --arg outcome "$outcome" \
    --arg repository "$REPOSITORY" \
    --argjson exitCode "$rc" --arg engine "${CUR_ENGINE:-}" --arg model "${CUR_MODEL:-}" \
    --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{event:$event, phase:$phase, repository:(if ($repository|length)>0 then $repository else null end),
      outcome:$outcome, exitCode:$exitCode, engine:$engine, model:$model, ts:$ts}' |
    loop_emit "$RUN_ID" event
}
trap 'loop_runner_finish $?' EXIT

[[ -d "$WT" && -n "$RUN_DIR" && -f "$PROMPT_FILE" ]] || {
  echo "loop-runner: --worktree, --run-dir, --prompt-file are required" >&2; exit 1
}
source "$MODELS_CONF" || { echo "loop-runner: cannot source $MODELS_CONF" >&2; exit 1; }
(( ${+CODEX_EXTRA_ARGS} )) || CODEX_EXTRA_ARGS=()
(( ${+CLAUDE_EXTRA_ARGS} )) || CLAUDE_EXTRA_ARGS=()
LOOP_TASK_DEFAULT_PROFILE="${LOOP_TASK_DEFAULT_PROFILE:-default}"
LOOP_TASK_LIGHT_PROFILE="${LOOP_TASK_LIGHT_PROFILE:-light}"
LOOP_JEV_KEY_FILE="${LOOP_JEV_KEY_FILE:-$SCRIPT_DIR/.loop-secrets/typesafe-api-key}"
if [[ -z "${LOOP_JEV_MODE_EXPLICIT:-}" ]]; then
  [[ -n "${LOOP_JEV_MODE:-}" ]] && LOOP_JEV_MODE_EXPLICIT=1 || LOOP_JEV_MODE_EXPLICIT=0
fi
if [[ -z "${LOOP_JEV_MODE:-}" || "$LOOP_JEV_MODE_EXPLICIT" == 0 ]]; then
  [[ -n "${TYPESAFE_API_KEY:-}" || -s "$LOOP_JEV_KEY_FILE" ]] && LOOP_JEV_MODE=shadow || LOOP_JEV_MODE=off
fi
LOOP_JEV_ROUTE_MIN_CONFIDENCE="${LOOP_JEV_ROUTE_MIN_CONFIDENCE:-0.8}"
export LOOP_JEV_KEY_FILE LOOP_JEV_MODE LOOP_JEV_MODE_EXPLICIT
BUDGET="${BUDGET:-$LOOP_BUDGET_USD}"

# Legacy review chain names (in-flight loops) map onto the max tier's adversaries.
case "$CHAIN_NAME" in
review-astra) CHAIN_NAME=review-adv-a REVIEW_TIER=max ;;
review-fable) CHAIN_NAME=review-adv-b REVIEW_TIER=max ;;
esac

EFFORT=""
case "$CHAIN_NAME" in
task) chain=("${CHAIN_TASK[@]}"); EFFORT="${EFFORT_TASK:-}"; TIMEOUT="${TIMEOUT:-$LOOP_TIMEOUT_TASK}" ;;
escalate) chain=("${CHAIN_ESCALATE[@]}"); EFFORT="${EFFORT_ESCALATE:-}"; TIMEOUT="${TIMEOUT:-$LOOP_TIMEOUT_ESCALATE}" ;;
review-adv-a|review-adv-b|review-fix|review-final)
  REVIEW_TIER="${REVIEW_TIER:-${LOOP_REVIEW_TIER_DEFAULT:-medium}}"
  [[ "$REVIEW_TIER" == (shallow|medium|max) ]] || {
    echo "loop-runner: unknown review tier '$REVIEW_TIER' (shallow|medium|max)" >&2; exit 1
  }
  review_role="${${CHAIN_NAME#review-}//-/_}"   # adv_a | adv_b | fix | final
  review_var="REVIEW_${(U)REVIEW_TIER}_${(U)review_role}"
  chain_var="CHAIN_$review_var" effort_var="EFFORT_$review_var"
  (( ${(P)+chain_var} )) && (( ${#${(P)chain_var}} )) || {
    echo "loop-runner: tier '$REVIEW_TIER' has no ${CHAIN_NAME#review-} reviewer ($chain_var)" >&2; exit 1
  }
  chain=("${(@P)chain_var}")
  EFFORT="${(P)effort_var:-}"
  if [[ "$CHAIN_NAME" == review-fix ]]; then
    TIMEOUT="${TIMEOUT:-$LOOP_TIMEOUT_REMEDIATE}"
  else
    TIMEOUT="${TIMEOUT:-$LOOP_TIMEOUT_REVIEW}"
  fi
  ;;
*) echo "loop-runner: unknown chain '$CHAIN_NAME'" >&2; exit 1 ;;
esac
[[ "$TIMEOUT" == <-> ]] || { echo "loop-runner: --timeout must be integer seconds" >&2; exit 1; }
CHECKPOINT_LIMIT="${LOOP_CHECKPOINT_SEC:-1800}"
[[ "$CHECKPOINT_LIMIT" == <-> ]] || { echo "loop-runner: LOOP_CHECKPOINT_SEC must be integer seconds" >&2; exit 1; }
(( TIMEOUT > CHECKPOINT_LIMIT )) && TIMEOUT="$CHECKPOINT_LIMIT"
CHECKPOINT_MODE=1

mkdir -p "$RUN_DIR"
LOOP_DIR="${LOOP_DIR:-${RUN_DIR:A:h:h}}"
CONTROL_DIR="$LOOP_DIR/control"
CONTROL_POLL="${LOOP_CONTROL_POLL_SEC:-2}"

# User model override (loop-top [m] / daemon POST control): one leg, default chain behind it.
MODEL_OVERRIDE=""
if [[ "$CHAIN_NAME" == task && -n "$PHASE" && -z "$RESUME_SID" && -f "$CONTROL_DIR/model-$PHASE" ]]; then
  override_leg="$(head -n 1 "$CONTROL_DIR/model-$PHASE" | tr -d '[:space:]')"
  if [[ "$override_leg" =~ '^(codex|claude):@?[A-Za-z0-9._-]+(\+[A-Za-z0-9._-]+(,[A-Za-z0-9._-]+)*)?$' ]]; then
    MODEL_OVERRIDE="$override_leg"
    chain=("$MODEL_OVERRIDE" ${CHAIN_TASK:#$MODEL_OVERRIDE})
    ROUTE_FALLBACK_REASON="user-override"
  else
    echo "loop-runner: ignoring invalid model override '$override_leg' in $CONTROL_DIR/model-$PHASE" >&2
  fi
fi

NOTE_FILE=""
[[ -n "$PHASE" ]] && NOTE_FILE="$LOOP_DIR/notes/$PHASE.md"
note_sig() { # "0" = no note; otherwise a content checksum
  if [[ -n "$NOTE_FILE" && -f "$NOTE_FILE" ]]; then echo "1:$(cksum < "$NOTE_FILE")"; else echo 0; fi
}
NOTE_SIG="$(note_sig)" NOTE_ROUND=0 NOTE_SID=""

is_paused() {
  [[ -e "$CONTROL_DIR/pause" ]] || [[ -n "$PHASE" && -e "$CONTROL_DIR/pause-$PHASE" ]]
}
route_task_attempt() {
  local attempt=1 phase_key cache decision raw request candidate confidence probabilities
  local mode decision_status fallback model actual threshold_ok tmp task_evidence evidence_ready
  [[ "$CHAIN_NAME" == "task" && -n "$RUN_ID" && -n "$PHASE" && -z "$MODEL_OVERRIDE" ]] || return 0
  [[ "${RUN_DIR:t}" =~ '-a([0-9]+)$' ]] && attempt="$match[1]"
  phase_key="${PHASE//[^A-Za-z0-9._-]/_}"
  cache="${RUN_DIR:h}/.route-${phase_key}.json"
  decision="$RUN_DIR/route-decision.json"

  if jq -e '.version == 1 and .stage == "route" and (.actualProfile | type == "string")' \
      "$decision" >/dev/null 2>&1; then
    :
  elif jq -e '.version == 1 and .stage == "route" and (.actualProfile | type == "string")' \
      "$cache" >/dev/null 2>&1; then
    tmp="$decision.tmp.$$"
    jq --argjson attempt "$attempt" --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      '.attempt=$attempt | .ts=$ts' "$cache" > "$tmp" && mv "$tmp" "$decision"
  else
    raw="$RUN_DIR/.route-jev.json"
    request="$RUN_DIR/.route-request.json"
    # The phase block starts near the top; never send the full runner prompt.
    task_evidence="$(head -c 8192 "$PROMPT_FILE" | jq -Rsc '
      def safe:
        gsub("(?i)[a-z][a-z0-9+.-]*://[^[:space:]/@:]+:[^[:space:]/@]+@[^[:space:]]+"; "[REDACTED_URL]")
        | gsub("[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=[[:space:]]*[^[:space:],;]+"; "[REDACTED]")
        | gsub("(?i)(password|secret|token|api[_-]?key|credential):[[:space:]]*[^[:space:],;]+"; "[REDACTED]")
        | .[:300];
      def field($block; $label):
        ($block | split("\n") | map(select(startswith("- **" + $label + ":**"))
          | ltrimstr("- **" + $label + ":**") | gsub("^[[:space:]]+"; "") | safe) | .[0] // null);
      (split("PHASE (from the shared plan):") | .[1] // "" | split("CONTEXT:")[0]) as $block
      | {title: ($block | split("\n") | map(select(startswith("### Phase "))
          | sub("^### Phase [^—]*—[[:space:]]*"; "")
          | sub("[[:space:]]*\\[lane:.*$"; "") | safe) | .[0] // null),
         doneWhen: field($block; "Done when"), estimate: field($block; "Estimate")}
    ')"
    evidence_ready="$(print -r -- "$task_evidence" | jq -r '(.doneWhen != null) and (.estimate != null)')"
    jq -cn --arg phase "$PHASE" --arg repository "$REPOSITORY" \
      --arg default "$LOOP_TASK_DEFAULT_PROFILE" --arg light "$LOOP_TASK_LIGHT_PROFILE" \
      --argjson task "$task_evidence" \
      '{stage:"route",state:{phase:$phase,repository:$repository,chain:"task",task:$task},questions:{profile:{type:"choice",instructions:"Choose the bounded task-runner profile for this ready phase using the task evidence.",criteria:{($default):"Established task chain",($light):"Lower-cost chain for a small, low-risk task"}}}}' \
      > "$request"
    if ! node "${LOOP_JEV_CLIENT:-$SCRIPT_DIR/loop-jev.mjs}" < "$request" > "$raw" 2>/dev/null; then
      print -r -- '{}' > "$raw"
    fi

    mode="$LOOP_JEV_MODE"
    decision_status="$(jq -r '.status // empty' "$raw" 2>/dev/null)"
    candidate="$(jq -r '.answers.profile.choice // empty' "$raw" 2>/dev/null)"
    confidence="$(jq -r 'if (.confidence | type) == "number" then .confidence else empty end' "$raw" 2>/dev/null)"
    probabilities="$(jq -c '.answers.profile.probabilities // {}' "$raw" 2>/dev/null)"
    [[ "$probabilities" == \{* ]] || probabilities='{}'
    model="$(jq -r '.model // empty' "$raw" 2>/dev/null)"
    fallback="$(jq -r '.reason // empty' "$raw" 2>/dev/null)"
    actual="$LOOP_TASK_DEFAULT_PROFILE"
    threshold_ok=false
    [[ -n "$confidence" ]] && threshold_ok="$(jq -nr --argjson value "$confidence" \
      --argjson minimum "$LOOP_JEV_ROUTE_MIN_CONFIDENCE" '$value >= $minimum')"

    if [[ "$candidate" != "$LOOP_TASK_DEFAULT_PROFILE" && "$candidate" != "$LOOP_TASK_LIGHT_PROFILE" && -n "$candidate" ]]; then
      fallback=unsupported-profile
    elif [[ "$decision_status" != "ok" ]]; then
      [[ -n "$fallback" ]] || fallback=decision-error
    elif [[ "$threshold_ok" != true ]]; then
      fallback=low-confidence
    elif [[ "$mode" == "shadow" ]]; then
      fallback=shadow-mode
    elif [[ "$mode" == "active" && "$candidate" == "$LOOP_TASK_LIGHT_PROFILE" ]]; then
      if [[ "$evidence_ready" != true ]]; then
        fallback=missing-task-evidence
      elif (( ! ${+CHAIN_TASK_LIGHT} )); then
        fallback=unsupported-profile
      else
        actual="$LOOP_TASK_LIGHT_PROFILE"
        fallback=""
      fi
    elif [[ "$mode" == "active" && "$candidate" == "$LOOP_TASK_DEFAULT_PROFILE" ]]; then
      fallback=""
    else
      [[ -n "$fallback" ]] || fallback=disabled
    fi

    tmp="$decision.tmp.$$"
    jq -cn --arg phase "$PHASE" --argjson attempt "$attempt" --arg mode "$mode" \
      --arg candidate "$candidate" --arg confidence "$confidence" --argjson probabilities "$probabilities" \
      --arg actual "$actual" --arg fallback "$fallback" --arg model "$model" \
      --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      '{version:1,phase:$phase,attempt:$attempt,stage:"route",mode:$mode,
        candidate:(if ($candidate|length)>0 then $candidate else null end),
        confidence:(if ($confidence|length)>0 then ($confidence|tonumber) else null end),
        probabilities:$probabilities,appliedAction:$actual,
        fallbackReason:(if ($fallback|length)>0 then $fallback else null end),
        resolvedModel:(if ($model|length)>0 then $model else null end),
        proposedProfile:(if ($candidate|length)>0 then $candidate else null end),actualProfile:$actual,ts:$ts}' \
      > "$tmp" && mv "$tmp" "$decision"
    cp "$decision" "$cache.tmp.$$" && mv "$cache.tmp.$$" "$cache"
    rm -f "$raw" "$request"
  fi

  ROUTE_PROPOSED_PROFILE="$(jq -r '.proposedProfile // empty' "$decision")"
  ROUTE_ACTUAL_PROFILE="$(jq -r '.actualProfile // empty' "$decision")"
  ROUTE_FALLBACK_REASON="$(jq -r '.fallbackReason // empty' "$decision")"
  [[ "$ROUTE_ACTUAL_PROFILE" == "$LOOP_TASK_LIGHT_PROFILE" ]] && chain=("${CHAIN_TASK_LIGHT[@]}")
  cat "$decision" | loop_emit_jev_decision "$RUN_ID"
}
route_task_attempt
[[ -n "${RUN_ID:-}" && -n "${PHASE:-}" ]] && jq -cn --arg event phase.attempt.start \
  --arg phase "$PHASE" --arg repository "$REPOSITORY" --arg detail "$CHAIN_NAME" --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{event:$event, phase:$phase, repository:(if ($repository|length)>0 then $repository else null end),
    detail:$detail, ts:$ts}' | loop_emit "$RUN_ID" event
[[ "$PROMPT_FILE" -ef "$RUN_DIR/prompt.md" ]] || cp "$PROMPT_FILE" "$RUN_DIR/prompt.md"
if (( CHECKPOINT_MODE )); then
  ATTEMPT_DEADLINE=$(( $(date +%s) + TIMEOUT ))
  cat >> "$RUN_DIR/prompt.md" <<EOF

30-MINUTE PHASE CHECKPOINT: This attempt ends at Unix time $ATTEMPT_DEADLINE.
Check \`date +%s\` between steps. About five minutes before the deadline, prepare a
short status. Before the deadline, if unfinished, write $RUN_DIR/status.json with
outcome "question", checkpoint true, a 1-3 line summary of work done and remaining,
and one concrete question for the orchestrator. Then stop. Do not ask the user or
run loop-handoff for unfinished work. The orchestrator will answer and resume this
phase or review stage in a fresh attempt; other lanes continue. If complete, use
the normal done handoff.
EOF
fi
TRANSCRIPT="$RUN_DIR/transcript.jsonl" STDERR="$RUN_DIR/stderr.log"
STATUS="$RUN_DIR/status.json" LAST="$RUN_DIR/last.md"
HEAD_BEFORE="$(git -C "$WT" rev-parse HEAD 2>/dev/null || echo unknown)"
CUR_ENGINE="" CUR_MODEL="" TIMED_OUT=0
ORIG_RESUME_SID="$RESUME_SID" PROMPT_IN="$RUN_DIR/prompt.md"

write_meta() { # $1 = engine exit code
  local sid model="$CUR_MODEL" init_model
  # claude aliases (opus, sonnet, fable, resume) → the concrete model from the init event
  if [[ "$CUR_ENGINE" == claude ]]; then
    init_model="$(jq -r 'select(.type == "system" and .subtype == "init") | .model // empty' \
      "$TRANSCRIPT" 2>/dev/null | head -1)"
    [[ -n "$init_model" ]] && model="$init_model"
  fi
  sid="$(jq -r '.session_id // .sessionId // .thread_id // (.msg.session_id? // empty) // empty' \
    "$TRANSCRIPT" 2>/dev/null | head -1)"
  jq -n --arg engine "$CUR_ENGINE" --arg model "$model" --arg effort "$(leg_effort)" --arg sid "${sid:-$NOTE_SID}" \
    --arg before "$HEAD_BEFORE" --arg after "$(git -C "$WT" rev-parse HEAD 2>/dev/null || echo unknown)" \
    --arg proposedProfile "$ROUTE_PROPOSED_PROFILE" --arg actualProfile "$ROUTE_ACTUAL_PROFILE" \
    --arg routeFallbackReason "$ROUTE_FALLBACK_REASON" --arg modelOverride "$MODEL_OVERRIDE" \
    --argjson rc "${1:-0}" --argjson timedOut "$TIMED_OUT" \
    '{engine: $engine, model: $model, effort:(if ($effort|length)>0 then $effort else null end),
      sessionId: $sid, headBefore: $before,
      headAfter: $after, engineExit: $rc, timedOut: ($timedOut == 1),
      proposedProfile:(if ($proposedProfile|length)>0 then $proposedProfile else null end),
      actualProfile:(if ($actualProfile|length)>0 then $actualProfile else null end),
      routeFallbackReason:(if ($routeFallbackReason|length)>0 then $routeFallbackReason else null end),
      modelOverride:(if ($modelOverride|length)>0 then $modelOverride else null end)}' \
    > "$RUN_DIR/meta.json"
}

# --- engine launchers (backgrounded by run_leg; cwd/-C = the worktree) ---

# Effort actually passed for the current leg: claude has no `ultra`, so it clamps to max.
leg_effort() {
  [[ -n "$EFFORT" ]] || return 0
  if [[ "$CUR_ENGINE" == claude && "$EFFORT" == ultra ]]; then print -r -- max; else print -r -- "$EFFORT"; fi
}

launch_claude() { # $1 model, $2 fallback (may be empty)
  local fb_args=() effort_args=() effort
  [[ -n "$2" ]] && fb_args=(--fallback-model "$2")
  effort="$(leg_effort)"
  [[ -n "$effort" ]] && effort_args=(--effort "$effort")
  if [[ -n "$RESUME_SID" ]]; then
    ( cd "$WT" && command claude -p --resume "$RESUME_SID" "${fb_args[@]}" "${effort_args[@]}" "${CLAUDE_EXTRA_ARGS[@]}" \
        --output-format stream-json --verbose \
        --allow-dangerously-skip-permissions --permission-mode bypassPermissions \
        --max-budget-usd "$BUDGET" < "$PROMPT_IN" > "$TRANSCRIPT" 2> "$STDERR" )
  else
    ( cd "$WT" && command claude -p --model "$1" "${fb_args[@]}" "${effort_args[@]}" "${CLAUDE_EXTRA_ARGS[@]}" \
        --output-format stream-json --verbose \
        --allow-dangerously-skip-permissions --permission-mode bypassPermissions \
        --max-budget-usd "$BUDGET" < "$PROMPT_IN" > "$TRANSCRIPT" 2> "$STDERR" )
  fi
}

launch_codex() { # $1 model
  local effort_args=() effort
  effort="$(leg_effort)"
  [[ -n "$effort" ]] && effort_args=(-c "model_reasoning_effort=\"$effort\"")
  # `codex exec resume` has no -C flag and filters sessions by cwd — cd is load-bearing
  if [[ -n "$RESUME_SID" ]]; then
    ( cd "$WT" && codex exec resume "$RESUME_SID" --json "${effort_args[@]}" "${CODEX_EXTRA_ARGS[@]}" \
        --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check \
        -o "$LAST" "$(cat "$PROMPT_IN")" > "$TRANSCRIPT" 2> "$STDERR" )
  else
    codex exec --json -C "$WT" -m "$1" "${effort_args[@]}" "${CODEX_EXTRA_ARGS[@]}" \
      --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check \
      -o "$LAST" - < "$PROMPT_IN" > "$TRANSCRIPT" 2> "$STDERR"
  fi
}

descendants() { # print pids of the full process tree under $1 (depth-first)
  local p
  for p in $(pgrep -P "$1" 2>/dev/null); do
    echo "$p"
    descendants "$p"
  done
}

run_leg() { # runs current engine with watchdogs; returns engine exit code (200 = timeout, 201 = paused, 202 = note changed)
  rm -f "$RUN_DIR/.timeout" "$RUN_DIR/.victims" "$RUN_DIR/.paused" "$RUN_DIR/.note-changed" "$RUN_DIR/.ctl-victims"
  local leg_timeout="$TIMEOUT"
  if (( CHECKPOINT_MODE )); then
    leg_timeout=$(( ATTEMPT_DEADLINE - $(date +%s) ))
    if (( leg_timeout <= 0 )); then
      TIMED_OUT=1
      return 200
    fi
  fi
  if [[ "$CUR_ENGINE" == "claude" ]]; then
    launch_claude "$CUR_MODEL" "$CUR_FALLBACK" &
  else
    launch_codex "$CUR_MODEL" &
  fi
  local child=$!
  # watchdog only marks + TERMs the whole tree; the KILL follow-through happens in the
  # main flow after wait (the watchdog dies with its TERM'd parent otherwise)
  ( sleep "$leg_timeout"
    kill -0 "$child" 2>/dev/null || exit 0
    { echo "$child"; descendants "$child"; } > "$RUN_DIR/.victims"
    touch "$RUN_DIR/.timeout"
    xargs kill -TERM 2>/dev/null < "$RUN_DIR/.victims"
  ) &
  local watchdog=$!
  # control watcher: a pause file or a note edit halts the engine within one poll
  ( while kill -0 "$child" 2>/dev/null; do
      sleep "$CONTROL_POLL"
      local marker=""
      if is_paused; then marker=.paused
      elif [[ "$(note_sig)" != "$NOTE_SIG" ]]; then marker=.note-changed
      fi
      if [[ -n "$marker" ]]; then
        { echo "$child"; descendants "$child"; } > "$RUN_DIR/.ctl-victims"
        touch "$RUN_DIR/$marker"
        xargs kill -TERM 2>/dev/null < "$RUN_DIR/.ctl-victims"
        exit 0
      fi
    done
  ) &
  local watcher=$!
  wait "$child"; local rc=$?
  kill "$watchdog" 2>/dev/null; wait "$watchdog" 2>/dev/null
  { echo "$watcher"; descendants "$watcher"; } | xargs kill 2>/dev/null; wait "$watcher" 2>/dev/null
  if [[ -f "$RUN_DIR/.paused" ]]; then
    reap_victims "$RUN_DIR/.ctl-victims"
    return 201
  fi
  if [[ -f "$RUN_DIR/.note-changed" ]]; then
    reap_victims "$RUN_DIR/.ctl-victims"
    return 202
  fi
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

apply_note_change() { # resume the interrupted session with the new note (same attempt, no retry used)
  local sid content prompt="$RUN_DIR/note-$(( NOTE_ROUND + 1 )).md"
  sid="$(jq -r '.session_id // .sessionId // .thread_id // (.msg.session_id? // empty) // empty' \
    "$TRANSCRIPT" 2>/dev/null | head -1)"
  sid="${sid:-$NOTE_SID}"
  NOTE_ROUND=$(( NOTE_ROUND + 1 ))
  NOTE_SIG="$(note_sig)"
  if [[ "$NOTE_SIG" == 0 ]]; then
    content="Steering note removed by the user (notes/$PHASE.md): note removed. Disregard the previous steering note and continue the phase from where you stopped."
  else
    content="Steering note updated by the user (notes/$PHASE.md):

$(cat "$NOTE_FILE")

Incorporate it and continue the phase from where you stopped."
  fi
  [[ -f "$TRANSCRIPT" ]] && mv "$TRANSCRIPT" "$RUN_DIR/transcript.pre-note-$NOTE_ROUND.jsonl"
  if [[ -n "$sid" ]]; then
    NOTE_SID="$sid" RESUME_SID="$sid"
    print -r -- "$content" > "$prompt"
  else # nothing to resume yet: restart the leg with the note appended
    { cat "$RUN_DIR/prompt.md"; print -r -- ""; print -r -- "$content"; } > "$prompt"
  fi
  PROMPT_IN="$prompt"
  [[ -n "${RUN_ID:-}" ]] && jq -cn --arg event note.applied --arg phase "$PHASE" \
    --arg detail "${${content%%$'\n'*}}" --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{event:$event, phase:$phase, detail:$detail, ts:$ts}' | loop_emit "$RUN_ID" event
  echo "loop-runner: note changed — resuming ${sid:-a fresh run} with the update" >&2
}

# codex has no "latest" aliases: `codex:@sol` → the highest-versioned *listed* gpt-<v>-sol in
# codex's live catalog (`codex debug models`, else its on-disk cache). Empty = not found.
resolve_codex_family() { # $1 = family (sol, astra, terra, …)
  # bounded (perl alarm): a hung catalog call must never eat the phase deadline
  { perl -e 'alarm shift; exec @ARGV' 15 codex debug models 2>/dev/null ||
      cat "$HOME/.codex/models_cache.json" 2>/dev/null; } |
    jq -r --arg fam "$1" '[.models[]? | select((.visibility // "list") == "list") | .slug
        | select(test("^gpt-[0-9]+(\\.[0-9]+)*-" + $fam + "$"))]
      | sort_by(split("-")[1] | split(".") | map(tonumber)) | last // empty' 2>/dev/null
}

reap_victims() { # $1 = pid list file; KILL anything that ignored the TERM
  local survivors
  sleep 0.5
  survivors="$(xargs -n1 sh -c 'kill -0 "$0" 2>/dev/null && echo "$0"' < "$1" 2>/dev/null)"
  [[ -n "$survivors" ]] && echo "$survivors" | xargs kill -KILL 2>/dev/null
  return 0
}

write_checkpoint() {
  local head_after change_count
  head_after="$(git -C "$WT" rev-parse HEAD 2>/dev/null || echo unknown)"
  change_count="$(git -C "$WT" status --short 2>/dev/null | wc -l | tr -d ' ')"
  {
    print -r -- "# Phase checkpoint"
    print -r -- "HEAD: $HEAD_BEFORE → $head_after"
    print -r -- "Worktree changes: $change_count"
    print -r -- ""
    print -r -- "## Commits in this attempt"
    git -C "$WT" log --oneline "$HEAD_BEFORE..$head_after" 2>/dev/null | head -20
    print -r -- ""
    print -r -- "## Current worktree"
    git -C "$WT" status --short 2>/dev/null | head -30
    print -r -- ""
    print -r -- "## Diff summary"
    git -C "$WT" diff --stat 2>/dev/null | head -30
  } > "$RUN_DIR/checkpoint.md"
  jq -n --arg summary "30-minute checkpoint: HEAD $HEAD_BEFORE → $head_after; $change_count worktree changes. See checkpoint.md." \
    --arg question "What should this phase prioritize in the next work block?" \
    '{outcome:"question",checkpoint:true,summary:$summary,question:$question}' > "$STATUS"
}

pause_retry() {
  local delay="$1" remaining
  if (( CHECKPOINT_MODE )); then
    remaining=$(( ATTEMPT_DEADLINE - $(date +%s) ))
    (( remaining <= 0 )) && return 0
    (( delay > remaining )) && delay="$remaining"
  fi
  sleep "$delay"
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
  if (( CHECKPOINT_MODE && $(date +%s) >= ATTEMPT_DEADLINE )); then
    TIMED_OUT=1
    write_checkpoint
    write_meta 124
    exit 10
  fi
  CUR_ENGINE="${leg%%:*}"
  rest="${leg#*:}"
  CUR_MODEL="${rest%%+*}"
  CUR_FALLBACK=""
  [[ "$rest" == *"+"* ]] && CUR_FALLBACK="${rest#*+}"
  if [[ "$CUR_ENGINE" == codex && "$CUR_MODEL" == @* ]]; then
    resolved="$(resolve_codex_family "${CUR_MODEL#@}")"
    if [[ -z "$resolved" ]]; then
      echo "loop-runner: no codex model for family $CUR_MODEL in the catalog — skipping leg" >&2
      continue
    fi
    CUR_MODEL="$resolved"
  fi
  # a note-driven resume belongs to the leg that was interrupted; the next leg starts fresh,
  # carrying the latest note if one was applied mid-attempt
  RESUME_SID="$ORIG_RESUME_SID" PROMPT_IN="$RUN_DIR/prompt.md"
  if (( NOTE_ROUND > 0 )) && [[ "$NOTE_SIG" != 0 ]]; then
    { cat "$RUN_DIR/prompt.md"; print -r -- ""; print -r -- "Latest steering note (notes/$PHASE.md):"; cat "$NOTE_FILE"; } > "$RUN_DIR/note-fresh.md"
    PROMPT_IN="$RUN_DIR/note-fresh.md"
  fi

  retries=0 delay=10 leg_done=0
  while [[ $retries -lt 3 ]]; do
    if is_paused; then
      write_meta 30
      echo "loop-runner: paused before spawning $leg" >&2
      exit 30
    fi
    rm -f "$STATUS"
    run_leg; rc=$?
    if [[ $rc -eq 201 ]]; then
      rm -f "$STATUS"
      write_meta 30
      echo "loop-runner: paused — engine halted on $leg" >&2
      exit 30
    fi
    if [[ $rc -eq 202 ]]; then
      apply_note_change
      continue
    fi
    if [[ $rc -eq 200 ]]; then
      if (( CHECKPOINT_MODE )); then
        if [[ "$(jq -r '.outcome // empty' "$STATUS" 2>/dev/null)" == "done" ]]; then
          leg_done=1
          break
        fi
        [[ "$(jq -r '.outcome // empty' "$STATUS" 2>/dev/null)" == "question" ]] || write_checkpoint
        write_meta 124
        echo "loop-runner: phase checkpoint after ${TIMEOUT}s" >&2
        exit 10
      fi
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
      [[ $retries -lt 3 ]] && { pause_retry "$delay"; delay=$((delay * 3)); }
      continue
    fi
    # unknown failure: one retry, then next leg
    if [[ $retries -eq 0 ]]; then
      retries=1
      echo "loop-runner: unknown failure on $leg (rc=$rc), one retry" >&2
      pause_retry "$delay"
      continue
    fi
    echo "loop-runner: giving up on $leg (rc=$rc)" >&2
    break
  done
  [[ $leg_done -eq 1 ]] && { final_rc=0; break; }
done

if (( CHECKPOINT_MODE && final_rc == 40 && $(date +%s) >= ATTEMPT_DEADLINE )); then
  TIMED_OUT=1
  write_checkpoint
  write_meta 124
  exit 10
fi

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
