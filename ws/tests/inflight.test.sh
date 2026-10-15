#!/bin/zsh
# ws tests: Stop delivery ack, in-flight reclaim, resume-id ownership and stale-launch hooks.
# Not standalone: ws/tests/run.sh sources helpers.sh then its group's area files in order, in one
# process, because workspaces, tabs and global session ids carry over within a group.
(( $+functions[ws_tests_require_sandbox] )) && ws_tests_require_sandbox || { print -u2 "run via: zsh ws/tests/run.sh"; exit 2; }

echo "ws stop: in-flight inbox from an interrupted Stop is redelivered first"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
print -r -- '{"message":"stale"}' > "$WS_HOME/sessions/$SOURCE_A/inbox.inflight"
print -rn -- fresh | "$WS" send --force "$SOURCE_A" - 2>/dev/null
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'stale\n\n[ws terminal]\nfresh' "stale in-flight messages precede new ones"
assert_eq "$([[ -e "$WS_HOME/sessions/$SOURCE_A/inbox.inflight" ]] && print yes)" "" "in-flight file removed after reply"

# --- G1: Stop delivery ack + resume-id ownership ---
inflight_count() { ls "$WS_HOME/sessions/$1" | grep -c '^inbox\.inflight'; }
echo "ws stop: a reply that never reached the agent stays in flight and is redelivered"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
print -rn -- lost-reply | "$WS" send --force "$SOURCE_A" - 2>/dev/null
hook api/triggers "$SOURCE_A_WIN" stop </dev/null 2>/dev/null | head -c 0 # hook dies (SIGPIPE) writing its reply
assert_eq "$(inflight_count "$SOURCE_A")" "1" "unwritten reply keeps its in-flight file"
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'[ws terminal]\nlost-reply' "next Stop redelivers the unacked reply"
assert_eq "$(inflight_count "$SOURCE_A")" "0" "written reply acks its in-flight file"

echo "ws stop: a live Stop's in-flight messages are never reclaimed by a concurrent send"
sleep 30 &
owner_pid=$!
print -r -- '{"message":"owned"}' > "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$owner_pid"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
print -rn -- concurrent | "$WS" send --force "$SOURCE_A" - 2>/dev/null
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'[ws terminal]\nconcurrent' "live owner's messages are not redelivered"
assert_eq "$(cat "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$owner_pid")" '{"message":"owned"}' "live owner's in-flight file untouched"
kill "$owner_pid"; wait "$owner_pid" 2>/dev/null
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "owned" "dead owner's in-flight messages are redelivered"
assert_eq "$(inflight_count "$SOURCE_A")" "0" "reclaimed in-flight file removed"

echo "ws hook: a stale hook never rewrites a reused slot's resume row"
# resume_row <pane-session-id> <sid> — sessions-file row after record_resume_id for slot 1.
resume_row() {
  ( source "$WS"
    sessions_file() { print -r -- "$TMP/g1-ws-sessions"; }
    tmux_() {
      case "$*" in
        *@ws_path*) print -r -- "$TMP/g1-wt" ;;
        *@ws_slot*) print 1 ;;
        *@ws_agent*) print claude ;;
        *@ws_session*) print -r -- "$G1_PANE_ID" ;;
      esac
    }
    G1_PANE_ID="$1" TMUX_PANE=%0 record_resume_id "{\"session_id\":\"$2\"}"
    cat "$TMP/g1-ws-sessions" )
}
( source "$WS"
  f="$TMP/g1-ws-sessions"
  session_append "$f" claude "$TMP/g1-wt" >/dev/null # slot 1 → old session
  session_remove_locked "$f" 1
  session_append "$f" claude "$TMP/g1-wt" >/dev/null ) # slot 1 reused → new session
G1_OLD="$(( $(cat "$WS_HOME/sessions/counter") - 1 ))"
G1_NEW="$(cat "$WS_HOME/sessions/counter")"
assert_eq "$(resume_row "$G1_OLD" sid-old)" "1	claude		$G1_NEW" "stale hook leaves the new owner's row intact"
assert_eq "$(resume_row "$G1_NEW" sid-new)" "1	claude	sid-new	$G1_NEW" "current owner records its resume id"
assert_eq "$(resume_row "$G1_OLD" sid-old)" "1	claude	sid-new	$G1_NEW" "stale hook never replaces a recorded owner"
rm -f "$TMP/g1-ws-sessions" "$TMP/g1-ws-sessions.lock"
# --- end G1 ---

# --- G3b: stale-launch hooks, pid-reuse inflight reclaim, Stop lock budget ---
g3b_resume() { jq -r '.resume_id // empty' "$WS_HOME/sessions/$1/meta"; }
echo "ws hook: a delayed hook from an older launch on a reused pane touches nothing"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
print -rn -- for-new-launch | "$WS" send --force "$SOURCE_A" - 2>/dev/null
G3B_INBOX="$(cat "$WS_HOME/sessions/$SOURCE_A/inbox")"
G3B_RESUME="$(g3b_resume "$SOURCE_A")"
reply="$(print -r -- '{"session_id":"sid-stale-launch"}' | WS_SESSION="$SOURCE_B" hook api/triggers "$SOURCE_A_WIN" stop)"
assert_eq "$reply" "{}" "stale Stop replies {}"
assert_eq "$(cat "$WS_HOME/sessions/$SOURCE_A/inbox")" "$G3B_INBOX" "stale Stop leaves the pane's inbox untouched"
assert_eq "$(inflight_count "$SOURCE_A")" "0" "stale Stop drains nothing in flight"
assert_eq "$(g3b_resume "$SOURCE_A")" "$G3B_RESUME" "stale Stop keeps the pane's resume id"
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" $'[ws terminal]\nfor-new-launch' "current launch still receives its inbox"

echo "ws stop: an old in-flight file whose pid was reused is reclaimed"
sleep 30 &
g3b_pid=$!
print -r -- '{"message":"pid-reused"}' > "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$g3b_pid"
touch -t 202001010000 "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$g3b_pid"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "pid-reused" "stale in-flight file is redelivered despite a live pid"
assert_eq "$(inflight_count "$SOURCE_A")" "0" "stale in-flight file reclaimed"
echo "ws done: reclaims stale in-flight files before refusing"
print -r -- '{"message":"pid-reused-done"}' > "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$g3b_pid"
touch -t 202001010000 "$WS_HOME/sessions/$SOURCE_A/inbox.inflight.$g3b_pid"
WS_SESSION="$SOURCE_A" "$WS" done 2>/dev/null
assert_exit "$?" "1" "done still refuses while messages are pending"
assert_eq "$(inflight_count "$SOURCE_A")" "0" "done requeued the stale in-flight file"
assert_eq "$(jq -r '.message' "$WS_HOME/sessions/$SOURCE_A/inbox")" "pid-reused-done" "requeued message waits in the inbox"
kill "$g3b_pid"; wait "$g3b_pid" 2>/dev/null
hook api/triggers "$SOURCE_A_WIN" working </dev/null
reply="$(hook api/triggers "$SOURCE_A_WIN" stop </dev/null)"
assert_eq "$(print -r -- "$reply" | jq -r '.reason // empty')" "pid-reused-done" "next Stop delivers the requeued message"

echo "ws hook: Stop stays inside the hook timeout when resume-id locks are held"
G3B_META="$WS_HOME/sessions/$SOURCE_A/meta"
G3B_SESS="$(git -C "$(T display -p -t "$SOURCE_A_WIN" '#{@ws_path}')" rev-parse --absolute-git-dir)/ws-sessions"
: >> "$G3B_META.lock"; : >> "$G3B_SESS.lock"
zsh -c 'zmodload zsh/system; zsystem flock -f a "$1.lock" && zsystem flock -f b "$2.lock" && : > "$3" && sleep 8' g3b "$G3B_META" "$G3B_SESS" "$TMP/g3b-held" &
g3b_holder=$!
wait_until test -e "$TMP/g3b-held"
G3B_RESUME="$(g3b_resume "$SOURCE_A")"
hook api/triggers "$SOURCE_A_WIN" working </dev/null
zmodload zsh/datetime
g3b_t0=$EPOCHREALTIME
reply="$(print -r -- '{"session_id":"sid-under-contention"}' | hook api/triggers "$SOURCE_A_WIN" stop 2>/dev/null)"
g3b_elapsed=$(( EPOCHREALTIME - g3b_t0 ))
assert_eq "$reply" "{}" "contended Stop still replies"
assert_eq "$(( g3b_elapsed < 2 ))" "1" "contended Stop finishes well under 5s (took ${g3b_elapsed}s)"
assert_eq "$(g3b_resume "$SOURCE_A")" "$G3B_RESUME" "contended Stop skips the resume-id update"
kill "$g3b_holder"; wait "$g3b_holder" 2>/dev/null
rm -f "$TMP/g3b-held"
# --- end G3b ---
