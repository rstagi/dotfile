#!/bin/zsh
# Minimal shell test lib for the loop-engineering scripts. Source it, run assert_*,
# end with test_summary (exits 0 all-pass / 1 any-fail). tests/run.sh aggregates files.
set -u

# Never let tests push events to the user's real daemon (:7717); tests that need one
# start their own and override this.
export LOOP_DAEMON_URL="http://127.0.0.1:9"
# Suites enable ws only after they configure a private daemon.
export LOOP_WS_SESSIONS=0

TESTS_RUN=0
TESTS_FAIL=0

_red() { printf '\033[31m%s\033[0m\n' "$1"; }
_grn() { printf '\033[32m%s\033[0m\n' "$1"; }

# assert_eq <actual> <expected> <msg>
assert_eq() {
  TESTS_RUN=$((TESTS_RUN + 1))
  if [[ "$1" == "$2" ]]; then
    _grn "  ok: $3"
  else
    TESTS_FAIL=$((TESTS_FAIL + 1))
    _red "  FAIL: $3"
    _red "    expected: [$2]"
    _red "    actual:   [$1]"
  fi
}

# assert_exit <actual-code> <expected-code> <msg>
assert_exit() {
  TESTS_RUN=$((TESTS_RUN + 1))
  if [[ "$1" == "$2" ]]; then
    _grn "  ok: $3 (exit $1)"
  else
    TESTS_FAIL=$((TESTS_FAIL + 1))
    _red "  FAIL: $3 (expected exit $2, got $1)"
  fi
}

# assert_contains <haystack> <needle> <msg>
assert_contains() {
  TESTS_RUN=$((TESTS_RUN + 1))
  if [[ "$1" == *"$2"* ]]; then
    _grn "  ok: $3"
  else
    TESTS_FAIL=$((TESTS_FAIL + 1))
    _red "  FAIL: $3"
    _red "    [$1] does not contain [$2]"
  fi
}

test_summary() {
  echo "-----"
  if [[ $TESTS_FAIL -eq 0 ]]; then
    _grn "$TESTS_RUN passed"
    exit 0
  fi
  _red "$TESTS_FAIL/$TESTS_RUN failed"
  exit 1
}

# path_without_command <name> — print $PATH without each directory that holds an executable <name>.
# Use it when a test must control <name>: a host installation must not replace a test double.
path_without_command() {
  local directory
  local -a kept=()
  for directory in ${(s.:.)PATH}; do
    [[ -f "$directory/$1" && -x "$directory/$1" ]] && continue
    kept+=("$directory")
  done
  print -r -- "${(j.:.)kept}"
}

# make_fake_ws <dir> — write <dir>/ws, a double that appends each call to $FAKE_WS_LOG (the arguments as
# one JSON array per line) and exits with $FAKE_WS_EXIT (default 0). Put <dir> first on PATH to use it.
make_fake_ws() {
  mkdir -p "$1"
  cat > "$1/ws" <<'FAKE'
#!/bin/sh
[ -n "${FAKE_WS_LOG:-}" ] && jq -cn '$ARGS.positional' --args -- "$@" >> "$FAKE_WS_LOG"
[ "${FAKE_WS_EXIT:-0}" = 0 ] || echo "fake ws failure" >&2
exit "${FAKE_WS_EXIT:-0}"
FAKE
  chmod +x "$1/ws"
}

# Require the debug binary for each Rust suite.
require_ws_test_binary() {
  [[ -x "$ROOT/wsd/target/debug/ws" ]] && return 0
  print -u2 -- "Missing Rust binary: $ROOT/wsd/target/debug/ws. Run cargo build in wsd."
  return 1
}

# Start a private Rust daemon on $WS_SOCK. The caller must stop WS_TEST_DAEMON_PID on exit.
# The probe pings only after the socket exists: `ws ping` autostarts a daemon when the socket is missing.
start_ws_test_daemon() {
  require_ws_test_binary || return 1
  WS_RUST_BIN="$ROOT/wsd/target/debug/ws"
  [[ -n "${WS_SOCK:-}" ]] || { print -u2 -- "Set WS_SOCK to a private socket before the test daemon starts."; return 1; }
  "$WS_RUST_BIN" daemon > "$TMP/ws-daemon.log" 2>&1 &
  WS_TEST_DAEMON_PID=$!
  local deadline=$(( SECONDS + 30 ))
  until [[ -S "$WS_SOCK" ]] && "$WS_RUST_BIN" ping >/dev/null 2>&1; do
    if ! kill -0 "$WS_TEST_DAEMON_PID" 2>/dev/null || (( SECONDS >= deadline )); then
      cat "$TMP/ws-daemon.log" >&2
      return 1
    fi
    sleep 0.05
  done
}
