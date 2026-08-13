#!/bin/zsh
set -u -o pipefail

# Loop Engineering — state helper: atomic state.json ops, run lock, event journal.
# State dir defaults to .loop under the cwd repo (flattened — plan.md sits beside state.json).
#
# Usage:
#   loop-state.sh init [--dir <d>] --json '<initial state json>'
#   loop-state.sh lock [--dir <d>] --owner <run-id> [--force]
#     reentrant for the same owner; exit 3 if another owner holds it (prints owner+age);
#     --force steals. The orchestrator judges staleness (a session has no stable pid).
#   loop-state.sh unlock [--dir <d>]
#   loop-state.sh get [--dir <d>] '<jq filter>'
#   loop-state.sh set [--dir <d>] '<jq expression>'   # state.json | jq expr, atomic mv
#   loop-state.sh log [--dir <d>] <event> [<phase>] [<detail>]
#   loop-state.sh note [--dir <d>] [--force] <key> [text]
#   loop-state.sh note [--dir <d>] --clear <key>
#   loop-state.sh notes [--dir <d>]
#   loop-state.sh occupancy --transcript <f> [--window <n>]
#     sum the LAST usage-bearing assistant event in a stream-json transcript; print the
#     token total (+ percent with --window); exit 10 when >= LOOP_ORCH_RECYCLE_TOKENS.

DIR=".loop"
CMD="${1:-}"; [[ -n "$CMD" ]] && shift

JSON="" OWNER="" FORCE=0 CLEAR=0 TRANSCRIPT="" WINDOW="" REPOSITORY="" ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
  --dir) DIR="$2"; shift 2 ;;
  --json) JSON="$2"; shift 2 ;;
  --owner) OWNER="$2"; shift 2 ;;
  --force) FORCE=1; shift ;;
  --clear) CLEAR=1; shift ;;
  --transcript) TRANSCRIPT="$2"; shift 2 ;;
  --window) WINDOW="$2"; shift 2 ;;
  --repository) REPOSITORY="$2"; shift 2 ;;
  *) ARGS+=("$1"); shift ;;
  esac
done

STATE="$DIR/state.json"

die() { echo "loop-state: $1" >&2; exit "${2:-1}"; }

# Best-effort daemon emission (sourced; no-op if loop-emit.sh is missing).
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ -r "$SCRIPT_DIR/loop-emit.sh" ]]; then
  source "$SCRIPT_DIR/loop-emit.sh"
else
  loop_emit() { :; }
  loop_ensure_daemon() { :; }
fi

state_run_id() { jq -r '.runId // empty' "$STATE" 2>/dev/null || true; }

# Ensure the daemon is up, register this loop (identity + plan path), and seed its state.
emit_register() {
  [[ -f "$STATE" ]] || return 0
  local run_id loop_abs plan_file
  run_id="$(state_run_id)"; [[ -n "$run_id" ]] || return 0
  loop_abs="$(cd "$DIR" 2>/dev/null && pwd)" || return 0
  plan_file="$loop_abs/plan.md"   # flattened .loop: plan.md sits beside state.json
  loop_ensure_daemon
  jq -c --arg loopDir "$loop_abs" --arg planFile "$plan_file" --arg started "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{loopDir:$loopDir, planFile:$planFile, effort:(.effort//null),
      projectId:(.projectId//null), integrationBranch:(.integrationBranch//null),
      startedAt:$started}' "$STATE" | loop_emit "$run_id" register
  loop_emit "$run_id" state < "$STATE"
}

# Push the current state.json to the daemon (authoritative status folds it monotonically).
emit_state() {
  local run_id; run_id="$(state_run_id)"; [[ -n "$run_id" ]] || return 0
  loop_emit "$run_id" state < "$STATE"
}

case "$CMD" in
init)
  [[ -n "$JSON" ]] || die "init requires --json"
  # printf, not echo — zsh echo mangles backslash escapes inside JSON strings
  printf '%s\n' "$JSON" | jq . >/dev/null || die "init: --json is not valid JSON"
  mkdir -p "$DIR/runs" "$DIR/answers" "$DIR/hil" "$DIR/notes"
  if [[ -f "$STATE" ]]; then
    die "state.json already exists (use resume, or remove $DIR)" 2
  fi
  printf '%s\n' "$JSON" | jq . > "$STATE"
  emit_register
  ;;
lock)
  [[ -n "$OWNER" ]] || die "lock requires --owner <run-id>"
  mkdir -p "$DIR"
  if mkdir "$DIR/lock" 2>/dev/null; then
    printf '%s\n' "$OWNER" > "$DIR/lock/owner"
    exit 0
  fi
  holder="$(cat "$DIR/lock/owner" 2>/dev/null || true)"
  if [[ "$holder" == "$OWNER" ]]; then
    exit 0 # reentrant
  fi
  if [[ -z "$holder" || "$FORCE" -eq 1 ]]; then
    # empty holder = crash between mkdir and owner write → treat as stale
    rm -rf "$DIR/lock"
    mkdir "$DIR/lock" 2>/dev/null || die "lock race" 3
    printf '%s\n' "$OWNER" > "$DIR/lock/owner"
    # narrow the concurrent-steal window: confirm we really are the recorded owner
    [[ "$(cat "$DIR/lock/owner" 2>/dev/null)" == "$OWNER" ]] || die "lock race" 3
    exit 0
  fi
  age="$(( $(date +%s) - $(stat -f %m "$DIR/lock" 2>/dev/null || date +%s) ))"
  die "locked by '$holder' (${age}s ago) — use --force to steal" 3
  ;;
unlock)
  rm -rf "$DIR/lock"
  ;;
get)
  [[ -f "$STATE" ]] || die "no state.json in $DIR"
  jq -r "${ARGS[1]:-.}" "$STATE"
  ;;
set)
  [[ -f "$STATE" ]] || die "no state.json in $DIR"
  [[ -n "${ARGS[1]:-}" ]] || die "set requires a jq expression"
  tmp="$(mktemp "$DIR/.state.XXXXXX")"
  if jq "${ARGS[1]}" "$STATE" > "$tmp"; then
    mv "$tmp" "$STATE"
  else
    rm -f "$tmp"
    die "jq expression failed"
  fi
  emit_state
  ;;
log)
  mkdir -p "$DIR"
  line="$(jq -cn --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg event "${ARGS[1]:-}" --arg phase "${ARGS[2]:-}" --arg repository "$REPOSITORY" \
    --arg detail "${ARGS[3]:-}" \
    '{ts: $ts, event: $event, phase: $phase,
      repository:(if ($repository|length)>0 then $repository else null end), detail: $detail}')"
  printf '%s\n' "$line" >> "$DIR/events.jsonl"
  run_id="$(state_run_id)"
  [[ -n "$run_id" ]] && printf '%s' "$line" | loop_emit "$run_id" event
  ;;
note)
  key="${ARGS[1]:-}"
  [[ -n "$key" ]] || die "note requires <key>"
  note_key_re='^[A-Za-z0-9._-]+$'
  [[ "$key" =~ $note_key_re && "$key" != "." && "$key" != ".." ]] \
    || die "invalid note key '$key' (use letters, numbers, dot, underscore, or hyphen)"
  note_file="$DIR/notes/$key.md"
  if [[ "$CLEAR" -eq 1 ]]; then
    rm -f "$note_file"
    exit 0
  fi
  if [[ "$key" == <-> && "$FORCE" -ne 1 && -f "$STATE" ]]; then
    phase_status="$(jq -r --arg key "$key" '.phases[$key].status // empty' "$STATE" 2>/dev/null || true)"
    if [[ "$phase_status" == "done" || "$phase_status" == "merged" ]]; then
      die "phase $key is already $phase_status; use --force to write anyway"
    fi
  fi
  mkdir -p "$DIR/notes"
  if (( ${#ARGS} >= 2 )); then
    note_text="${ARGS[2]}"
  else
    note_text="$(cat)"
  fi
  printf '%s\n' "$note_text" > "$note_file"
  ;;
notes)
  for note_file in "$DIR/notes"/*.md(N); do
    key="${${note_file:t}%.md}"
    first_line="$(head -n 1 "$note_file")"
    printf '%s\t%s\n' "$key" "$first_line"
  done
  ;;
register)
  # Idempotent ensure-daemon + register + seed state (used by resume).
  [[ -f "$STATE" ]] || die "no state.json in $DIR"
  emit_register
  ;;
finish)
  # POST the loop's terminal payload ({status?, prUrl?, review?}) to the daemon.
  [[ -f "$STATE" ]] || die "no state.json in $DIR"
  body="$JSON"; [[ -n "$body" ]] || body='{}'
  printf '%s' "$body" | jq -e . >/dev/null 2>&1 || die "finish: --json is not valid JSON"
  run_id="$(state_run_id)"
  [[ -n "$run_id" ]] || die "finish: state.json has no runId"
  printf '%s' "$body" | jq -c --arg fa "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '. + {finishedAt: (.finishedAt // $fa)}' | loop_emit "$run_id" finish
  ;;
occupancy)
  # SUB self-measurement: the last usage-bearing `assistant` event's context size =
  # input_tokens + cache_read_input_tokens + cache_creation_input_tokens. `fromjson?`
  # tolerates a malformed/partial last line (a mid-write transcript). No state.json needed.
  [[ -n "$TRANSCRIPT" ]] || die "occupancy requires --transcript <file>"
  tokens=0
  if [[ -f "$TRANSCRIPT" ]]; then
    tokens="$(jq -R 'fromjson? | select(.type == "assistant" and .message.usage != null
        and ((.message.usage.input_tokens // .message.usage.cache_read_input_tokens // .message.usage.cache_creation_input_tokens) != null))
      | (.message.usage | (.input_tokens // 0) + (.cache_read_input_tokens // 0) + (.cache_creation_input_tokens // 0))' \
      "$TRANSCRIPT" 2>/dev/null | tail -n1)"
    [[ -n "$tokens" ]] || tokens=0
  fi
  # Recycle threshold + UI window: explicit env > loop-models.conf > default. Snapshot env
  # BEFORE sourcing the conf (the conf assigns both knobs unconditionally, and gating the
  # source on one would clobber the env or suppress the other's default).
  _e_recycle="${LOOP_ORCH_RECYCLE_TOKENS:-}"
  _e_window="${LOOP_ORCH_CTX_WINDOW:-}"
  [[ -r "$SCRIPT_DIR/loop-models.conf" ]] && source "$SCRIPT_DIR/loop-models.conf"
  threshold="${_e_recycle:-${LOOP_ORCH_RECYCLE_TOKENS:-150000}}"
  [[ "$threshold" == <-> ]] || threshold=150000  # non-integer override → default (keep exit 0/10 contract)
  win="$WINDOW"; [[ -n "$win" ]] || win="${_e_window:-${LOOP_ORCH_CTX_WINDOW:-}}"
  percent=""
  [[ "$win" == <1-> ]] && percent=$(( tokens * 100 / win ))
  # stdout: bare occupancy prints ONLY the token total; percent is appended only when --window
  # was explicitly requested AND resolved to a valid integer (no dangling space on a bad window).
  if [[ -n "$WINDOW" && -n "$percent" ]]; then
    printf '%s %s\n' "$tokens" "$percent"
  else
    printf '%s\n' "$tokens"
  fi
  # Best-effort sub.saturation heartbeat (only when a run id is resolvable; never fails).
  run_id="$(state_run_id)"
  if [[ -n "$run_id" ]]; then
    printf '%s' "$(jq -cn --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --argjson tokens "$tokens" --argjson percent "${percent:-null}" \
      '{ts: $ts, event: "sub.saturation", tokens: $tokens, percent: $percent}')" \
      | loop_emit "$run_id" event
  fi
  if [[ "$tokens" -ge "$threshold" ]]; then exit 10; fi
  exit 0
  ;;
*)
  die "unknown command '${CMD}' (init|lock|unlock|get|set|log|note|notes|register|finish|occupancy)"
  ;;
esac
