#!/bin/zsh
# Check the plan tools in a directory that has no loop-web files.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
export HOME="$TMP/home" WS_HOME="$TMP/ws" WS_STATE="$TMP/state" WS_SOCK="$TMP/ws.sock"
mkdir -p "$HOME" "$TMP/bin" "$TMP/tools" "$TMP/coord"
export PATH="$TMP/bin:$PATH" LOOP_EMIT_SH="$HERE/fake/loop-emit.sh"
cat > "$TMP/bin/gh" <<'SH'
#!/bin/sh
printf '{"state":"MERGED"}\n'
SH
chmod +x "$TMP/bin/gh"

cp "$ROOT/loop-plan.sh" "$ROOT/loop-repo.sh" "$ROOT/loop-plan-archive.mjs" "$ROOT/loop-plan-parse.mjs" "$TMP/tools/"
cp -R "$ROOT/loop-plan-model" "$TMP/tools/"
FIX="$HERE/fixtures/milestones"
cp "$FIX/plan.md" "$TMP/coord/plan.md"

echo "loop-plan-tools: archive and parse without loop-web"
out="$(zsh "$TMP/tools/loop-plan.sh" archive --milestone 1 --dir "$TMP/coord" 2>&1)"; rc=$?
assert_exit "$rc" 0 "archive succeeds without loop-web ($out)"
assert_eq "$(cat "$TMP/coord/plan.md")" "$(cat "$FIX/archived-m1/plan.md")" "the plan matches the archived fixture"
assert_eq "$(cat "$TMP/coord/milestones/m1.md" 2>/dev/null)" "$(cat "$FIX/archived-m1/milestones/m1.md")" "the archive matches the fixture"

node "$TMP/tools/loop-plan-parse.mjs" --plan "$TMP/coord/plan.md" > "$TMP/parsed.json"
assert_exit "$?" 0 "the parser reads the archived plan file"
assert_eq "$(jq -c '[.phases[] | select(.archived) | [.phase, .status]]' "$TMP/parsed.json")" \
  '[["1","done"],["2","done"],["9","done"],["3","done"]]' "the stub phases remain done"
assert_eq "$(jq -r '.milestones[0].archive.file' "$TMP/parsed.json")" "milestones/m1.md" "the stub keeps the archive path"
assert_eq "$(jq -c '.warnings' "$TMP/parsed.json")" '[]' "the archived plan has no warnings"

node "$TMP/tools/loop-plan-parse.mjs" --plan - < "$TMP/coord/plan.md" > "$TMP/stdin.json"
assert_exit "$?" 0 "the parser reads standard input"
assert_eq "$(cat "$TMP/stdin.json")" "$(cat "$TMP/parsed.json")" "file and standard input give the same JSON"

echo "loop-plan-tools: parser errors"
for args in '' '--unknown file' '--plan' '--plan file extra'; do
  node "$TMP/tools/loop-plan-parse.mjs" ${=args} > "$TMP/out" 2> "$TMP/error"
  assert_exit "$?" 1 "the parser rejects invalid arguments: $args"
  assert_eq "$(cat "$TMP/out")" '' "the parser writes no JSON for invalid arguments"
  assert_contains "$(cat "$TMP/error")" 'use --plan <file|->' "the error states the required arguments"
done
node "$TMP/tools/loop-plan-parse.mjs" --plan "$TMP/missing.md" > "$TMP/out" 2> "$TMP/error"
assert_exit "$?" 1 "the parser rejects a missing file"
assert_eq "$(cat "$TMP/out")" '' "the parser writes no JSON for a missing file"
assert_contains "$(cat "$TMP/error")" 'loop-plan-parse:' "the file error identifies the tool"

test_summary
