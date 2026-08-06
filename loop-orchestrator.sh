#!/bin/zsh
set -u -o pipefail

# loop-orchestrator.sh — the sequential SUB respawner for the two-tier self-recycling loop.
# Detached (nohup) by `kestral-loop` supervise. Runs disposable headless sub-orchestrator
# instances (claude -p, CHAIN_ORCHESTRATE) ONE AT A TIME; on each exit it reads sub/status.json
# and recycles / completes / gives up. Sequential-by-construction: the predecessor has exited
# before the successor starts, so two SUBs never share the run id (the reentrant lock gives no
# mutual exclusion). See loop-protocol.md § Two-tier orchestration.
#
# Usage: loop-orchestrator.sh --run-id <id> [--dir <loopDir>] [--prompt-file <f>]

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DIR=".kestral/loop"
RUN_ID=""
PROMPT_FILE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
  --dir) DIR="$2"; shift 2 ;;
  --run-id) RUN_ID="$2"; shift 2 ;;
  --prompt-file) PROMPT_FILE="$2"; shift 2 ;;
  *) echo "loop-orchestrator: unknown arg $1" >&2; exit 1 ;;
  esac
done
[[ -n "$RUN_ID" ]] || { echo "loop-orchestrator: --run-id required" >&2; exit 1; }

# --- config + wiring (seams: ENGINE_CMD / LOOP_EMIT_SH / LOOP_NOTIFY_SH override for tests) ---
# Precedence: explicit env > loop-models.conf > hardcoded default. Snapshot env BEFORE sourcing
# the conf (the conf assigns these knobs unconditionally and would otherwise clobber the env).
_e_recycle="${LOOP_ORCH_RECYCLE_TOKENS:-}"
_e_window="${LOOP_ORCH_CTX_WINDOW:-}"
_e_stall="${LOOP_ORCH_STALL_SEC:-}"
_e_poll="${LOOP_ORCH_POLL_SEC:-}"
_e_max="${LOOP_ORCH_MAX_RESPAWN:-}"
_e_budget="${LOOP_ORCH_BUDGET_USD:-}"
[[ -r "$SCRIPT_DIR/loop-models.conf" ]] && source "$SCRIPT_DIR/loop-models.conf"

EMIT_SH="${LOOP_EMIT_SH:-$SCRIPT_DIR/loop-emit.sh}"
if [[ -r "$EMIT_SH" ]]; then source "$EMIT_SH"; else loop_emit() { :; }; loop_ensure_daemon() { :; }; fi
NOTIFY_SH="${LOOP_NOTIFY_SH:-$SCRIPT_DIR/loop-notify.sh}"
ENGINE_CMD="${ENGINE_CMD:-claude}"

RECYCLE_TOKENS="${_e_recycle:-${LOOP_ORCH_RECYCLE_TOKENS:-150000}}"
CTX_WINDOW="${_e_window:-${LOOP_ORCH_CTX_WINDOW:-200000}}"
STALL_SEC="${_e_stall:-${LOOP_ORCH_STALL_SEC:-1500}}"
POLL_SEC="${_e_poll:-${LOOP_ORCH_POLL_SEC:-30}}"
MAX_RESPAWN="${_e_max:-${LOOP_ORCH_MAX_RESPAWN:-3}}"
BUDGET="${_e_budget:-${LOOP_ORCH_BUDGET_USD:-15}}"

SUB="$DIR/sub"
STATUS="$SUB/status.json"
PIDFILE="$SUB/current.pid"
mkdir -p "$SUB/control"

# First CHAIN_ORCHESTRATE leg → engine:model[+fallback]  (zsh arrays are 1-indexed).
leg="${CHAIN_ORCHESTRATE[1]:-claude:fable+opus}"
[[ "$leg" == *:* ]] || leg="claude:fable+opus"  # guard a scalar/garbage CHAIN_ORCHESTRATE
LEG_MODEL="${leg#*:}"; LEG_MODEL="${LEG_MODEL%%+*}"
LEG_FALLBACK=""; [[ "${leg#*:}" == *"+"* ]] && LEG_FALLBACK="${leg#*+}"

# Line-buffer wrapper so stream-json flushes per turn (occupancy freshness). Best-effort:
# macOS lacks stdbuf; occupancy tolerates ~1-turn lag and a pty sidecar is the documented fallback.
LB=()
if command -v stdbuf >/dev/null 2>&1; then LB=(stdbuf -oL -eL)
elif command -v gstdbuf >/dev/null 2>&1; then LB=(gstdbuf -oL -eL)
fi

notify() { "$NOTIFY_SH" --level "$1" --run-id "$RUN_ID" --title "loop-orchestrator" --body "$2" >/dev/null 2>&1 || true; }

descendants() { local p; for p in $(pgrep -P "$1" 2>/dev/null); do echo "$p"; descendants "$p"; done; }
kill_tree() {
  { echo "$1"; descendants "$1"; } | xargs kill -TERM 2>/dev/null
  sleep 2
  { echo "$1"; descendants "$1"; } | xargs kill -KILL 2>/dev/null
}

# --- SUB prompt (instance 1 may use a supervise-supplied file; later instances resume) ------
build_sub_prompt() {
  local k="$1"
  if [[ "$k" -eq 1 && -n "$PROMPT_FILE" && -f "$PROMPT_FILE" ]]; then
    cat "$PROMPT_FILE"; return
  fi
  local handoff_note=""
  [[ -f "$SUB/handoff.md" ]] && handoff_note="A recycle handoff is waiting — read \`$SUB/handoff.md\` ONCE as your first act, then rename it to \`$SUB/handoff.consumed-$k.md\`."
  cat <<EOF
You are a headless sub-orchestrator (instance $k) for a two-tier self-recycling Kestral loop,
run id $RUN_ID. Invoke the \`kestral-loop\` skill in headless \`sub\` mode: start by running
\`kestral-loop resume\` to reconcile git > Kestral > state and re-attach to any live phase
runners (NEVER double-spawn a phase whose run dir has a live pid).
$handoff_note
Each tick, self-measure occupancy:
  loop-state.sh occupancy --dir $DIR --transcript $SUB/transcript-$k.jsonl --window $CTX_WINDOW
At or above $RECYCLE_TOKENS tokens AND a safe phase boundary (no runner mid-attempt without a
written status.json, no merge in progress), flush durable state, write \`$SUB/handoff.md\`,
then write \`$STATUS\` with outcome "recycle" and exit. NEVER run pr-review; NEVER
AskUserQuestion (HIL → files only, keep other lanes running). When every phase is merged,
write \`$STATUS\` outcome "complete". On an unrecoverable error, outcome "fatal" or "blocked".
Your final act MUST be writing \`$STATUS\` (schema: {outcome, summary, tokens, recycleIndex}).
EOF
}

# --- orphan guard: a prior loop-orchestrator may have left a live SUB — wait it out ---------
# Adopt ONLY a live process that still LOOKS like our SUB (its argv carries the distinctive
# `--output-format stream-json`) — never block on a stranger the pid was recycled to after an
# abnormal orchestrator death. A non-matching / dead pid falls through to a fresh spawn.
is_our_sub() { ps -p "$1" -o command= 2>/dev/null | grep -q -- '--output-format stream-json'; }
if [[ -f "$PIDFILE" ]]; then
  op="$(cat "$PIDFILE" 2>/dev/null || true)"
  if [[ -n "$op" ]] && kill -0 "$op" 2>/dev/null && is_our_sub "$op"; then
    echo "loop-orchestrator: adopting orphaned SUB pid $op — waiting for it to exit" >&2
    while kill -0 "$op" 2>/dev/null && is_our_sub "$op"; do sleep "$POLL_SEC"; done
  fi
fi

# --- main loop -----------------------------------------------------------------------------
# LOOP_SUB=1 tells `kestral-loop resume` it is a headless SUB instance (not a human re-attach).
export LOOP_SUB=1
export LOOP_SUB_DIR="$SUB"
k=1
recycles=0
consecutive_crashes=0

while :; do
  transcript="$SUB/transcript-$k.jsonl"
  stall_marker="$SUB/.stalled-$k"
  rm -f "$STATUS" "$stall_marker"
  : > "$transcript"
  build_sub_prompt "$k" > "$SUB/prompt-$k.md"

  "${LB[@]}" "$ENGINE_CMD" -p --model "$LEG_MODEL" ${LEG_FALLBACK:+--fallback-model "$LEG_FALLBACK"} \
    --output-format stream-json --verbose \
    --allow-dangerously-skip-permissions --permission-mode bypassPermissions \
    --max-budget-usd "$BUDGET" \
    < "$SUB/prompt-$k.md" > "$transcript" 2> "$SUB/spawn-$k.log" &
  child=$!
  echo "$child" > "$PIDFILE"

  # Stall watchdog: kill the SUB pid tree if the transcript mtime goes stale (heartbeat lost).
  (
    while kill -0 "$child" 2>/dev/null; do
      sleep "$POLL_SEC"
      kill -0 "$child" 2>/dev/null || break
      mt=$(stat -f %m "$transcript" 2>/dev/null || echo 0)
      age=$(( $(date +%s) - mt ))
      if [[ "$age" -ge "$STALL_SEC" ]]; then
        touch "$stall_marker"
        kill_tree "$child"
        break
      fi
    done
  ) &
  watchdog=$!

  wait "$child" 2>/dev/null; rc=$?
  { echo "$watchdog"; descendants "$watchdog"; } | xargs kill 2>/dev/null  # reap the watchdog + its sleep child
  wait "$watchdog" 2>/dev/null

  outcome="$(jq -r '.outcome // empty' "$STATUS" 2>/dev/null || true)"

  # crash = killed by the stall watchdog, or exited without a valid status.json outcome.
  if [[ -f "$stall_marker" || -z "$outcome" ]]; then
    consecutive_crashes=$(( consecutive_crashes + 1 ))
    echo "loop-orchestrator: SUB instance $k crashed (rc=$rc, stalled=$([[ -f "$stall_marker" ]] && echo 1 || echo 0)) — $consecutive_crashes/$MAX_RESPAWN" >&2
    if [[ "$consecutive_crashes" -ge "$MAX_RESPAWN" ]]; then
      notify error "SUB crashed $consecutive_crashes× (last rc=$rc) — orchestrator giving up"
      rm -f "$PIDFILE"
      exit 1
    fi
    k=$(( k + 1 ))
    continue
  fi

  case "$outcome" in
  recycle)
    consecutive_crashes=0  # a clean recycle is progress — only this resets the crash streak
    tokens="$(jq -r '.tokens // 0' "$STATUS" 2>/dev/null || echo 0)"
    ridx="$(jq -r '.recycleIndex // empty' "$STATUS" 2>/dev/null || true)"
    recycles=$(( recycles + 1 ))
    [[ -n "$ridx" ]] || ridx="$recycles"
    printf '%s' "$(jq -cn --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --argjson tokens "${tokens:-0}" --argjson recycleIndex "$ridx" \
      '{ts: $ts, event: "sub.recycle", tokens: $tokens, recycleIndex: $recycleIndex}')" \
      | loop_emit "$RUN_ID" event
    echo "loop-orchestrator: SUB instance $k recycled at ${tokens} tok (recycle #$ridx) — respawning" >&2
    k=$(( k + 1 ))
    continue
    ;;
  complete)
    touch "$SUB/PHASES_DONE"
    notify info "phases complete — ready to finalize"
    rm -f "$PIDFILE"
    exit 0
    ;;
  fatal|blocked)
    summary="$(jq -r '.summary // "no summary"' "$STATUS" 2>/dev/null || echo "?")"
    notify error "SUB $outcome: $summary"
    rm -f "$PIDFILE"
    exit 2
    ;;
  *)
    consecutive_crashes=$(( consecutive_crashes + 1 ))  # unknown outcome → treat as a crash
    if [[ "$consecutive_crashes" -ge "$MAX_RESPAWN" ]]; then
      notify error "SUB unknown outcome '$outcome' ×$consecutive_crashes — giving up"
      rm -f "$PIDFILE"
      exit 1
    fi
    k=$(( k + 1 ))
    continue
    ;;
  esac
done
