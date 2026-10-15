#!/bin/zsh
# ws tests: List, add, resume by id, agent command building, Grok hooks, clean-exit forgetting.
# Not standalone: ws/tests/run.sh sources helpers.sh then its group's area files in order, in one
# process, because workspaces, tabs and global session ids carry over within a group.
(( $+functions[ws_tests_require_sandbox] )) && ws_tests_require_sandbox || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

echo "ws list: rows for open and window-less workspaces"
git -C "$REPO" worktree add -q -b orphan "$WS_ROOT/api/orphan"
out="$("$WS" list)"
assert_contains "$out" $'waiting\tapi/feat-login\t'"$WT"$'\tclaude\t'"$WIN" "open session row w/ state, agent, window"
assert_contains "$out" $'stopped\tapi/orphan\t' "worktree without window is stopped"

echo "ws add: second session on the same worktree; resumed by id after a restart"
"$WS" add api/feat-login --agent codex --detach
W2="$(slot_window "$WT" 2)"
assert_eq "$(session_of "$W2")" "api/feat-login" "second session is a tab in the worktree's tmux session"
assert_eq "$(T display -p -t "$W2" '#{window_name}')" "codex#2" "tab named agent#slot"
assert_contains "$("$WS" list)" $'api/feat-login#2\t' "list names extra sessions repo/branch#slot"
out="$(wait_pane "$W2" FAKE-codex)"
assert_contains "$out" "--dangerously-bypass-approvals-and-sandbox" "codex bypasses approvals"
assert_contains "$out" "hooks.Stop" "codex gets Stop hook override"
CODEX_ID="$(T show -wqv -t "$W2" @ws_session)"
assert_agent_preamble codex "$CODEX_ID"
print '{"session_id":"sid-claude-1"}' | hook api/feat-login "$WIN" idle
print '{"session_id":"sid-codex-2"}' | hook api/feat-login "$W2" idle
SF="$(git -C "$WT" rev-parse --absolute-git-dir)/ws-sessions"
for i in {1..8}; do # parallel tool calls fire PostToolUse hooks concurrently
  print '{"session_id":"sid-claude-1"}' | hook api/feat-login "$WIN" working &
  print '{"session_id":"sid-codex-2"}' | hook api/feat-login "$W2" working &
done
wait
assert_eq "$(cut -f1,3 "$SF" | tr '\t\n' ': ')" "1:sid-claude-1 2:sid-codex-2 " "concurrent hooks keep every session row"
T kill-window -t "$WIN"; T kill-window -t "$W2"
rm -f "$WS_HOME/argv-1.json" "$WS_HOME/argv-$CODEX_ID.json"
assert_eq "$(windows_of "$WT")" "" "both windows gone (simulated reboot)"
assert_contains "$("$WS" list)" $'stopped\tapi/feat-login\t' "workspace shows stopped"
"$WS" open reviewer --detach
assert_exit "$?" "0" "named stopped session resumes independently"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" FAKE-claude)" "--resume sid-claude-1" "named resume uses its saved conversation id"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" FAKE-claude)" "ID=1 WHOAMI=1" "named resume preserves global identity"
assert_eq "$(slot_window "$WT" 2)" "" "resuming one id leaves other sessions stopped"
T kill-window -t "$(slot_window "$WT" 1)"
rm -f "$WS_HOME/argv-1.json"
"$WS" open api/feat-login --detach
wait_until slot_present "$WT" 2
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" sid-claude-1)" "--resume sid-claude-1" "claude session resumed by id"
assert_contains "$(wait_pane "$(slot_window "$WT" 2)" sid-codex-2)" "resume sid-codex-2" "codex session resumed by id"
assert_agent_preamble claude 1
assert_agent_preamble codex "$CODEX_ID"
assert_eq "$(session_of "$(slot_window "$WT" 2)")" "api/feat-login" "restored into one worktree session"
assert_contains "$(wait_pane "$(slot_window "$WT" 1)" FAKE-claude)" "ID=1 WHOAMI=1" "workspace resume keeps the same named global identity"

echo "ws agent_command: prompts stay positional; codex hooks survive a spaced WS_BIN"
# agent_argv <agent> <resume> <prompt> — JSON argv of the generated launch command.
agent_argv() {
  ( source "$WS"
    WS_BIN="$TMP/ws bin/it's ws.sh" WS_HOME="$TMP/argv-home" GROK_HOME="$TMP/argv-grok"
    dump() { python3 -c 'import json,sys; json.dump(sys.argv[1:],sys.stdout)' "$@"; }
    claude() { dump "$@"; }; codex() { dump "$@"; }; grok() { dump "$@"; }
    eval "$(agent_command "$1" "$2" "$3" 9)" )
}
for a in claude codex grok; do
  assert_eq "$(agent_argv "$a" "" --help | jq -r '.[-2:] | join(" ")')" "-- --help" "$a option-like prompt follows --"
  assert_eq "$(agent_argv "$a" r1 completion | jq -r '.[-2:] | join(" ")')" "-- completion" "$a resume prompt follows --"
  assert_eq "$(agent_argv "$a" "" "" | jq -r 'index("--")')" "null" "$a without prompt has no --"
done
assert_eq "$(agent_argv codex r1 hi | jq -r '.[:2] | join(" ")')" "resume r1" "codex resume subcommand stays first"
hook_cmd="$(agent_argv codex "" "" | python3 -c '
import json, re, sys
arg = next(a for a in json.load(sys.stdin) if a.startswith("hooks.Stop="))
print(json.loads(re.search(r"command=(\"(?:[^\"\\\\]|\\\\.)*\")", arg).group(1)))')"
assert_eq "$(zsh -fc 'print -rl -- ${(Q)${(z)1}}' _ "$hook_cmd")" "$TMP/ws bin/it's ws.sh"$'\nhook\nstop' "codex hook command shell-quotes a spaced WS_BIN"
mkdir -p "$TMP/ws bin" && printf '#!/bin/sh\n' > "$TMP/ws bin/it's ws.sh" && chmod +x "$TMP/ws bin/it's ws.sh"
( source "$WS"
  WS_BIN="$TMP/ws bin/it's ws.sh" WS_HOME="$TMP/argv-home" GROK_HOOKS="$TMP/argv-grok/hooks/ws.json"
  write_claude_settings; write_grok_hooks )
for f in "$TMP/argv-home/claude-settings.json" "$TMP/argv-grok/hooks/ws.json"; do
  hook_cmd="$(jq -r '.hooks.Stop[0].hooks[0].command' "$f")"
  assert_eq "$(zsh -fc 'print -rl -- ${(Q)${(z)1}}' _ "$hook_cmd")" "$TMP/ws bin/it's ws.sh"$'\nhook\nstop' "${f:t} hook command shell-quotes a spaced WS_BIN"
done
echo "ws grok hooks: never wired to a missing/non-executable WS_BIN"
print -r -- '{"keep":true}' > "$TMP/argv-grok/hooks/keep.json"
for bad in "$TMP/ws bin/missing.sh" "$TMP/ws bin"; do
  err="$( ( source "$WS"; WS_BIN="$bad" GROK_HOOKS="$TMP/argv-grok/hooks/keep.json"; write_grok_hooks ) 2>&1 >/dev/null)"
  assert_exit "$?" "1" "grok hooks refuse WS_BIN ${bad:t}"
  assert_contains "$err" "not executable" "grok hooks explain refused WS_BIN ${bad:t}"
  assert_eq "$(cat "$TMP/argv-grok/hooks/keep.json")" '{"keep":true}' "grok hooks file untouched for WS_BIN ${bad:t}"
done

echo "ws grok: launched w/ --always-approve + global ws hook file; resumed by GROK_SESSION_ID"
"$WS" new --repo "$REPO" --branch grokky --agent grok --detach
GWT="$WS_ROOT/api/grokky"
GW="$(slot_window "$GWT" 1)"
assert_eq "$(T display -p -t "$GW" '#{window_name}')" "grok" "grok tab named after agent"
assert_contains "$(wait_pane "$GW" FAKE-grok)" "--always-approve" "grok skips approvals"
GROK_ID="$(T show -wqv -t "$GW" @ws_session)"
assert_agent_preamble grok "$GROK_ID"
GH="$GROK_HOME/hooks/ws.json"
assert_eq "$(jq -r '.hooks.StopCancelled[0].hooks[0].command' "$GH" 2>/dev/null)" "$WS hook cancel" "grok hook file: StopCancelled → turn-checked cancel"
assert_eq "$(jq -r '.hooks.Notification[0].matcher' "$GH" 2>/dev/null)" "permission_prompt" "grok hook file: permission prompt → waiting"
"$WS" add api/grokky --agent grok --detach
GW2="$(slot_window "$GWT" 2)"
assert_eq "$(T display -p -t "$GW2" '#{window_name}')" "grok#2" "second grok tab named grok#2"
GROK_SESSION_ID=sid-grok-2 hook api/grokky "$GW2" idle </dev/null
GROK_RESUME_ID="$(T show -wqv -t "$GW2" @ws_session)"
T kill-window -t "$GW"; T kill-window -t "$GW2"
rm -f "$WS_HOME/argv-$GROK_RESUME_ID.json"
"$WS" open api/grokky --detach
wait_until slot_present "$GWT" 2
assert_contains "$(wait_pane "$(slot_window "$GWT" 2)" sid-grok-2)" "--resume sid-grok-2" "grok resumed by GROK_SESSION_ID"
assert_agent_preamble grok "$GROK_RESUME_ID"
"$WS" add api/grokky --agent gemini --detach 2>/dev/null
assert_exit "$?" "1" "unknown agent refused"
"$WS" rm api/grokky --force --delete-branch

echo "ws add: defaults to the worktree you're in"
before="$(windows_of "$WT" | wc -l | tr -d ' ')"
(cd "$WT" && "$WS" add --agent codex --detach)
assert_eq "$(windows_of "$WT" | wc -l | tr -d ' ')" "$((before + 1))" "add w/o target uses current worktree"
T kill-window -t "$(slot_window "$WT" 3)"
session_forget() { local f="$(git -C "$WT" rev-parse --absolute-git-dir)/ws-sessions"; awk -F '\t' '$1 != 3' "$f" > "$f.tmp" && mv "$f.tmp" "$f"; }
session_forget

echo "ws: quitting an agent cleanly forgets its session"
W2="$(slot_window "$WT" 2)"
T send-keys -t "$W2" Enter
wait_until slot_absent "$WT" 2
assert_eq "$(slot_window "$WT" 2)" "" "window closes when agent exits"
T kill-window -t "$(slot_window "$WT" 1)"
assert_eq "$(T has-session -t '=api/feat-login' 2>/dev/null && print alive)" "" "worktree session gone with its last tab"
"$WS" open api/feat-login --detach
sleep 0.3
assert_eq "$(windows_of "$WT" | wc -l | tr -d ' ')" "1" "only the surviving session is restored"
