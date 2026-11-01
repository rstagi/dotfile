#!/bin/zsh
set -u -o pipefail

# Loop Engineering — supervise poller. Read-only on loop state; one fixed entry point so the
# supervise session can allowlist it and never hit a permission prompt mid-run.
#
# Usage:
#   loop-poll.sh status   --dir <abs .loop>
#     Print compact state: non-todo phases, last events, pending outbox, orchestrator, HIL, barrier.
#   loop-poll.sh wait     --dir <abs .loop> [--max <ticks>]
#     Wait (10 s ticks, default 170) for a phase status change, REVIEW_READY, a dead orchestrator
#     or an open HIL brief. Write the reason + status to <dir>/poll.out and print it.
#   loop-poll.sh wait-run --dir <abs .loop> --run <abs run dir> [--max <ticks>]
#     Wait for <run>/meta.json or a dead runner pid. Write the result to <dir>/poll.out.
#   loop-poll.sh nag      --dir <abs .loop> [--every <sec>] [--count <n>]
#     While REVIEW_READY exists, no merge gate is parked and no review run moved for <every>
#     seconds (default 900), send a loud notification. At most <count> times (default 8).

DIR="" RUN="" MAX=170 EVERY=900 COUNT=8
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

main() {
  local cmd="${1:-}"; [[ -n "$cmd" ]] && shift
  while [[ $# -gt 0 ]]; do
    case "$1" in
    --dir) DIR="$2"; shift 2 ;;
    --run) RUN="$2"; shift 2 ;;
    --max) MAX="$2"; shift 2 ;;
    --every) EVERY="$2"; shift 2 ;;
    --count) COUNT="$2"; shift 2 ;;
    *) echo "loop-poll: unknown arg $1" >&2; exit 1 ;;
    esac
  done
  [[ -n "$DIR" && -f "$DIR/state.json" ]] || { echo "loop-poll: --dir must be a loop dir with state.json" >&2; exit 1; }
  STATE="$DIR/state.json" SUB="$DIR/sub" OUT="$DIR/poll.out"

  case "$cmd" in
  status) print_status ;;
  wait) wait_for_change ;;
  wait-run)
    [[ -n "$RUN" ]] || { echo "loop-poll: wait-run requires --run" >&2; exit 1; }
    wait_for_run
    ;;
  nag) nag_review_barrier ;;
  *) echo "loop-poll: unknown command '$cmd' (status|wait|wait-run|nag)" >&2; exit 1 ;;
  esac
}

print_status() {
  echo "now: $(date -u +%FT%TZ)"
  echo "phases: $(jq -c '.phases|to_entries|map(select(.value.status!="todo"))|map({(.key):.value.status})|add // {}' "$STATE")"
  echo "orchestrator: $(orch_alive && echo alive || echo stopped)"
  [[ -e "$SUB/REVIEW_READY" ]] && echo "barrier: REVIEW_READY"
  [[ "$(jq -r '.awaitingMerge // empty' "$STATE")" != "" ]] && echo "parked: $(jq -c '.awaitingMerge' "$STATE")"
  local hil; hil="$(open_hil)"; [[ -n "$hil" ]] && echo "hil: $hil"
  echo "--- events"
  tail -n 8 "$DIR/events.jsonl" 2>/dev/null
  if [[ -s "$DIR/board-outbox.jsonl" ]]; then
    echo "--- outbox"
    cat "$DIR/board-outbox.jsonl"
  fi
}

wait_for_change() {
  local before reason="timeout" i hil
  before="$(phase_statuses)"
  for (( i = 1; i <= MAX; i++ )); do
    sleep 10
    [[ -e "$SUB/REVIEW_READY" ]] && { reason="REVIEW_READY"; break; }
    orch_alive || { reason="ORCH_DEAD"; break; }
    hil="$(open_hil)"; [[ -n "$hil" ]] && { reason="HIL $hil"; break; }
    [[ "$(phase_statuses)" != "$before" ]] && { reason="STATUS_CHANGE"; break; }
  done
  { echo "reason: $reason"; print_status; } > "$OUT"
  cat "$OUT"
}

wait_for_run() {
  local reason="timeout" i pid
  pid="$(cat "$RUN/pid" 2>/dev/null || true)"
  for (( i = 1; i <= MAX; i++ )); do
    sleep 10
    [[ -f "$RUN/meta.json" ]] && { reason="DONE"; break; }
    [[ -n "$pid" ]] && ! kill -0 "$pid" 2>/dev/null && { reason="RUNNER_DEAD"; break; }
  done
  {
    echo "reason: $reason"
    echo "run: $RUN"
    [[ -f "$RUN/meta.json" ]] && echo "meta: $(jq -c . "$RUN/meta.json")"
    [[ -f "$RUN/status.json" ]] && echo "status: $(jq -c '{outcome,summary}' "$RUN/status.json" 2>/dev/null)"
    echo "spawn.log: $(tail -n 2 "$RUN/spawn.log" 2>/dev/null | tr '\n' ' ')"
  } > "$DIR/poll-run.out"
  cat "$DIR/poll-run.out"
}

# A stalled review barrier is silent otherwise: the SUB exits and only the supervise chat can
# start the review. Repeat a loud notification until a review run moves or the barrier clears.
nag_review_barrier() {
  local sent=0 run_id last now
  run_id="$(jq -r '.runId' "$STATE")"
  while (( sent < COUNT )); do
    sleep "$EVERY"
    [[ -e "$SUB/REVIEW_READY" ]] || return 0
    [[ "$(jq -r '.awaitingMerge // empty' "$STATE")" != "" ]] && return 0
    last="$(newest_review_mtime)"; now="$(date +%s)"
    (( now - last < EVERY )) && continue
    "$SCRIPT_DIR/loop-notify.sh" --level question --run-id "$run_id" --event review.stalled \
      --title "Loop review is waiting" \
      --body "$run_id: the review phase has not started. Open the Beluga chat, or run /loop-execute resume." \
      >/dev/null 2>&1 || true
    sent=$(( sent + 1 ))
  done
}

orch_alive() { kill -0 "$(cat "$SUB/orch.pid" 2>/dev/null)" 2>/dev/null; }

phase_statuses() { jq -c '[.phases[] | .status]' "$STATE"; }

open_hil() {
  local b
  for b in "$DIR"/hil/*.md(N); do
    [[ "$b" == *.answer.md || -e "${b%.md}.answer.md" ]] && continue
    echo "${b:t}"; return
  done
}

newest_review_mtime() {
  local f newest=0 m
  for f in "$DIR"/runs/review-p*/*(N) "$DIR"/runs/converge-p*/*(N) "$SUB/REVIEW_READY"; do
    m="$(stat -f %m "$f" 2>/dev/null || echo 0)"
    (( m > newest )) && newest=$m
  done
  echo "$newest"
}

main "$@"
