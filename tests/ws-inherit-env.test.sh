#!/bin/zsh
# Test both Loop launch boundaries with new and installed ws CLI capabilities.
set -u
if [[ "${1:-}" != --clean ]]; then
  exec env -i PATH="$PATH" /bin/zsh -f "$0" --clean
fi
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
mkdir -p "$TMP/bin" "$TMP/home" "$TMP/runtime"
cp "$ROOT/loop-runner.sh" "$ROOT/loop-orchestrator.sh" "$TMP/runtime/"
cat > "$TMP/runtime/loop-emit.sh" <<'SH'
loop_emit() { cat >/dev/null; }
loop_emit_jev_decision() { cat >/dev/null; }
SH
cat > "$TMP/models.conf" <<'SH'
CHAIN_TASK=(codex:test-engine)
LOOP_BUDGET_USD=1
LOOP_TIMEOUT_TASK=30
SH
cat > "$TMP/bin/ws" <<'PY'
#!/usr/bin/env python3
import json
import os
import pathlib
import sys

args = sys.argv[1:]
if args == ['--version']:
    print('ws 0.1.0')
elif args == ['ping']:
    pass
elif args == ['session', 'run', '--help']:
    print('Usage: ws session run [--headless] [--env K=V]')
    if os.environ['FAKE_WS_CAPABILITY'] == 'new':
        print('  --inherit-env  Read the CLI environment.')
elif args[:2] == ['session', 'run']:
    marker = os.environ['WS_TEST_PRIVATE_VALUE']
    result = {
        'marker_in_argv': any(marker in arg for arg in args),
        'inherit': '--inherit-env' in args,
        'env_count': args.count('--env'),
        'marker_in_environment': bool(marker),
    }
    pathlib.Path(os.environ['FAKE_WS_RESULT']).write_text(json.dumps(result))
    if os.environ['FAKE_WS_SITE'] == 'runner':
        pathlib.Path(os.environ['FAKE_RUN_DIR'], 'status.json').write_text('{"outcome":"done"}')
        print('{"type":"result","result":"done"}')
else:
    sys.exit(90)
PY
chmod +x "$TMP/bin/ws"
export PATH="$TMP/bin:$PATH" HOME="$TMP/home"
export WS_HOME="$TMP/home/.ws" WS_STATE="$TMP/state" WS_SOCK="$TMP/wsd.sock"
export WS_TMUX_SOCKET="$TMP/tmux.sock" LOOP_WS_SESSIONS=1 LOOP_JEV_MODE=off
export LOOP_JEV_KEY_FILE="$TMP/missing-key"
# Keep the value inside the shell environment. Never write it to an argument log.
export WS_TEST_PRIVATE_VALUE="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
print -r -- 'Complete the sandbox task.' > "$TMP/prompt.md"
for capability in new installed; do
  for site in runner orchestrator; do
    export FAKE_WS_CAPABILITY="$capability" FAKE_WS_SITE="$site"
    export FAKE_WS_RESULT="$TMP/$capability-$site.json"
    export FAKE_RUN_DIR="$TMP/$capability-$site-run"
    mkdir -p "$FAKE_RUN_DIR"
    if [[ "$site" == runner ]]; then
      zsh "$TMP/runtime/loop-runner.sh" --worktree "$TMP" --run-dir "$FAKE_RUN_DIR" \
        --prompt-file "$TMP/prompt.md" --models-conf "$TMP/models.conf" --timeout 30 \
        > "$TMP/$capability-$site.log" 2>&1
    else
      zsh "$TMP/runtime/loop-orchestrator.sh" --run-id sandbox --dir "$FAKE_RUN_DIR" \
        > "$TMP/$capability-$site.log" 2>&1
    fi
    assert_exit "$?" 0 "$site accepts the $capability CLI"
    assert_eq "$(jq -r '.marker_in_environment' "$FAKE_WS_RESULT")" true "$site receives the exported marker"
    if [[ "$capability" == new ]]; then
      assert_eq "$(jq -r '.inherit' "$FAKE_WS_RESULT")" true "$site uses environment inheritance"
      assert_eq "$(jq -r '.marker_in_argv' "$FAKE_WS_RESULT")" false "$site keeps the marker out of arguments"
      assert_eq "$(jq -r '.env_count' "$FAKE_WS_RESULT")" 0 "$site supplies no environment assignments"
    else
      assert_eq "$(jq -r '.inherit' "$FAKE_WS_RESULT")" false "$site supports the installed CLI"
      assert_eq "$(jq -r '.marker_in_argv' "$FAKE_WS_RESULT")" true "$site retains legacy environment forwarding"
    fi
  done
done
test_summary
