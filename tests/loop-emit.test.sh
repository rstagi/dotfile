#!/bin/zsh
# loop-emit.sh sends nothing to the loop-web daemon when WS_LOOP_ID is set (skills loop),
# and behaves as before when it is unset (old engine). Fake curl/launchctl/nohup are first on PATH.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

unset WS_LOOP_ID WS_SESSION_ID LOOP_DAEMON_URL
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

export HOME="$TMP/home"
FAKEBIN="$TMP/bin"
CALLS="$TMP/calls.log"
UP="$TMP/daemon-up"
mkdir -p "$HOME/dotfile" "$FAKEBIN"
printf '#!/bin/sh\nexit 0\n' >"$HOME/dotfile/loop-web.sh"
chmod +x "$HOME/dotfile/loop-web.sh"

# The fake daemon answers /api/health only after $UP exists.
cat >"$FAKEBIN/curl" <<FAKE
#!/bin/sh
echo "curl \$*" >>"$CALLS"
case "\$*" in
  */api/health*) [ -e "$UP" ] ;;
  *) cat >/dev/null 2>&1; exit 0 ;;
esac
FAKE
# launchctl brings the daemon up only when FAKE_LAUNCHCTL_UP=1.
cat >"$FAKEBIN/launchctl" <<FAKE
#!/bin/sh
echo "launchctl \$*" >>"$CALLS"
[ "\${FAKE_LAUNCHCTL_UP:-0}" = 1 ] && : >"$UP"
exit 0
FAKE
cat >"$FAKEBIN/nohup" <<FAKE
#!/bin/sh
echo "nohup \$*" >>"$CALLS"
: >"$UP"
exit 0
FAKE
chmod +x "$FAKEBIN/curl" "$FAKEBIN/launchctl" "$FAKEBIN/nohup"
export PATH="$FAKEBIN:$PATH"
export LOOP_DAEMON_URL="http://127.0.0.1:9"

source "$ROOT/loop-emit.sh"
reset() { : >"$CALLS"; rm -f "$UP"; }
calls() { cat "$CALLS"; }

echo "# WS_LOOP_ID set: skills loop"
export WS_LOOP_ID="01SKILLSLOOP"
reset
echo '{"a":1}' | loop_emit run-1 event
assert_exit "$?" "0" "loop_emit returns 0"
echo '{"a":1}' | loop_emit_jev_decision run-1
assert_exit "$?" "0" "loop_emit_jev_decision returns 0"
loop_ensure_daemon
assert_exit "$?" "0" "loop_ensure_daemon returns 0"
assert_eq "$(calls)" "" "no curl, launchctl or nohup call"

echo "# WS_LOOP_ID unset: old engine"
unset WS_LOOP_ID
reset
echo '{"a":1}' | loop_emit run-1 event
assert_contains "$(calls)" "curl -sf -m 2 -X POST --data-binary @- http://127.0.0.1:9/api/loops/run-1/event" "loop_emit posts to the daemon"

reset
echo '{"a":1}' | loop_emit_jev_decision run-1
assert_contains "$(calls)" "/api/loops/run-1/event" "loop_emit_jev_decision posts the event"

reset
FAKE_LAUNCHCTL_UP=1 loop_ensure_daemon
assert_exit "$?" "0" "loop_ensure_daemon returns 0"
assert_contains "$(calls)" "launchctl kickstart -k gui/$(id -u)/com.rstagi.loop-web" "ensure kicks the LaunchAgent"
assert_eq "$(grep -c '^nohup' "$CALLS")" "0" "no nohup when launchctl brings the daemon up"

reset
loop_ensure_daemon
assert_exit "$?" "0" "loop_ensure_daemon returns 0 after the fallback"
assert_contains "$(calls)" "nohup $HOME/dotfile/loop-web.sh --daemon" "ensure falls back to loop-web.sh"

reset
: >"$UP"
loop_ensure_daemon
assert_eq "$(grep -c -e '^launchctl' -e '^nohup' "$CALLS")" "0" "healthy daemon needs no launchctl or nohup"

test_summary
