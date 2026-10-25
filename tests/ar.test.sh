#!/bin/zsh
# AR public contract: report parsing, rotation, and replay of completed ledger rows.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
source "$ROOT/loop-ar.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
REPORT="$TMP/report.md"
LEDGER="$TMP/review.ledger.tsv"
T=$'\t'

echo "ar: last non-empty verdict line"
printf 'F1: example\nNEEDS_CHANGES: broken replay\n\n \t\n' > "$REPORT"
assert_eq "$(ar_parse_verdict "$REPORT" reviewer)" NEEDS_CHANGES "reviewer summary becomes token"
printf 'NEEDS_CHANGES: old finding\nAPPROVE\n' > "$REPORT"
assert_eq "$(ar_parse_verdict "$REPORT" reviewer)" APPROVE "only last verdict counts"
for verdict in AGREE 'DISAGREE_EVIDENCE: code refutes F1' 'DISAGREE_CONCERN: missed F2'; do
  print -r -- "$verdict" > "$REPORT"
  assert_eq "$(ar_parse_verdict "$REPORT" critic)" "${verdict%%:*}" "critic token: $verdict"
done
for verdict in 'APPROVE followed by prose' 'NEEDS_CHANGES:' 'DISAGREE_EVIDENCE: ' 'AGREE'; do
  print -r -- "$verdict" > "$REPORT"
  actual="$(ar_parse_verdict "$REPORT" reviewer)"; rc=$?
  assert_eq "$actual" invalid "malformed reviewer verdict: $verdict"
  assert_exit "$rc" 2 "invalid verdict exits 2"
done
printf 'APPROVE\ntrailing prose\n' > "$REPORT"
actual="$(ar_parse_verdict "$REPORT" reviewer)"; rc=$?
assert_eq "$actual" invalid "earlier valid line cannot hide trailing prose"
assert_exit "$rc" 2 "trailing prose exits 2"

echo "ar: provider rotation"
assert_eq "$(ar_rotation 1)" "gpt${T}claude${T}grok" "round 1 R/C/T"
assert_eq "$(ar_rotation 2)" "claude${T}grok${T}gpt" "round 2 R/C/T"
assert_eq "$(ar_rotation 3)" "grok${T}gpt${T}claude" "round 3 R/C/T"
assert_eq "$(ar_rotation 4)" "gpt${T}claude${T}grok" "rotation wraps"
ar_rotation 0 > /dev/null 2>&1; assert_exit "$?" 1 "round zero rejected"

echo "ar: a clean round ends the review without a third provider"
record() {
  local count=0
  [[ -f "$LEDGER" ]] && count=$(wc -l < "$LEDGER")
  printf 'a%d\t%s\t%s\t%s\t%s\truns/a%d\n' "$(( count + 1 ))" "$1" "$2" "$3" "$4" "$(( count + 1 ))" >> "$LEDGER"
}
next() { ar_next --rounds 3 --inner 2 --ledger "$LEDGER" "$@"; }
assert_eq "$(next)" "round1${T}review${T}gpt" "missing ledger starts review"
record round1 review gpt APPROVE
assert_eq "$(next)" "round1${T}critic${T}claude" "opening review goes to critic"
record round1 critic claude AGREE
assert_eq "$(next)" "final${T}final${T}claude" "clean pair skips later rounds without T"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" clean "clean pair is clean"
record final final claude APPROVE
assert_eq "$(next)" done "posted final ends pipeline"

echo "ar: findings, fix completion, and three outer rounds"
rm -f "$LEDGER"
record round1 review gpt NEEDS_CHANGES
record round1 critic claude AGREE
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" findings "converged flags are findings"
assert_eq "$(next)" "fix1${T}fix${T}claude" "findings before last round require fix"
assert_eq "$(next --rounds 1)" "final${T}final${T}claude" "one outer round goes directly to final"
record fix1 fix claude NOOP
assert_eq "$(next)" "round2${T}review${T}claude" "NOOP fix advances to round 2"
record round2 review claude NEEDS_CHANGES
assert_eq "$(next)" "round2${T}critic${T}grok" "round 2 critic rotates"
record round2 critic grok AGREE
assert_eq "$(next)" "fix2${T}fix${T}claude" "round 2 findings require fix"
record fix2 fix claude PUSHED
assert_eq "$(next)" "round3${T}review${T}grok" "pushed fix advances to round 3"
record round3 review grok NEEDS_CHANGES
assert_eq "$(next)" "round3${T}critic${T}gpt" "round 3 critic rotates"
record round3 critic gpt AGREE
assert_eq "$(next)" "final${T}final${T}claude" "last round omits fix"
record final final claude REQUEST_CHANGES
assert_eq "$(next)" done "request-changes final also completes"

echo "ar: disagreement converges after one response; replay is deterministic"
rm -f "$LEDGER"
record round1 review gpt NEEDS_CHANGES
record round1 critic claude DISAGREE_EVIDENCE
assert_eq "$(next)" "round1${T}respond${T}gpt" "disagreement gets a reviewer response"
cp "$LEDGER" "$TMP/replay.tsv"
assert_eq "$(ar_next --rounds 3 --inner 2 --ledger "$TMP/replay.tsv")" \
  "round1${T}respond${T}gpt" "fresh ledger replay yields same step"
assert_eq "$(next)" "round1${T}respond${T}gpt" "asking for next does not mutate ledger"
record round1 respond gpt NEEDS_CHANGES
assert_eq "$(next)" "round1${T}critic${T}claude" "response returns to critic"
record round1 critic claude AGREE
assert_eq "$(next)" "fix1${T}fix${T}claude" "agreement after response converges"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" findings "remaining flags need fixes"

echo "ar: legacy T veto rows still replay"
rm -f "$LEDGER"
record round1 review gpt APPROVE
record round1 critic claude AGREE
record round1 confirm grok DISAGREE_CONCERN
assert_eq "$(next)" "round1${T}respond${T}gpt" "T veto requests response"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" findings "vetoed round is no longer clean"
record round1 respond gpt APPROVE
assert_eq "$(next)" "round1${T}critic${T}grok" "T becomes critic after veto"
record round1 critic grok AGREE
assert_eq "$(next)" "final${T}final${T}claude" "resolved legacy veto is a clean round"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" clean "resolved veto leaves no blocking finding"
rm -f "$LEDGER"
record round1 review gpt APPROVE
record round1 critic claude AGREE
record round1 confirm grok DISAGREE_EVIDENCE
record round1 respond gpt NEEDS_CHANGES
record round1 critic grok AGREE
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" findings "veto that R accepts is a blocking finding"

echo "ar: inner cap preserves disputes, including critic-only issues"
for reviewer_verdict in NEEDS_CHANGES APPROVE; do
  rm -f "$LEDGER"
  record round1 review gpt "$reviewer_verdict"
  record round1 critic claude DISAGREE_CONCERN
  assert_eq "$(next --inner 1)" "disputed${T}disputed${T}-" "cap 1 stops without response: $reviewer_verdict"
  assert_eq "$(next)" "round1${T}respond${T}gpt" "cap 2 permits exactly one response"
  record round1 respond gpt "$reviewer_verdict"
  record round1 critic claude DISAGREE_EVIDENCE
  assert_eq "$(next)" "disputed${T}disputed${T}-" "second disagreement exhausts cap"
  record disputed disputed - DISPUTED
  print -r -- 'F2: critic-only unresolved issue' > "${LEDGER%.tsv}.disputed.md"
  assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" disputed "dispute outranks reviewer verdict"
  assert_eq "$(next)" "fix1${T}fix${T}claude" "disputed round requires fix: $reviewer_verdict"
  assert_eq "$(next --rounds 1)" "final${T}final${T}claude" "last-round dispute goes to final"
  assert_eq "$(next --single-round)" done "plan dispute ends without fix/final"
  record fix1 fix claude PUSHED
  assert_eq "$(next)" "round2${T}review${T}claude" "disputed marker is complete after fix"
  record round2 review claude APPROVE
  record round2 critic grok AGREE
  assert_eq "$(ar_round_status --ledger "$LEDGER" --round 2)" clean "earlier dispute does not taint next round"
  assert_eq "$(next)" "final${T}final${T}claude" "later clean round ends the review"
done
rm -f "$LEDGER" "${LEDGER%.tsv}.disputed.md"
record round1 review gpt APPROVE
record round1 critic claude AGREE
record round1 confirm grok DISAGREE_EVIDENCE
assert_eq "$(next --inner 1)" "disputed${T}disputed${T}-" "T veto shares the opening inner count"

echo "ar: independent plan-review round starts at round 2"
rm -f "$LEDGER"
assert_eq "$(next --start-round 2 --single-round)" "round2${T}review${T}claude" "empty ledger starts specified round"
record round2 review claude APPROVE
assert_eq "$(next --start-round 2 --single-round)" "round2${T}critic${T}grok" "round 2 uses Grok critic"
record round2 critic grok AGREE
assert_eq "$(next --start-round 2 --single-round)" done "single-round clean has no T and no final"
rm -f "$LEDGER"
record round2 review claude NEEDS_CHANGES
record round2 critic grok AGREE
assert_eq "$(next --start-round 2 --single-round)" done "single-round findings have no fix or final"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 2)" findings "planner receives findings status"

echo "ar: a revised approval is a clean round: final, no fix"
rm -f "$LEDGER"
record round1 review gpt NEEDS_CHANGES
record round1 critic claude DISAGREE_EVIDENCE
record round1 respond gpt APPROVE
record round1 critic claude AGREE
assert_eq "$(next)" "final${T}final${T}claude" "revised approval ends the review without a fix"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" clean "withdrawn findings no longer count"
assert_eq "$(next --rounds 1)" "final${T}final${T}claude" "revised approval in last round goes to final"
record fix1 fix claude NOOP
assert_eq "$(next --single-round)" done "single-round cannot advance after a completed fix"

echo "ar: a legacy ledger that moved past a clean round without a fix still replays"
rm -f "$LEDGER"
record round1 review gpt NEEDS_CHANGES
record round1 critic claude DISAGREE_EVIDENCE
record round1 respond gpt APPROVE
record round1 critic claude AGREE
record round2 review claude APPROVE
assert_eq "$(next)" "round2${T}critic${T}grok" "started later round continues"

echo "ar: configuration and ledger validation"
for args in '--rounds 0' '--rounds 6' '--inner 0' '--inner 6' '--inner nope' '--start-round 4' '--unknown'; do
  next ${=args} >/dev/null 2>&1
  assert_exit "$?" 1 "invalid arguments: $args"
done
ar_next --rounds 2 --inner 2 --ledger >/dev/null 2>&1
assert_exit "$?" 1 "missing ledger value rejected"
rm -f "$LEDGER"
record round1 review gpt UNKNOWN
next >/dev/null 2>&1; assert_exit "$?" 1 "unknown ledger verdict rejected"
ar_round_status --ledger "$LEDGER" --round 1 >/dev/null 2>&1
assert_exit "$?" 1 "status also rejects unknown verdict"
rm -f "$LEDGER"
record round1 fix claude NOOP
next >/dev/null 2>&1; assert_exit "$?" 1 "mismatched ledger role/stage rejected"
printf 'a1\tround1\treview\tgpt\tAPPROVE\n' > "$LEDGER"
next >/dev/null 2>&1; assert_exit "$?" 1 "ledger must have all six columns"

echo "ar: prompt preserves vendored paper and opaque framing"
PROMPTS="$ROOT/.claude/skills/loop-execute/references/ar-prompts.md"
LOOP="$TMP/loop with spaces"
mkdir -p "$LOOP/runs"
LEDGER="$LOOP/runs/review.ledger.tsv"
FRAMING="$TMP/framing.md"
cat > "$FRAMING" <<'EOF'
Frozen artifact: PR #12 at abc123.
Literal data: {prior_exchange} $HOME $(touch forbidden) `touch forbidden` \path
EOF
ar_prompt --role review --framing "$FRAMING" --ledger "$LEDGER" --run-dir "$LOOP/runs/new" > "$TMP/prompt.md"
assert_exit "$?" 0 "render opening review without a ledger"
head -n "$(wc -l < "$PROMPTS")" "$TMP/prompt.md" > "$TMP/vendored.md"
diff -u "$PROMPTS" "$TMP/vendored.md" > "$TMP/prompt.diff"
assert_exit "$?" 0 "all paper sentences and adaptation diff against vendored source"
assert_contains "$(cat "$TMP/prompt.md")" "$(cat "$FRAMING")" "framing is opaque literal data"
assert_contains "$(cat "$TMP/prompt.md")" "RUN_DIR=$LOOP/runs/new" "bind report and status output directory"
assert_contains "$(cat "$TMP/prompt.md")" 'Role: review (A.2.3)' "review uses paper reviewer rules"
for rule in '`[blocking: security]`' '`[blocking: unprotected-breakage]`' '`[blocking: failing-check]`' '`[deferred]`' \
  'When you are not sure, the finding is deferred.' 'only when at least one blocking finding stays' \
  'A blocking finding outside the three categories' 'A deferred finding is never a disagreement.'; do
  assert_contains "$(cat "$TMP/prompt.md")" "$rule" "finding classes rule: $rule"
done

echo "ar: exchanges follow ledger order and report paths, not attempt names"
mkdir -p "$LOOP/runs/z-review" "$LOOP/runs/a-critic" "$LOOP/runs/m-response"
printf 'F1 [major]: first report\nNEEDS_CHANGES: first\n' > "$LOOP/runs/z-review/report.md"
printf 'F2 [minor]: critic added issue\nDISAGREE_CONCERN: confirm F2\n' > "$LOOP/runs/a-critic/report.md"
printf 'F1 [major]: response retains literal {diff}\nF2 [minor]: incorporated\nNEEDS_CHANGES: second\n' > "$LOOP/runs/m-response/report.md"
cat > "$LEDGER" <<EOF
a1${T}round1${T}review${T}gpt${T}NEEDS_CHANGES${T}runs/z-review
a2${T}round1${T}critic${T}claude${T}DISAGREE_CONCERN${T}runs/a-critic
a3${T}round1${T}respond${T}gpt${T}NEEDS_CHANGES${T}runs/m-response
EOF
ar_prompt --role critic --framing "$FRAMING" --ledger "$LEDGER" --run-dir "$LOOP/runs/new" > "$TMP/exchange.md"
assert_exit "$?" 0 "read prior reports relative to loop directory"
python3 - "$TMP/exchange.md" "$LOOP" <<'PYTEST'
import sys
from pathlib import Path
prompt = Path(sys.argv[1]).read_text()
loop = Path(sys.argv[2])
reports = [(loop / 'runs' / name / 'report.md').read_text() for name in ('z-review', 'a-critic', 'm-response')]
assert all(prompt.count(report) == 1 for report in reports)
assert [prompt.index(report) for report in reports] == sorted(prompt.index(report) for report in reports)
PYTEST
assert_exit "$?" 0 "all reports preserved once, in ledger order"
for role in review respond critic confirm; do
  actual="$(ar_prompt --role "$role" --framing "$FRAMING" --ledger "$LEDGER" --run-dir "$LOOP/runs/new")"
  case "$role" in review|respond) section=A.2.3 ;; *) section=A.2.4 ;; esac
  assert_contains "$actual" "Role: $role ($section)" "$role selects its paper instructions"
done
ar_prompt --role review --framing "$FRAMING" --ledger "$LEDGER" --run-dir "$LOOP/runs/new" > "$TMP/opening.md"
assert_exit "$?" 0 "opening review accepts a non-empty ledger"
python3 - "$TMP/opening.md" <<'PYTEST'
import sys
from pathlib import Path
prompt = Path(sys.argv[1]).read_text()
assert prompt.split('## Prior exchange (ledger order)\n', 1)[1].strip() == ''
PYTEST
assert_exit "$?" 0 "opening review never inherits a prior exchange"
cp "$LEDGER" "$TMP/ledger-before.tsv"
rm "$LOOP/runs/a-critic/report.md"
ar_prompt --role respond --framing "$FRAMING" --ledger "$LEDGER" --run-dir "$LOOP/runs/new" > "$TMP/missing.md" 2>/dev/null
assert_exit "$?" 1 "missing exchange report fails instead of omitting evidence"
assert_eq "$(cat "$TMP/missing.md")" '' "missing evidence emits no partial prompt"
cmp -s "$LEDGER" "$TMP/ledger-before.tsv"
assert_exit "$?" 0 "prompt rendering never mutates ledger"
ar_prompt --role review --framing "$FRAMING" --ledger "$LEDGER" --run-dir "$LOOP/runs/new" > /dev/null
assert_exit "$?" 0 "opening review does not require prior report files"

echo "ar: disputed sidecar preserves last reports and critic-added findings"
# Replace the earlier critic and finish the second inner exchange.
printf 'F9 [minor]: obsolete review\nNEEDS_CHANGES: obsolete\n' > "$LOOP/runs/z-review/report.md"
printf 'DISAGREE_EVIDENCE: F9 refuted by source:2\n' > "$LOOP/runs/a-critic/report.md"
mkdir -p "$LOOP/runs/b-last-critic"
printf 'F1 is refuted at source:3. F7 [major]: critic-only missing validation at source:4.\nDISAGREE_EVIDENCE: F1 and F7 remain disputed' > "$LOOP/runs/b-last-critic/report.md"
printf 'a4\tround1\tcritic\tclaude\tDISAGREE_EVIDENCE\truns/b-last-critic\n' >> "$LEDGER"
assert_eq "$(ar_next --rounds 2 --inner 2 --ledger "$LEDGER")" "disputed${T}disputed${T}-" "writer runs at disagreement cap"
ar_write_disputed --ledger "$LEDGER" > "$TMP/disputed-path"
assert_exit "$?" 0 "write disputed sidecar and ledger marker"
DISPUTED="${LEDGER%.tsv}.disputed.md"
assert_eq "$(cat "$TMP/disputed-path")" "$DISPUTED" "writer returns sidecar path"
python3 - "$DISPUTED" "$LOOP" <<'PYTEST'
import sys
from pathlib import Path
text = Path(sys.argv[1]).read_text()
loop = Path(sys.argv[2])
for name in ('m-response', 'b-last-critic'):
    assert (loop / 'runs' / name / 'report.md').read_text() in text
assert 'obsolete review' not in text
ids = text.split('## Unresolved finding IDs\n', 1)[1].split('\n## ', 1)[0]
assert ids.strip().splitlines() == ['- F1', '- F2', '- F7']
PYTEST
assert_exit "$?" 0 "last R/C reports verbatim, unresolved IDs include critic-only issue"
assert_eq "$(tail -n 1 "$LEDGER")" "a5${T}disputed${T}disputed${T}-${T}DISPUTED${T}runs" "append frozen six-column disputed row"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 1)" disputed "state machine recognizes writer marker"
assert_eq "$(ar_next --rounds 2 --inner 2 --ledger "$LEDGER")" "fix1${T}fix${T}claude" "writer advances pipeline to required fix"
cp "$LEDGER" "$TMP/disputed-ledger.tsv"
cp "$DISPUTED" "$TMP/disputed-report.md"
ar_write_disputed --ledger "$LEDGER" > /dev/null
assert_exit "$?" 0 "replaying disputed writer succeeds"
cmp -s "$LEDGER" "$TMP/disputed-ledger.tsv"
assert_exit "$?" 0 "replay does not duplicate ledger row"
cmp -s "$DISPUTED" "$TMP/disputed-report.md"
assert_exit "$?" 0 "replay preserves sidecar"

echo "ar: later disputes replace the sidecar, including confirmation-only bugs"
# Skip runner-only rows in prompt exchange, and resolve paths independently of cwd.
printf 'a6\tfix1\tfix\tclaude\tNOOP\truns/no-fix-report\n' >> "$LEDGER"
mkdir -p "$LOOP/runs/round2-r" "$LOOP/runs/round2-c" "$LOOP/runs/round2-t"
printf 'No reviewer flags.\nAPPROVE\n' > "$LOOP/runs/round2-r/report.md"
printf 'AGREE\n' > "$LOOP/runs/round2-c/report.md"
printf 'F10 [blocker]: missing guard at source:10.\nDISAGREE_EVIDENCE: F10 missed' > "$LOOP/runs/round2-t/report.md"
printf 'a7\tround2\treview\tclaude\tAPPROVE\truns/round2-r\na8\tround2\tcritic\tgrok\tAGREE\truns/round2-c\na9\tround2\tconfirm\tgpt\tDISAGREE_EVIDENCE\truns/round2-t' >> "$LEDGER"
for role in critic respond confirm; do
  (cd "$TMP" && ar_prompt --role "$role" --framing "$FRAMING" --ledger "$LEDGER" --run-dir "$LOOP/runs/new") > "$TMP/later.md"
  assert_exit "$?" 0 "$role uses ledger run_dir from another cwd and skips synthetic/fix rows"
  python3 - "$TMP/later.md" "$LOOP" <<'PYTEST'
import sys
from pathlib import Path
prompt = Path(sys.argv[1]).read_text().split('## Prior exchange (ledger order)\n', 1)[1]
loop = Path(sys.argv[2])
reports = [(loop / 'runs' / name / 'report.md').read_text() for name in ('round2-r', 'round2-c', 'round2-t')]
assert all(prompt.count(report) == 1 for report in reports)
assert [prompt.index(report) for report in reports] == sorted(prompt.index(report) for report in reports)
for name in ('z-review', 'a-critic', 'm-response', 'b-last-critic'):
    assert (loop / 'runs' / name / 'report.md').read_text() not in prompt
PYTEST
  assert_exit "$?" 0 "$role includes only current-round reports in ledger order"
done
mv "$LOOP/runs/a-critic/report.md" "$TMP/archived-critic.md"
ar_prompt --role respond --framing "$FRAMING" --ledger "$LEDGER" --run-dir "$LOOP/runs/new" > /dev/null
assert_exit "$?" 0 "current-round exchange does not require archived report files"
mv "$TMP/archived-critic.md" "$LOOP/runs/a-critic/report.md"
ar_write_disputed --ledger "$LEDGER" > /dev/null
assert_exit "$?" 0 "third-provider veto can produce a dispute after APPROVE"
python3 - "$DISPUTED" "$LOOP" <<'PYTEST'
import sys
from pathlib import Path
text = Path(sys.argv[1]).read_text()
loop = Path(sys.argv[2])
assert text.startswith('# Disputed AR round2\n')
assert (loop / 'runs/round2-r/report.md').read_text() in text
assert (loop / 'runs/round2-t/report.md').read_text() in text
assert '## Unresolved finding IDs\n- F10\n' in text
assert 'F1 [major]' not in text and 'F7' not in text
PYTEST
assert_exit "$?" 0 "latest round uses confirming critic and critic-only finding"
assert_eq "$(tail -n 1 "$LEDGER")" "a10${T}disputed${T}disputed${T}-${T}DISPUTED${T}runs" "append after unterminated ledger line"
assert_eq "$(ar_round_status --ledger "$LEDGER" --round 2)" disputed "APPROVE plus critic-only issue remains disputed"

# Simulate a crash after sidecar publication but before the atomic ledger rename.
head -n 9 "$LEDGER" > "$TMP/pre-marker.tsv"
cp "$TMP/pre-marker.tsv" "$LEDGER"
ar_write_disputed --ledger "$LEDGER" > /dev/null
assert_exit "$?" 0 "replay safely publishes a previously unrecorded sidecar"
assert_eq "$(wc -l < "$LEDGER" | tr -d ' ')" 10 "recovery appends exactly one marker"

# A missing current report must preserve the prior evidence and ledger.
head -n 9 "$LEDGER" > "$TMP/pre-marker.tsv"
cp "$TMP/pre-marker.tsv" "$LEDGER"
cp "$DISPUTED" "$TMP/prior-disputed.md"
rm "$LOOP/runs/round2-t/report.md"
ar_write_disputed --ledger "$LEDGER" > /dev/null 2>&1
assert_exit "$?" 1 "missing last critic report fails"
cmp -s "$LEDGER" "$TMP/pre-marker.tsv"
assert_exit "$?" 0 "missing evidence cannot append a marker"
cmp -s "$DISPUTED" "$TMP/prior-disputed.md"
assert_exit "$?" 0 "missing evidence cannot replace prior dispute"

echo "ar: invalid prompt and dispute inputs do not produce outputs"
for args in '--role unknown' '--framing /no/such/file' '--unexpected value'; do
  ar_prompt --role review --framing "$FRAMING" --ledger "$TMP/empty.tsv" --run-dir "$LOOP/runs/new" ${=args} > "$TMP/invalid.md" 2>/dev/null
  assert_exit "$?" 1 "invalid prompt arguments: $args"
  assert_eq "$(cat "$TMP/invalid.md")" '' "invalid arguments emit no prompt"
done
ar_prompt --role review --framing "$FRAMING" --ledger > /dev/null 2>&1
assert_exit "$?" 1 "missing prompt flag value rejected"
printf 'a1\tround1\treview\tgpt\tAPPROVE\truns/z-review\n' > "$TMP/incomplete.tsv"
ar_write_disputed --ledger "$TMP/incomplete.tsv" > /dev/null 2>&1
assert_exit "$?" 1 "review without disagreement cannot be disputed"
printf 'a2\tround1\tcritic\tclaude\tAGREE\truns/a-critic\n' >> "$TMP/incomplete.tsv"
ar_write_disputed --ledger "$TMP/incomplete.tsv" > /dev/null 2>&1
assert_exit "$?" 1 "agreement cannot be disputed"

echo "ar: commands run as a script and stay inert when sourced"
AR="$ROOT/loop-ar.sh"
zsh -c 'source "$1"; print sourced-ok' _ "$AR" > "$TMP/sourced.out" 2>&1
assert_eq "$(cat "$TMP/sourced.out")" sourced-ok "sourcing runs no command and prints no usage"
zsh "$AR" > /dev/null 2>&1; assert_exit "$?" 1 "no command is a usage error"
zsh "$AR" bogus > /dev/null 2>&1; assert_exit "$?" 1 "unknown command is a usage error"

echo "ar: verdict command"
printf 'F1: x\nNEEDS_CHANGES: broken\n' > "$REPORT"
assert_eq "$(zsh "$AR" verdict "$REPORT" reviewer)" NEEDS_CHANGES "verdict prints the reviewer token"
printf 'AGREE\n' > "$REPORT"
assert_eq "$(zsh "$AR" verdict "$REPORT" critic)" AGREE "verdict prints the critic token"
actual="$(zsh "$AR" verdict "$REPORT" reviewer)"; rc=$?
assert_eq "$actual" invalid "verdict prints invalid for a wrong kind"
assert_exit "$rc" 2 "invalid verdict exits 2"
zsh "$AR" verdict "$REPORT" > /dev/null 2>&1; assert_exit "$?" 1 "verdict needs a report and a kind"

echo "ar: prompt and disputed commands"
CLI_LOOP="$TMP/cli-loop"
CLI_LEDGER="$CLI_LOOP/runs/cli.ledger.tsv"
mkdir -p "$CLI_LOOP/runs/a1" "$CLI_LOOP/runs/a2"
print -r -- 'F1 [major]: broken at source:1' > "$CLI_LOOP/runs/a1/report.md"
print -r -- 'NEEDS_CHANGES: F1' >> "$CLI_LOOP/runs/a1/report.md"
printf 'review-p1-acme--api\n' > "$TMP/cli-framing.md"
printf 'a1\tround1\treview\tgpt\tNEEDS_CHANGES\truns/a1\n' > "$CLI_LEDGER"
zsh "$AR" prompt --role critic --framing "$TMP/cli-framing.md" --ledger "$CLI_LEDGER" --run-dir "$CLI_LOOP/runs/a2" > "$TMP/cli-prompt.md"
assert_exit "$?" 0 "prompt command renders a critic prompt"
assert_eq "$(ar_prompt --role critic --framing "$TMP/cli-framing.md" --ledger "$CLI_LEDGER" --run-dir "$CLI_LOOP/runs/a2")" "$(cat "$TMP/cli-prompt.md")" "prompt command equals ar_prompt"
zsh "$AR" prompt --role unknown --framing "$TMP/cli-framing.md" --ledger "$CLI_LEDGER" --run-dir "$CLI_LOOP/runs/a2" > /dev/null 2>&1
assert_exit "$?" 1 "prompt command rejects an unknown role"
zsh "$AR" disputed --ledger "$CLI_LEDGER" > /dev/null 2>&1
assert_exit "$?" 1 "disputed command needs a pending disagreement"
print -r -- 'DISAGREE_EVIDENCE: F1 refuted at source:2' > "$CLI_LOOP/runs/a2/report.md"
printf 'a2\tround1\tcritic\tclaude\tDISAGREE_EVIDENCE\truns/a2\na3\tround1\trespond\tgpt\tNEEDS_CHANGES\truns/a1\na4\tround1\tcritic\tclaude\tDISAGREE_EVIDENCE\truns/a2\n' >> "$CLI_LEDGER"
actual="$(zsh "$AR" disputed --ledger "$CLI_LEDGER")"; rc=$?
assert_exit "$rc" 0 "disputed command writes the sidecar"
assert_eq "$actual" "${CLI_LEDGER%.tsv}.disputed.md" "disputed command prints the sidecar path"
assert_eq "$(tail -n 1 "$CLI_LEDGER" | cut -f2,3,5)" "disputed${T}disputed${T}DISPUTED" "disputed command appends the marker row"

echo "ar: prove-fix command checks the remote head"
PF_REPO="$TMP/pf-repo"; PF_REMOTE="$TMP/pf-remote.git"; PF_RUN="$TMP/pf-run"
git init -q "$PF_REPO"
git -C "$PF_REPO" -c user.name=Test -c user.email=test@example.test commit -qm initial --allow-empty
PF_BEFORE="$(git -C "$PF_REPO" rev-parse HEAD)"
git clone -q --bare "$PF_REPO" "$PF_REMOTE"
git --git-dir="$PF_REMOTE" update-ref refs/heads/review "$PF_BEFORE"
mkdir -p "$PF_RUN"
pf_status() {
  jq -n --arg result "$1" --arg before "$2" --arg after "$3" --arg remote "$PF_REMOTE" --arg wt "$PF_REPO" \
    '{outcome:"done",result:$result,headBefore:$before,headAfter:$after,remote:$remote,branch:"review",worktree:$wt}' > "$PF_RUN/status.json"
}
pf_status NOOP "$PF_BEFORE" "$PF_BEFORE"
assert_eq "$(zsh "$AR" prove-fix "$PF_RUN")" NOOP "NOOP fix proven by the remote head"
pf_status NOOP "$PF_BEFORE" "${PF_BEFORE/#?/f}"
zsh "$AR" prove-fix "$PF_RUN" > /dev/null 2>&1; assert_exit "$?" 1 "NOOP with changed head rejected"
git -C "$PF_REPO" -c user.name=Test -c user.email=test@example.test commit -qm fix --allow-empty
PF_AFTER="$(git -C "$PF_REPO" rev-parse HEAD)"
pf_status PUSHED "$PF_BEFORE" "$PF_AFTER"
actual="$(zsh "$AR" prove-fix "$PF_RUN" 2>&1)"; rc=$?
assert_exit "$rc" 1 "unpublished head cannot prove PUSHED"
assert_eq "$actual" "loop-ar: remote head does not match completed fix" "mismatch names the cause"
git --git-dir="$PF_REMOTE" fetch -q "$PF_REPO" HEAD:refs/heads/review
assert_eq "$(zsh "$AR" prove-fix "$PF_RUN")" PUSHED "published head proves PUSHED"
assert_eq "$(ar_prove_fix "$PF_RUN")" PUSHED "ar_prove_fix works when sourced"
printf '{"outcome":"blocked"}\n' > "$PF_RUN/status.json"
zsh "$AR" prove-fix "$PF_RUN" > /dev/null 2>&1; assert_exit "$?" 1 "a blocked fix is not proven"
zsh "$AR" prove-fix > /dev/null 2>&1; assert_exit "$?" 1 "prove-fix needs a run dir"

test_summary
