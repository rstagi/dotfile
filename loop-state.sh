#!/bin/zsh
set -u -o pipefail

# Loop Engineering — state helper: atomic state.json ops, run lock, event journal.
# State dir defaults to .kestral/loop under the cwd repo.
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

DIR=".kestral/loop"
CMD="${1:-}"; [[ -n "$CMD" ]] && shift

JSON="" OWNER="" FORCE=0 ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
  --dir) DIR="$2"; shift 2 ;;
  --json) JSON="$2"; shift 2 ;;
  --owner) OWNER="$2"; shift 2 ;;
  --force) FORCE=1; shift ;;
  *) ARGS+=("$1"); shift ;;
  esac
done

STATE="$DIR/state.json"

die() { echo "loop-state: $1" >&2; exit "${2:-1}"; }

case "$CMD" in
init)
  [[ -n "$JSON" ]] || die "init requires --json"
  # printf, not echo — zsh echo mangles backslash escapes inside JSON strings
  printf '%s\n' "$JSON" | jq . >/dev/null || die "init: --json is not valid JSON"
  mkdir -p "$DIR/runs" "$DIR/answers" "$DIR/hil"
  if [[ -f "$STATE" ]]; then
    die "state.json already exists (use resume, or remove $DIR)" 2
  fi
  printf '%s\n' "$JSON" | jq . > "$STATE"
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
  ;;
log)
  mkdir -p "$DIR"
  jq -cn --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg event "${ARGS[1]:-}" --arg phase "${ARGS[2]:-}" --arg detail "${ARGS[3]:-}" \
    '{ts: $ts, event: $event, phase: $phase, detail: $detail}' >> "$DIR/events.jsonl"
  ;;
*)
  die "unknown command '${CMD}' (init|lock|unlock|get|set|log)"
  ;;
esac
