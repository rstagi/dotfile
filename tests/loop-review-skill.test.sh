#!/bin/zsh
# loop-review skill: prose contract + one-round step tests with a fake ws.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
SKILL_DIR="$ROOT/.claude/skills/loop-review"
SKILL="$(cat "$SKILL_DIR/SKILL.md")"
STEP="$SKILL_DIR/step.sh"
TMP="$(mktemp -d /tmp/loop-review-skill.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT
trap 'exit 1' INT TERM HUP
unset WS_LOOP_ID WS_SESSION_ID WS_SESSION LOOP_DAEMON_URL

echo "loop-review skill: prose contract"
assert_contains "$SKILL" 'name: loop-review' "frontmatter name"
assert_contains "$SKILL" 'a PR, a plan, a document or a file' "targets"
assert_contains "$SKILL" '--publish-output' "runner publishes output"
assert_contains "$SKILL" '--review-key' "runner gets the step key"
assert_contains "$SKILL" 'review:<phase>:<repo or none>:<role>:<round>:<inner>:attempt:<k>' "step key form"
assert_contains "$SKILL" 'session_output' "reads output with session_output"
assert_contains "$SKILL" 'loop-ar.sh verdict' "verdict from loop-ar.sh"
assert_contains "$SKILL" 'You write no `loop.review` event.' "skill writes no review event"
assert_contains "$SKILL" 'A step stays pending until its result exists.' "pending rule"
assert_contains "$SKILL" 'start no new attempt for that step' "no attempt after a result"
assert_contains "$SKILL" 'a reviewer that exits with no report' "no-report case"
assert_contains "$SKILL" 'A final review that exits 10 with `COMMENT` is a result.' "final COMMENT terminal"
assert_contains "$SKILL" 'A runner checkpoint is not a result.' "checkpoint is not a result"
assert_contains "$SKILL" 'There is no confirmation by a third provider.' "no third provider"
assert_contains "$SKILL" 'The default is 2.' "rounds default"
assert_contains "$SKILL" 'goes straight to the final review' "clean round skips ahead"
assert_contains "$SKILL" '`security`, `unprotected-breakage`, `failing-check`' "blocking classes"
assert_contains "$SKILL" 'The fixer fixes only blocking findings.' "fixer scope"
assert_contains "$SKILL" 'loop-ar.sh disputed' "disputed sidecar"
assert_contains "$SKILL" 'idempotency marker' "final marker"
assert_contains "$SKILL" 'with a `Verdict:` line' "same-author COMMENT"
assert_contains "$SKILL" '`loop.decision`' "departure decision"
assert_contains "$SKILL" 'ws log <loop> --type loop.review' "recovery reads the loop log"

# Fake ws. FAKE_LOG is the loop.review log. `session create` appends a started record and
# counts one process for each distinct key, as the daemon does.
mkdir -p "$TMP/bin"
cat > "$TMP/bin/ws" <<'FAKE'
#!/bin/zsh
case "$1" in
log) if [[ -n "${FAKE_LOG_OBJECT:-}" ]]; then jq '{entries: .}' "$FAKE_LOG"; else cat "$FAKE_LOG"; fi ;;
session)
  [[ "$2" == create ]] || exit 2
  shift 2
  key="" ; args=("$@")
  for (( i = 1; i <= $#args; i++ )); do [[ "$args[i]" == --key ]] && key="$args[i+1]"; done
  print -r -- "$*" >> "$FAKE_CALLS"
  if ! grep -qxF "$key" "$FAKE_KEYS" 2>/dev/null; then
    print -r -- "$key" >> "$FAKE_KEYS"
    jq --arg key "$key" '. + [{type:"loop.review", body:{state:"started", key:$key}}]' "$FAKE_LOG" > "$FAKE_LOG.new" && mv "$FAKE_LOG.new" "$FAKE_LOG"
  fi
  print -r -- "01SESSION$(grep -nxF "$key" "$FAKE_KEYS" | cut -d: -f1)" ;;
*) exit 2 ;;
esac
FAKE
chmod +x "$TMP/bin/ws"
export PATH="$TMP/bin:$PATH"
export FAKE_LOG="$TMP/log.json" FAKE_KEYS="$TMP/keys" FAKE_CALLS="$TMP/calls"
export LOOP_REVIEW_RUNNER="$TMP/runner.sh"
reset() { print -r -- '[]' > "$FAKE_LOG"; : > "$FAKE_KEYS"; : > "$FAKE_CALLS"; }
add() { jq --argjson e "$1" '. + [{type:"loop.review", body:$e}]' "$FAKE_LOG" > "$FAKE_LOG.new" && mv "$FAKE_LOG.new" "$FAKE_LOG"; }
processes() { grep -c . "$FAKE_KEYS" | tr -d ' '; }
K='review:7:acme/api:review:1:1:attempt:'

echo "loop-review skill: one round with a fake ws"
reset
out="$(zsh "$STEP" state L1 7 acme/api review 1 1)"
assert_eq "$out" $'run\t'"${K}1" "a new step runs attempt 1"
out="$(zsh "$STEP" run L1 7 acme/api review 1 1 gpt -- --chain review-gpt --publish-output /x/report.md)"
assert_eq "$out" $'started\t'"${K}1"$'\t01SESSION1' "the keyed create starts attempt 1"
assert_contains "$(<"$FAKE_CALLS")" "--loop L1 --phase 7 --attempt 1 --repo acme/api --role review --provider gpt --round 1 --inner 1" "create carries the loop object"
assert_contains "$(<"$FAKE_CALLS")" "--review-key ${K}1 --chain review-gpt --publish-output /x/report.md" "runner gets the review key and its args"

echo "loop-review skill: recovery sends the same key while the attempt is open"
out="$(zsh "$STEP" run L1 7 acme/api review 1 1 gpt -- --chain review-gpt)"
assert_eq "$out" $'started\t'"${K}1"$'\t01SESSION1' "same key again, same session"
assert_eq "$(processes)" "1" "no second process"

echo "loop-review skill: ws log prints {entries:[...]}"
out="$(FAKE_LOG_OBJECT=1 zsh "$STEP" state L1 7 acme/api review 1 1)"
assert_eq "$out" $'run\t'"${K}1" "the object form of ws log gives the open attempt"
add '{"state":"finished","key":"'"${K}1"'","verdict":"APPROVE"}'
out="$(FAKE_LOG_OBJECT=1 zsh "$STEP" state L1 7 acme/api review 1 1)"
assert_eq "$out" $'terminal\tAPPROVE' "the object form of ws log gives the result"
jq 'map(select(.body.state != "finished"))' "$FAKE_LOG" > "$FAKE_LOG.new" && mv "$FAKE_LOG.new" "$FAKE_LOG"

echo "loop-review skill: a reviewer exits with no report"
add '{"state":"exited","key":"'"${K}1"'"}'
out="$(zsh "$STEP" state L1 7 acme/api review 1 1)"
assert_eq "$out" $'run\t'"${K}2" "an exited attempt without a result gets attempt 2"
zsh "$STEP" run L1 7 acme/api review 1 1 gpt -- --chain review-gpt >/dev/null
assert_eq "$(processes)" "2" "the next create starts one new process"
zsh "$STEP" run L1 7 acme/api review 1 1 gpt -- --chain review-gpt >/dev/null
assert_eq "$(processes)" "2" "attempt 2 is not started twice"

echo "loop-review skill: a result is terminal"
add '{"state":"exited","key":"'"${K}2"'"}'
add '{"state":"finished","key":"'"${K}2"':result","verdict":"NEEDS_CHANGES","role":"review"}'
out="$(zsh "$STEP" run L1 7 acme/api review 1 1 gpt -- --chain review-gpt)"
assert_eq "$out" $'terminal\tNEEDS_CHANGES' "a finished record ends the step"
assert_eq "$(processes)" "2" "no attempt after a result"

echo "loop-review skill: final review COMMENT is terminal"
reset
F='review:7:acme/api:final:2:0:attempt:'
zsh "$STEP" run L1 7 acme/api final 2 0 claude -- --chain review-final >/dev/null
add '{"state":"exited","key":"'"${F}1"'"}'
add '{"state":"finished","key":"'"${F}1"':result","verdict":"COMMENT","role":"final"}'
out="$(zsh "$STEP" run L1 7 acme/api final 2 0 claude -- --chain review-final)"
assert_eq "$out" $'terminal\tCOMMENT' "COMMENT ends the final review"
assert_eq "$(processes)" "1" "no second final review"

echo "loop-review skill: a checkpoint gets the next attempt"
reset
C='review:7:none:critic:1:2:attempt:'
zsh "$STEP" run L1 7 none critic 1 2 claude -- --chain review-claude >/dev/null
add '{"state":"exited","key":"'"${C}1"'"}'
out="$(zsh "$STEP" state L1 7 none critic 1 2)"
assert_eq "$out" $'run\t'"${C}2" "a checkpoint exit is not a result"

echo "loop-review skill: another step's result does not end this step"
reset
add '{"state":"finished","key":"review:7:acme/api:review:2:1:attempt:1:result","verdict":"APPROVE"}'
out="$(zsh "$STEP" state L1 7 acme/api review 1 1)"
assert_eq "$out" $'run\t'"${K}1" "round 2 result leaves round 1 pending"

test_summary
