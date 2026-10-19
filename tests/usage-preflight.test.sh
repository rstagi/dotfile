#!/bin/zsh
set -u
HERE="${0:A:h}"
ROOT="${HERE:h}"
source "$HERE/lib.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
mkdir -p "$TMP/bin" "$TMP/.loop"
unset WS_BIN
export HOME="$TMP" WS_HOME="$TMP/.ws" WS_STATE="$TMP/state"
export PATH="$TMP/bin:$PATH" WS_SOCK="$TMP/wsd.sock" RUNTIME_ROOT="$ROOT"
export FAKE_WS_LOG="$TMP/ws.log"
cat > "$TMP/bin/ws" <<'WS'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_WS_LOG"
[ "$WS_SOCK" != "$HOME/.ws/wsd.sock" ] || exit 99
exit "${FAKE_WS_EXIT:-0}"
WS
chmod +x "$TMP/bin/ws"
# Execute the start command from the skill. Tests never contact a daemon.
awk '/^Before the start, check runner quota:/{found=1;next} found && /^```sh/{code=1;next} code && /^```/{exit} code{print}' \
  "$ROOT/.claude/skills/loop-execute/SKILL.md" > "$TMP/preflight.sh"
[[ -s "$TMP/preflight.sh" ]]
assert_exit "$?" 0 "the skill declares a quota command before the start"
for code in 0 2 9; do
  (cd "$TMP" && FAKE_WS_EXIT=$code zsh -e preflight.sh) >"$TMP/out" 2>"$TMP/err"
  assert_exit "$?" 0 "the start continues after quota exit $code"
  assert_contains "$(cat "$FAKE_WS_LOG")" 'usage
--preflight
--plan
.loop/plan.md' "the start checks the plan"
  if (( code != 0 )); then
    assert_contains "$(cat "$TMP/err")" 'no quota data' "the start reports missing quota"
  fi
done
(cd "$TMP" && WS_BIN="$TMP/missing" zsh -e preflight.sh) >"$TMP/out" 2>"$TMP/err"
assert_exit "$?" 0 "a missing ws never blocks the start"
assert_contains "$(cat "$TMP/err")" 'no quota data' "a missing ws produces a warning"
test_summary
