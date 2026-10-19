#!/bin/zsh
# Test the aggregate runner through isolated suite fixtures.
set -u
HERE="${0:A:h}"
source "$HERE/lib.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
mkdir -p "$TMP/repo/tests"
cp "$HERE/run.sh" "$TMP/repo/tests/run.sh"
cp "$HERE/lib.sh" "$TMP/repo/tests/lib.sh"
print -r -- 'exit 1' > "$TMP/repo/tests/first.test.sh"
print -r -- 'exit 2' > "$TMP/repo/tests/second.test.sh"
print -r -- 'exit 0' > "$TMP/repo/tests/passing.test.sh"
zsh "$TMP/repo/tests/run.sh" > "$TMP/result" 2>&1
assert_exit "$?" 1 "the runner fails when suites fail"
assert_eq "$(tail -n 1 "$TMP/result")" \
  'SHELL TESTS FAILED: first.test.sh, second.test.sh' \
  "the last line names each failed suite"

rm "$TMP/repo/tests/first.test.sh" "$TMP/repo/tests/second.test.sh"
mkdir -p "$TMP/repo/ws/tests"
print -r -- 'exit 0' > "$TMP/repo/ws/tests/run.sh"
for suite in runner-ws runner-ws-cancel; do
  cat > "$TMP/repo/tests/$suite.test.sh" <<'EOF'
ROOT="${0:A:h:h}"
source "$ROOT/tests/lib.sh"
start_ws_test_daemon
EOF
done
zsh "$TMP/repo/tests/run.sh" > "$TMP/result" 2>&1
assert_exit "$?" 1 "a missing Rust binary fails the runner"
assert_eq "$(tail -n 1 "$TMP/result")" \
  'SHELL TESTS FAILED: runner-ws-cancel.test.sh, runner-ws.test.sh' \
  "the last line names both missing-binary failures"
assert_eq "$(rg -c 'Missing Rust binary: .* Run cargo build in wsd\.' "$TMP/result")" \
  2 "every Rust suite reports the same build instruction"

rm "$TMP/repo/tests/runner-ws.test.sh" "$TMP/repo/tests/runner-ws-cancel.test.sh"
mkdir -p "$TMP/repo/wsd/target/debug"
print -r -- '#!/bin/sh\nexit 0' > "$TMP/repo/wsd/target/debug/ws"
chmod +x "$TMP/repo/wsd/target/debug/ws"
cat > "$TMP/repo/ws/tests/run.sh" <<'EOF'
exit "${SUITE_WS_RESULT:-0}"
EOF
zsh "$TMP/repo/tests/run.sh" > "$TMP/result" 2>&1
assert_exit "$?" 0 "the runner passes when all suites pass"
assert_eq "$(tail -n 1 "$TMP/result")" 'ALL SHELL TESTS PASSED' \
  "the success summary stays unchanged"
SUITE_WS_RESULT=1 zsh "$TMP/repo/tests/run.sh" > "$TMP/result" 2>&1
assert_exit "$?" 1 "the runner fails when the picker smoke fails"
assert_eq "$(tail -n 1 "$TMP/result")" \
  'SHELL TESTS FAILED: ws/tests/run.sh' \
  "the last line names the picker smoke"
test_summary
