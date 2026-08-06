#!/bin/zsh
# Test stub for loop-emit.sh — records each loop_emit call (runId, action, stdin body) to
# $EMIT_LOG instead of POSTing to the daemon. Sourced by loop-orchestrator via LOOP_EMIT_SH.
loop_emit() {
  local rid="$1" act="$2"
  { printf '%s %s ' "$rid" "$act"; cat; printf '\n'; } >> "${EMIT_LOG:-/dev/null}"
  return 0
}
loop_ensure_daemon() { :; }
loop_ensure_wait() { :; }
