#!/bin/zsh
# ws tests: hook-file migration after ws moved (ws _migrate-hooks and startup), in a private WS_HOME/GROK_HOME.
# Not standalone: ws/tests/run.sh sources helpers.sh then the area files in order, in one process,
# because workspaces, tabs and global session ids carry over between areas.
[[ -n "${WS_TESTS_SUITE:-}" ]] || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

# stale_hooks <ws-bin> — writes claude-settings.json + Grok hooks that name <ws-bin>.
stale_hooks() {
  mkdir -p "$MIG_HOME" "${MIG_GROK_HOOKS:h}"
  jq -n --arg h "$1 hook" '{hooks: {Stop: [{hooks: [{type: "command", command: "\($h) stop", timeout: 5}]}]}}' > "$MIG_HOME/claude-settings.json"
  jq -n --arg h "$1 hook" '{hooks: {Stop: [{hooks: [{type: "command", command: "\($h) stop", timeout: 5}]}]}}' > "$MIG_GROK_HOOKS"
}
hook_stop_cmd() { jq -r '.hooks.Stop[0].hooks[0].command' "$1"; }
migrate() { WS_HOME="$MIG_HOME" GROK_HOME="$MIG_GROK" "$WS" "$@"; }

echo "ws migrate: stale hook files are re-pointed at the current ws"
MIG_HOME="$TMP/mig-home" MIG_GROK="$TMP/mig-grok" MIG_GROK_HOOKS="$TMP/mig-grok/hooks/ws.json"
stale_hooks "/old/dotfile/ws.sh"
migrate _migrate-hooks
assert_exit "$?" "0" "_migrate-hooks succeeds"
assert_eq "$(hook_stop_cmd "$MIG_HOME/claude-settings.json")" "${(q)WS} hook stop" "claude-settings.json names the current ws"
assert_eq "$(hook_stop_cmd "$MIG_GROK_HOOKS")" "${(q)WS} hook stop" "Grok hooks name the current ws"
assert_eq "$(jq -r '.hooks.PermissionRequest[0].hooks[0].command' "$MIG_GROK_HOOKS")" "${(q)WS} hook waiting" "Grok hooks are fully rewritten, not patched"

echo "ws migrate: a file that already names ws is left untouched"
touch -t 200001010000 "$MIG_HOME/claude-settings.json" "$MIG_GROK_HOOKS"
old_mtime="$(stat -f %m "$MIG_HOME/claude-settings.json")"
migrate _migrate-hooks
assert_eq "$(stat -f %m "$MIG_HOME/claude-settings.json")" "$old_mtime" "current claude-settings.json is not rewritten"
assert_eq "$(stat -f %m "$MIG_GROK_HOOKS")" "$old_mtime" "current Grok hooks are not rewritten"

echo "ws migrate: a longer path ending in the current ws does not count as current"
stale_hooks "/prefix$WS"
migrate _migrate-hooks
assert_eq "$(hook_stop_cmd "$MIG_GROK_HOOKS")" "${(q)WS} hook stop" "suffix-matching Grok path is re-pointed"
assert_eq "$(hook_stop_cmd "$MIG_HOME/claude-settings.json")" "${(q)WS} hook stop" "suffix-matching claude path is re-pointed"

echo "ws migrate: missing files are not created"
rm -f "$MIG_HOME/claude-settings.json" "$MIG_GROK_HOOKS"
migrate _migrate-hooks
assert_eq "$([[ -e "$MIG_HOME/claude-settings.json" || -e "$MIG_GROK_HOOKS" ]] && print created)" "" "neither file appears out of nowhere"

echo "ws migrate: ws startup migrates too"
stale_hooks "/old/dotfile/ws.sh"
migrate open no/such-workspace 2>/dev/null
assert_exit "$?" "1" "open of an unknown workspace still fails"
assert_eq "$(hook_stop_cmd "$MIG_GROK_HOOKS")" "${(q)WS} hook stop" "open re-points Grok hooks"
assert_eq "$(hook_stop_cmd "$MIG_HOME/claude-settings.json")" "${(q)WS} hook stop" "open re-points claude-settings.json"
stale_hooks "/old/dotfile/ws.sh"
migrate hook working </dev/null >/dev/null 2>&1
assert_eq "$(hook_stop_cmd "$MIG_GROK_HOOKS")" "/old/dotfile/ws.sh hook stop" "the hook hot path never rewrites hook files"
