#!/bin/zsh
# Test plan routing through the public CLI with private backends.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
TMP="$(mktemp -d)"
WS_TEST_DAEMON_PID=""
cleanup() {
  if [[ -n "$WS_TEST_DAEMON_PID" ]]; then
    kill "$WS_TEST_DAEMON_PID" 2>/dev/null
    wait "$WS_TEST_DAEMON_PID" 2>/dev/null
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
unset WS_LOOP_ID WS_SESSION_ID WS_SESSION WS_BIN LOOP_DAEMON_URL TMUX TMUX_PANE
export HOME="$TMP/home" WS_HOME="$TMP/ws" WS_STATE="$TMP/state" WS_SOCK="$TMP/ws.sock"
export WS_TMUX_SOCKET="$TMP/tmux.sock" WS_NOTIFY=0
export LOOP_DAEMON_URL=http://127.0.0.1:9
export FAKE_PLAN_WS="$TMP/store" FAKE_PLAN_WEB="$TMP/web"
export LOOP_EMIT_SH="$HERE/fake/loop-plan-emit.sh" EMIT_LOG="$TMP/emit.log"
mkdir -p "$HOME" "$TMP/bin" "$FAKE_PLAN_WS" "$FAKE_PLAN_WEB" "$TMP/loop"
ln -s "$HERE/fake/loop-plan-ws.mjs" "$TMP/bin/ws"
ln -s "$HERE/fake/loop-plan-curl.sh" "$TMP/bin/curl"
print -r -- $'#!/bin/sh\nexit 1' > "$TMP/bin/launchctl"
chmod +x "$TMP/bin/launchctl"
export PATH="$TMP/bin:$PATH"
: > "$EMIT_LOG"
plan() { zsh "$ROOT/loop-plan.sh" "$@"; }
cat > "$TMP/loop/plan.md" <<'PLAN'
<!-- loop-plan
planId: skills-plan
engine: skills
daemon: http://127.0.0.1:9
-->
# Skills effort
PLAN

echo 'loop-plan: skills register preserves the file bytes'
OUT="$(plan register --dir "$TMP/loop" --effort skills </dev/null 2>"$TMP/error")"; rc=$?
assert_exit "$rc" 0 'register succeeds'
assert_eq "$OUT" skills-plan 'register prints the plan id'
assert_eq "$(cat "$EMIT_LOG")" '' 'register does not write to loop-web'
if [[ -f "$FAKE_PLAN_WS/store.json" ]]; then
  jq -j '.[0].plan.text' "$FAKE_PLAN_WS/store.json" > "$TMP/received"
  cmp -s "$TMP/received" "$TMP/loop/plan.md"
  assert_exit "$?" 0 'the event retains the final newline'
else
  assert_eq missing present 'register creates a wsd plan'
fi

echo 'loop-plan: get reads skills plans from wsd'
plan get --plan-id skills-plan --out "$TMP/download" 2>"$TMP/error"; rc=$?
assert_exit "$rc" 0 'get succeeds'
cmp -s "$TMP/download" "$TMP/loop/plan.md"
assert_exit "$?" 0 'get --out preserves the file bytes'
assert_eq "$(cat "$EMIT_LOG")" '' 'get does not start loop-web for a skills plan'

echo 'loop-plan: note appends the Phase 88 body'
NOTE='a "quoted" $note `x`'
plan note --plan-id skills-plan --phase 7 --body "$NOTE" 2>"$TMP/error"; rc=$?
assert_exit "$rc" 0 'note succeeds'
assert_eq "$(jq -r '.[0].notes[0].text' "$FAKE_PLAN_WS/store.json")" "$NOTE" 'note retains its text'
assert_eq "$(jq -r '.[0].notes[0].phase | type' "$FAKE_PLAN_WS/store.json")" number 'note has a numeric phase'
plan note --plan-id skills-plan --body 'General guidance' 2>"$TMP/error"; rc=$?
assert_exit "$rc" 0 'a note can omit its phase'
assert_eq "$(jq -r '.[0].notes[1].phase | type' "$FAKE_PLAN_WS/store.json")" null 'a general note has no phase'
assert_eq "$(cat "$EMIT_LOG")" '' 'note does not write to loop-web'

echo 'loop-plan: list chooses one row for each active engine'
print -r -- '{"runId":"skills-plan","status":"finished","effort":"stale web copy","planText":"web fallback"}' > "$FAKE_PLAN_WEB/skills-plan.json"
print -r -- '{"runId":"web-only","status":"planned","effort":"Legacy","planText":"legacy text"}' > "$FAKE_PLAN_WEB/web-only.json"
OUT="$(plan list 2>"$TMP/error")"; rc=$?
assert_exit "$rc" 0 'list succeeds'
assert_eq "$OUT" $'skills-plan  ·  planned  ·  Skills effort\nweb-only  ·  planned  ·  Legacy' 'list prints the skills row first and omits its stale web row'
OUT="$(FAKE_PLAN_WS_FAIL='loop list' plan list 2>"$TMP/error")"; rc=$?
assert_exit "$rc" 0 'list tolerates a ws failure'
assert_contains "$OUT" 'stale web copy' 'list uses only web rows when ws fails'
assert_eq "$(FAKE_PLAN_WS_FAIL='loop plan' plan get --plan-id skills-plan)" 'web fallback' 'get falls back when ws rejects loop plan'
assert_eq "$(WS_BIN="$TMP/missing-ws" plan get --plan-id web-only)" 'legacy text' 'get falls back when ws is missing'
plan get --plan-id unknown >/dev/null 2>&1
assert_exit "$?" 3 'get fails when both backends have no plan'

echo 'loop-plan: rollback retires the skills copy'
sed '/^engine: skills$/d' "$TMP/loop/plan.md" > "$TMP/rollback"
mv "$TMP/rollback" "$TMP/loop/plan.md"
: > "$EMIT_LOG"
WS_LOOP_ID=inherited-skills-session plan push --dir "$TMP/loop" --effort Rollback > "$TMP/push" 2>"$TMP/error"; rc=$?
assert_exit "$rc" 0 'rollback push succeeds inside a skills session'
assert_eq "$(jq -r '.[0].engine | type' "$FAKE_PLAN_WS/store.json")" null 'rollback clears the wsd engine'
assert_contains "$(cat "$EMIT_LOG")" 'emit:unset:skills-plan:register' 'rollback unsets WS_LOOP_ID for the web write'
jq '.planText = "authoritative web text"' "$FAKE_PLAN_WEB/skills-plan.json" > "$TMP/web-edit"
mv "$TMP/web-edit" "$FAKE_PLAN_WEB/skills-plan.json"
assert_eq "$(WS_LOOP_ID=inherited-skills-session plan get --plan-id skills-plan)" 'authoritative web text' 'get uses loop-web after rollback'
OUT="$(plan list)"
assert_eq "$OUT" $'skills-plan  ·  planned  ·  Rollback\nweb-only  ·  planned  ·  Legacy' 'list uses the web row after rollback'
WS_LOOP_ID=inherited-skills-session plan note --plan-id skills-plan --phase 7 --body 'Old engine note'
assert_eq "$(jq -r '.detail' "$FAKE_PLAN_WEB/notes.jsonl")" 'Old engine note' 'rollback notes reach loop-web'

echo 'loop-plan: skills push reuses a canonical directory session'
sed '/^planId:/a\
engine: skills
' "$TMP/loop/plan.md" > "$TMP/skills"
mv "$TMP/skills" "$TMP/loop/plan.md"
ln -s "$TMP/loop" "$TMP/loop-link"
ws loop create --dir "$TMP/loop-link" > "$TMP/existing-id"
plan push --dir "$TMP/loop" > "$TMP/push" 2>"$TMP/error"; rc=$?
assert_exit "$rc" 0 'skills push succeeds'
assert_eq "$(jq length "$FAKE_PLAN_WS/store.json")" 1 'push reuses the session from the canonical loop directory'
BEFORE="$(jq -r '.[0].appends' "$FAKE_PLAN_WS/store.json")"
plan push --dir "$TMP/loop" > "$TMP/push" 2>"$TMP/error"; rc=$?
assert_exit "$rc" 0 'the repeated push succeeds'
assert_eq "$(jq -r '.[0].appends' "$FAKE_PLAN_WS/store.json")" "$BEFORE" 'the repeated push appends no plan event'

echo 'loop-plan: skills write failures do not fall back to loop-web'
: > "$EMIT_LOG"
OUT="$(FAKE_PLAN_WS_FAIL='loop create' plan push --dir "$TMP/loop" 2>&1)"; rc=$?
assert_exit "$rc" 3 'a failed loop create exits 3'
assert_contains "$OUT" 'ws: fixture failure' 'push prints the ws error'
OUT="$(FAKE_PLAN_WS_FAIL='event loop.plan' plan push --dir "$TMP/loop" 2>&1)"; rc=$?
assert_exit "$rc" 3 'a failed plan append exits 3'
assert_contains "$OUT" 'ws: fixture failure' 'push prints the append error'
OUT="$(FAKE_PLAN_WS_FAIL='event loop.note' plan note --plan-id skills-plan --phase 1 --body Failed 2>&1)"; rc=$?
assert_exit "$rc" 3 'a failed note append exits 3'
assert_contains "$OUT" 'ws: fixture failure' 'note prints the append error'
assert_eq "$(cat "$EMIT_LOG")" '' 'failed skills writes never reach loop-web'

echo 'loop-plan: archive sends its transformed skills plan to wsd'
ARCHIVE="$TMP/archive"; mkdir -p "$ARCHIVE"
sed '/^planId:/c\
planId: archive-skills
' "$HERE/fixtures/milestones/plan.md" | sed '/^daemon:/c\
engine: skills
' > "$ARCHIVE/plan.md"
cat > "$TMP/bin/gh" <<'GH'
#!/bin/sh
printf '{"state":"MERGED"}\n'
GH
chmod +x "$TMP/bin/gh"
plan archive --dir "$ARCHIVE" --milestone 1 > "$TMP/archive.out" 2>"$TMP/error"; rc=$?
assert_exit "$rc" 0 'skills archive succeeds'
plan get --plan-id archive-skills --out "$TMP/archive.download"
cmp -s "$TMP/archive.download" "$ARCHIVE/plan.md"
assert_exit "$?" 0 'archive pushes the transformed plan to wsd'
assert_eq "$(cat "$EMIT_LOG")" '' 'skills archive never reaches loop-web'

echo 'loop-plan: old plans do not require ws'
OLD="$TMP/old"; mkdir -p "$OLD"
print -r -- $'<!-- loop-plan\nplanId: old-plan\n-->\n# Old plan\nengine: skills' > "$OLD/plan.md"
WS_BIN="$TMP/missing-ws" WS_LOOP_ID=inherited plan register --dir "$OLD" --effort Old > "$TMP/old.out"
assert_eq "$(cat "$TMP/old.out")" old-plan 'an engine line outside the header does not select wsd'
assert_contains "$(cat "$EMIT_LOG")" 'ensure:unset' 'old plans unset WS_LOOP_ID for daemon startup'
assert_eq "$(WS_BIN="$TMP/missing-ws" plan get --plan-id old-plan)" "$(cat "$OLD/plan.md")" 'old plans work when ws is absent'

echo 'loop-plan: wsd accepts the SHA and deduplicates the exact file bytes'
start_ws_test_daemon || exit 1
export WS_BIN="$WS_RUST_BIN"
REAL="$TMP/real"; mkdir -p "$REAL"
print -r -- $'<!-- loop-plan\nplanId: real-skills-plan\nengine: skills\n-->\n# Real skills plan\n' > "$REAL/plan.md"
REAL_ID="$("$WS_BIN" loop create --dir "$REAL")" || exit 1
plan push --dir "$REAL" > "$TMP/real.out" 2>"$TMP/error"; rc=$?
assert_exit "$rc" 0 'the real wsd accepts a skills push with final newlines'
"$WS_BIN" log "$REAL_ID" --type loop.plan > "$TMP/first-log"
plan push --dir "$REAL" > "$TMP/real.out" 2>"$TMP/error"; rc=$?
assert_exit "$rc" 0 'the real wsd accepts the repeated push'
"$WS_BIN" log "$REAL_ID" --type loop.plan > "$TMP/second-log"
cmp -s "$TMP/first-log" "$TMP/second-log"
assert_exit "$?" 0 'a second push adds no loop.plan event to the real wsd'
plan get --plan-id real-skills-plan --out "$TMP/real.download"
cmp -s "$TMP/real.download" "$REAL/plan.md"
assert_exit "$?" 0 'the real wsd retains every final newline'
assert_eq "$("$WS_BIN" loop list --json | jq length)" 1 'push reuses the existing loop session'
plan note --plan-id real-skills-plan --phase 8 --body 'Real guidance' 2>"$TMP/error"
assert_exit "$?" 0 'the real wsd accepts the Phase 88 note body'
plan note --plan-id real-skills-plan --body 'General guidance' 2>"$TMP/error"
assert_exit "$?" 0 'the real wsd accepts a note with no phase'

test_summary
