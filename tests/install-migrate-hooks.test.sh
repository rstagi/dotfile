#!/bin/zsh
# install.sh re-points legacy hook files even when the selector chooses the Rust backend.
set -eu
ROOT="${0:A:h:h}"
SANDBOX="$(mktemp -d /tmp/ws-migrate-hooks.XXXXXX)"
trap 'rm -rf "$SANDBOX"' EXIT
mkdir -p "$SANDBOX/.ws" "$SANDBOX/dotfile"
ln -s "$ROOT/ws" "$SANDBOX/dotfile/ws"
print rust > "$SANDBOX/.ws/backend"
print -r -- '{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"/old/ws hook stop"}]}]}}' \
  > "$SANDBOX/.ws/claude-settings.json"
line="$(grep -F 're-point agent hooks' "$ROOT/install.sh")"
[[ -n "$line" ]] || { print -u2 'FAIL: install.sh has no hook migration call'; exit 1; }
# The Rust binary fails every command here: the migration must not depend on it.
output="$(HOME="$SANDBOX" WS_HOME="$SANDBOX/.ws" WS_RUST_BIN=/usr/bin/false \
  WS_TMUX_SOCKET="$SANDBOX/tmux.sock" GROK_HOOKS="$SANDBOX/grok-hooks.json" \
  zsh -c "$line" 2>&1)"
[[ "$output" != *"could not re-point"* ]] || { print -u2 "FAIL: migration failed: $output"; exit 1; }
grep -qF "${ROOT}/ws/ws.sh hook stop" "$SANDBOX/.ws/claude-settings.json" \
  || { print -u2 'FAIL: claude-settings.json still names the old ws'; exit 1; }
print 'PASS: install.sh re-points hooks under the Rust backend'
