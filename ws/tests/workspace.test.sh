#!/bin/zsh
# ws tests: Skill install, workspace creation, global session ids, names and legacy-id migration.
# Not standalone: ws/tests/run.sh sources helpers.sh then its group's area files in order, in one
# process, because workspaces, tabs and global session ids carry over within a group.
(( $+functions[ws_tests_require_sandbox] )) && ws_tests_require_sandbox || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

echo "ws skill: installer exposes the real session skill to Claude and Codex"
HOME="$TMP/skillhome" bash "$ROOT/install-agent-skills.sh" --no-ratel > "$TMP/skill-install.log" 2>&1
assert_exit "$?" "0" "shared skill installer succeeds in a private home"
for host in .claude .agents .codex; do
  assert_eq "$([[ "$TMP/skillhome/$host/skills/ws-sessions/SKILL.md" -ef "$ROOT/.claude/skills/ws-sessions/SKILL.md" ]] && print yes)" \
    "yes" "$host discovers the canonical ws-sessions skill"
done

REPO="$(make_repo api)"
print 'SECRET=1' > "$REPO/.env"

echo "ws new: creates worktree, copies env, opens a tmux session w/ one agent tab"
# An existing tmux server may predate the caller's WS_HOME override.
T set-environment -g WS_HOME "$TMP/stale-wshome"
"$WS" new --repo "$REPO" --branch feat/login --detach
assert_exit "$?" "0" "new succeeds"
T set-environment -g WS_HOME "$WS_HOME"
WT="$WS_ROOT/api/feat-login"
assert_eq "$(git -C "$WT" branch --show-current)" "feat/login" "worktree on new branch"
assert_eq "$(cat "$WT/.env" 2>/dev/null)" "SECRET=1" ".env copied from main checkout"
WIN="$(windows_of "$WT")"
assert_eq "${WIN:+found}" "found" "window tagged with worktree path"
assert_eq "$(session_of "$WIN")" "api/feat-login" "own tmux session named repo/branch"
assert_eq "$(T display -p -t "$WIN" '#{window_name}')" "claude" "agent tab named after agent"
assert_eq "$(T list-windows -t "$WIN" | wc -l | tr -d ' ')" "1" "session holds just the agent tab"
assert_eq "$(T list-panes -t "$WIN" | wc -l | tr -d ' ')" "1" "agent pane only"
out="$(wait_pane "$WIN" FAKE-claude)"
assert_contains "$out" "--dangerously-skip-permissions" "claude skips permissions"
assert_contains "$out" "--settings" "claude launched w/ ws hook settings"
assert_contains "$out" "WS=api/feat-login" "agent sees WS_WORKSPACE"

echo "ws sessions: new tabs have a global identity"
assert_contains "$out" "ID=1" "first agent gets global id in its environment"
assert_contains "$out" "WHOAMI=1" "agent can query its own id"
assert_agent_preamble claude 1
assert_contains "$("$WS" sessions)" $'1\t\tapi/feat-login\tclaude\tidle\tactive' "registry lists identity, workspace, agent, state and status"
env -u WS_SESSION "$WS" whoami 2>/dev/null
assert_exit "$?" "1" "whoami refuses outside a tracked session"

echo "ws name: labels address a session globally"
WS_SESSION=1 "$WS" name reviewer
assert_exit "$?" "0" "session can set its name"
assert_eq "$("$WS" sessions reviewer)" $'1\treviewer\tapi/feat-login\tclaude\tidle\tactive' "name resolves to the same registry entry"
"$WS" open reviewer --detach
assert_exit "$?" "0" "open accepts a session name"
"$WS" open 1 --detach
assert_exit "$?" "0" "open accepts a global id"
assert_eq "$(windows_of "$WT" | wc -l | tr -d ' ')" "1" "opening a live session focuses its existing tab"
WS_SESSION=1 "$WS" name 123 2>/dev/null
assert_exit "$?" "1" "numeric names cannot shadow ids"
assert_contains "$("$WS" sessions 1)" $'1\treviewer\t' "invalid rename preserves existing name"

echo "ws sessions: concurrent workspaces allocate distinct monotone ids"
global_pids=()
for i in {1..8}; do
  git -C "$REPO" worktree add -q -b "global-$i" "$WS_ROOT/api/global-$i" main
done
for i in {1..8}; do
  "$WS" add "api/global-$i" --detach > "$TMP/global-$i.log" 2>&1 &
  global_pids+=($!)
done
for pid in $global_pids; do
  wait "$pid"
  launch_exit=$?
  (( launch_exit == 0 )) || cat "$TMP"/global-*.log
  assert_exit "$launch_exit" "0" "concurrent workspace launches successfully"
done
assert_eq "$("$WS" sessions | awk -F '\t' '$3 ~ /^api\/global-/ {print $1}' | sort -n | tr '\n' ' ')" \
  "2 3 4 5 6 7 8 9 " "parallel allocation never duplicates or loses an id"
for i in {1..8}; do
  allocated_id="$("$WS" sessions | awk -F '\t' -v name="api/global-$i" '$3 == name {print $1}')"
  assert_contains "$(wait_pane "$(windows_of "$WS_ROOT/api/global-$i")" FAKE-claude)" \
    "WS=api/global-$i ID=$allocated_id WHOAMI=$allocated_id" "concurrent tab owns the identity of its workspace"
done
GLOBAL_SESSION_ID="$("$WS" sessions | awk -F '\t' '$3 == "api/global-1" {print $1}')"
GW="$(windows_of "$WS_ROOT/api/global-1")"
assert_contains "$(wait_pane "$GW" FAKE-claude)" "ID=$GLOBAL_SESSION_ID WHOAMI=$GLOBAL_SESSION_ID" "concurrent tab gets its own allocated identity"
WS_SESSION="$GLOBAL_SESSION_ID" "$WS" name reviewer 2>/dev/null
assert_exit "$?" "1" "duplicate session names are refused"
T kill-window -t "$GW"
assert_contains "$("$WS" sessions "$GLOBAL_SESSION_ID")" $'\tstopped\tactive' "killed session is stopped and remains active"
open_pids=()
for i in {1..16}; do
  "$WS" open "$GLOBAL_SESSION_ID" --detach > "$TMP/open-$i.log" 2>&1 &
  open_pids+=($!)
done
for pid in $open_pids; do
  wait "$pid"
  assert_exit "$?" "0" "concurrent resume succeeds"
done
for i in {1..16}; do
  assert_eq "$(cat "$TMP/open-$i.log")" "" "concurrent resume completes without errors"
done
assert_eq "$(windows_of "$WS_ROOT/api/global-1" | wc -l | tr -d ' ')" "1" "concurrent resume opens exactly one tab for a global id"
T kill-window -t "$(windows_of "$WS_ROOT/api/global-1")"
"$WS" open api/global-1 --detach
out="$(wait_pane "$(windows_of "$WS_ROOT/api/global-1")" FAKE-claude)"
assert_contains "$out" "ID=$GLOBAL_SESSION_ID WHOAMI=$GLOBAL_SESSION_ID" "workspace resume preserves global id without an agent resume id"
assert_eq "${out/*--resume*/resumed}" "$out" "global id is never passed as an agent resume id"
T send-keys -t "$(windows_of "$WS_ROOT/api/global-1")" Enter
registry_closed() { [[ "$("$WS" sessions "$1" | cut -f5,6)" == $'stopped\tclosed' ]]; }
wait_until registry_closed "$GLOBAL_SESSION_ID"
assert_exit "$?" "0" "clean exit closes the global record without deleting its identity"
"$WS" add api/global-1 --agent codex --detach
assert_contains "$(wait_pane "$(slot_window "$WS_ROOT/api/global-1" 1)" FAKE-codex)" "ID=10 WHOAMI=10" "reusing a workspace slot allocates a fresh global id"
"$WS" open "$GLOBAL_SESSION_ID" --detach
assert_exit "$?" "0" "closed global session can be reopened by id"
assert_contains "$(wait_pane "$(slot_window "$WS_ROOT/api/global-1" 2)" FAKE-claude)" \
  "ID=$GLOBAL_SESSION_ID WHOAMI=$GLOBAL_SESSION_ID" "closed resume preserves identity when its old slot is occupied"
assert_contains "$("$WS" sessions "$GLOBAL_SESSION_ID")" $'\tclaude\tidle\tactive' "reopened global record is active"
for i in {1..8}; do "$WS" rm "api/global-$i" --force --delete-branch >/dev/null; done

echo "ws sessions: legacy resume records acquire one stable global identity"
LEGACY_WT="$WS_ROOT/api/legacy"
git -C "$REPO" worktree add -q -b legacy "$LEGACY_WT" main
LEGACY_SF="$(git -C "$LEGACY_WT" rev-parse --absolute-git-dir)/ws-sessions"
printf '1\tclaude\tsid-legacy\n' > "$LEGACY_SF"
"$WS" open api/legacy --detach
assert_contains "$(wait_pane "$(slot_window "$LEGACY_WT" 1)" FAKE-claude)" "--resume sid-legacy" "legacy conversation resumes"
assert_contains "$(wait_pane "$(slot_window "$LEGACY_WT" 1)" FAKE-claude)" "ID=11 WHOAMI=11" "legacy tab acquires a global identity"
T kill-window -t "$(slot_window "$LEGACY_WT" 1)"
"$WS" open api/legacy --detach
assert_contains "$(wait_pane "$(slot_window "$LEGACY_WT" 1)" FAKE-claude)" "ID=11 WHOAMI=11" "legacy migration never allocates a second identity"
"$WS" rm api/legacy --force --delete-branch >/dev/null
