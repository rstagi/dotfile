#!/bin/zsh
# ws tests: hook-file migration after ws moved (ws _migrate-hooks and startup), in a private WS_HOME/GROK_HOME.
# Not standalone: ws/tests/run.sh sources helpers.sh then its group's area files in order, in one
# process, because workspaces, tabs and global session ids carry over within a group.
(( $+functions[ws_tests_require_sandbox] )) && ws_tests_require_sandbox || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

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
mkdir -p "$MIG_HOME"
print -r -- zsh > "$MIG_HOME/backend"
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

echo "ws migrate: startup leaves hooks that name another working ws alone"
OTHER_WS="$TMP/other-dotfile/ws/ws.sh"
mkdir -p "${OTHER_WS:h}" && printf '#!/bin/sh\n' > "$OTHER_WS" && chmod +x "$OTHER_WS"
stale_hooks "$OTHER_WS"
migrate open no/such-workspace 2>/dev/null
assert_eq "$(hook_stop_cmd "$MIG_GROK_HOOKS")" "$OTHER_WS hook stop" "open keeps Grok hooks naming a working ws"
assert_eq "$(hook_stop_cmd "$MIG_HOME/claude-settings.json")" "$OTHER_WS hook stop" "open keeps claude-settings.json naming a working ws"
migrate add no/such-workspace 2>/dev/null
assert_exit "$?" "1" "add of an unknown workspace still fails"
assert_eq "$(hook_stop_cmd "$MIG_GROK_HOOKS")" "$OTHER_WS hook stop" "add keeps Grok hooks naming a working ws"
migrate new --repo "$TMP/not-a-repo" --branch x 2>/dev/null
assert_exit "$?" "1" "new of a non-repo still fails"
assert_eq "$(hook_stop_cmd "$MIG_GROK_HOOKS")" "$OTHER_WS hook stop" "new keeps Grok hooks naming a working ws"
migrate _migrate-hooks
assert_eq "$(hook_stop_cmd "$MIG_GROK_HOOKS")" "${(q)WS} hook stop" "_migrate-hooks forces the re-point"
assert_eq "$(hook_stop_cmd "$MIG_HOME/claude-settings.json")" "${(q)WS} hook stop" "_migrate-hooks forces claude-settings.json too"

echo "ws migrate: add and new migrate hooks whose ws is gone"
stale_hooks "/old/dotfile/ws.sh"
migrate add no/such-workspace 2>/dev/null
assert_eq "$(hook_stop_cmd "$MIG_GROK_HOOKS")" "${(q)WS} hook stop" "add re-points Grok hooks"
stale_hooks "/old/dotfile/ws.sh"
migrate new --repo "$TMP/not-a-repo" --branch x 2>/dev/null
assert_eq "$(hook_stop_cmd "$MIG_HOME/claude-settings.json")" "${(q)WS} hook stop" "new re-points claude-settings.json"

echo "ws migrate: the bin link and tmux bindings follow a move"
mkdir -p "$MIG_HOME/bin"
ln -sfn "$TMP/gone/ws.sh" "$MIG_HOME/bin/ws"
T new-session -d -s mig-bind 2>/dev/null
T bind-key a run-shell "/old/dotfile/ws.sh add"
stale_hooks "/old/dotfile/ws.sh"
migrate open no/such-workspace 2>/dev/null
assert_eq "$([[ "$MIG_HOME/bin/ws" -ef "$WS" ]] && print yes)" "yes" "dangling bin/ws is re-linked"
assert_eq "$(T list-keys -T prefix a | grep -cF -- "$WS")" "1" "startup re-binds tmux keys after a move"
ln -sfn "$OTHER_WS" "$MIG_HOME/bin/ws"
T bind-key a run-shell "/old/dotfile/ws.sh add"
stale_hooks "$OTHER_WS"
migrate open no/such-workspace 2>/dev/null
assert_eq "$(readlink "$MIG_HOME/bin/ws")" "$OTHER_WS" "a working bin/ws link is left alone"
assert_eq "$(T list-keys -T prefix a | grep -cF -- "/old/dotfile/ws.sh")" "1" "nothing moved: bindings untouched"
migrate _migrate-hooks
assert_eq "$([[ "$MIG_HOME/bin/ws" -ef "$WS" ]] && print yes)" "yes" "_migrate-hooks re-links bin/ws"
assert_eq "$(T list-keys -T prefix a | grep -cF -- "$WS")" "1" "_migrate-hooks re-binds tmux keys"

echo "ws migrate: launches that skip the startup migration (send, resume, ws claude) re-bind a moved ws too"
rm -f "$MIG_HOME/bin/ws"; ln -sfn "$TMP/gone/ws.sh" "$MIG_HOME/bin/ws"
T bind-key a run-shell "/old/dotfile/ws.sh add"
stale_hooks "/old/dotfile/ws.sh"
WS_HOME="$MIG_HOME" GROK_HOME="$MIG_GROK" zsh -c 'source "$1"; agent_command claude "" "" 1 >/dev/null' _ "$WS" 2>/dev/null
assert_eq "$(hook_stop_cmd "$MIG_HOME/claude-settings.json")" "${(q)WS} hook stop" "launch re-points claude-settings.json"
assert_eq "$([[ "$MIG_HOME/bin/ws" -ef "$WS" ]] && print yes)" "yes" "launch re-links a dangling bin/ws"
assert_eq "$(T list-keys -T prefix a | grep -cF -- "$WS")" "1" "launch re-binds tmux keys"
T kill-session -t mig-bind 2>/dev/null

echo "ws migrate: an unusable ws cannot be wired into Grok but still migrates Claude settings"
COPY="$TMP/ws-copy"
cp -R "$ROOT/ws" "$COPY" && chmod -x "$COPY/ws.sh"
stale_hooks "/old/dotfile/ws.sh"
WS_HOME="$MIG_HOME" GROK_HOME="$MIG_GROK" zsh "$COPY/ws.sh" _migrate-hooks 2>/dev/null
assert_exit "$?" "1" "_migrate-hooks reports the Grok failure"
assert_eq "$(hook_stop_cmd "$MIG_GROK_HOOKS")" "/old/dotfile/ws.sh hook stop" "Grok hooks are not wired to a non-executable ws"
assert_eq "$(hook_stop_cmd "$MIG_HOME/claude-settings.json")" "${(q)COPY}/ws.sh hook stop" "claude-settings.json still migrates"
WS_HOME="$MIG_HOME" GROK_HOME="$MIG_GROK" zsh "$COPY/ws.sh" open no/such-workspace 2>/dev/null
assert_exit "$?" "1" "startup survives the Grok failure (open itself fails as usual)"
