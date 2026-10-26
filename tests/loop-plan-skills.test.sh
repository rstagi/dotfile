#!/bin/zsh
# Test plan routing through the public CLI with private backends.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
TMP="$(mktemp -d)"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
unset WS_LOOP_ID WS_SESSION_ID LOOP_DAEMON_URL
export HOME="$TMP/home" WS_HOME="$TMP/ws" WS_STATE="$TMP/state" WS_SOCK="$TMP/ws.sock"
export LOOP_DAEMON_URL=http://127.0.0.1:9
export FAKE_PLAN_WS="$TMP/store" LOOP_EMIT_SH="$HERE/fake/loop-emit.sh" EMIT_LOG="$TMP/emit.log"
mkdir -p "$HOME" "$TMP/bin" "$FAKE_PLAN_WS" "$TMP/loop"
ln -s "$HERE/fake/loop-plan-ws.mjs" "$TMP/bin/ws"
export PATH="$TMP/bin:$PATH"
export WS_BIN=ws
: > "$EMIT_LOG"
plan() { zsh "$ROOT/loop-plan.sh" "$@"; }
cat > "$TMP/loop/plan.md" <<'PLAN'
<!-- loop-plan
planId: skills-plan
engine: skills
daemon: http://127.0.0.1:9
-->
# Skills effort
PLAN

echo 'loop-plan: skills register preserves the file bytes'
OUT="$(plan register --dir "$TMP/loop" --effort skills </dev/null 2>"$TMP/error")"; rc=$?
assert_exit "$rc" 0 'register succeeds'
assert_eq "$OUT" skills-plan 'register prints the plan id'
assert_eq "$(cat "$EMIT_LOG")" '' 'register does not write to loop-web'
if [[ -f "$FAKE_PLAN_WS/store.json" ]]; then
  jq -j '.[0].plan.text' "$FAKE_PLAN_WS/store.json" > "$TMP/received"
  cmp -s "$TMP/received" "$TMP/loop/plan.md"
  assert_exit "$?" 0 'the event retains the final newline'
else
  assert_eq missing present 'register creates a wsd plan'
fi

test_summary
