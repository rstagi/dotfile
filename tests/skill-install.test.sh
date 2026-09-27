#!/bin/zsh
# Shared Loop skill installer: Claude Code, current/legacy Codex, and Ratel Local.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
SOURCE="$TMP/source"
TEST_HOME="$TMP/home"
FAKE_BIN="$TMP/bin"
SKILLS=(checkpoint-30 multiphase-plan loop-execute loop-pickup loop-handoff pr-review pr-review-fix-all)
mkdir -p "$SOURCE" "$TEST_HOME/.agents/skills/unrelated" "$FAKE_BIN"
SOURCE="$(cd "$SOURCE" && pwd -P)"
print -r -- keep > "$TEST_HOME/.agents/skills/unrelated/marker"

for skill in "${SKILLS[@]}"; do
  mkdir -p "$SOURCE/$skill"
  print -r -- "---\nname: $skill\ndescription: test\n---" > "$SOURCE/$skill/SKILL.md"
done
ln -s "$HERE/fixtures/fake-ratel-local.sh" "$FAKE_BIN/ratel-local"

export FAKE_RATEL_LOG="$TMP/ratel.log"
export FAKE_SKILL_SOURCE="$SOURCE"
export FAKE_RATEL_CONFIGURED_JSON='[]'

HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" bash "$ROOT/install-agent-skills.sh" \
  --source "$SOURCE" > "$TMP/install.out" 2>&1
RC=$?

echo "skill install: shares Loop skills across hosts"
assert_exit "$RC" "0" "installer succeeds"
for skill in "${SKILLS[@]}"; do
  assert_eq "$(readlink "$TEST_HOME/.claude/skills/$skill")" "$SOURCE/$skill" "Claude exposes $skill"
  assert_eq "$(readlink "$TEST_HOME/.agents/skills/$skill")" "$SOURCE/$skill" "Codex exposes $skill"
  assert_eq "$(readlink "$TEST_HOME/.codex/skills/$skill")" "$SOURCE/$skill" "legacy Codex exposes $skill"
done
assert_eq "$(cat "$TEST_HOME/.agents/skills/unrelated/marker")" "keep" "preserves unrelated Codex skills"
assert_eq "$(grep -c '^skill import ' "$FAKE_RATEL_LOG")" "7" "registers every shared skill with Ratel"
assert_contains "$(cat "$FAKE_RATEL_LOG")" \
  'skill import cand-pr-review-fix-all --scope user --mode reference --yes' \
  "registers pr-review-fix-all by stable candidate id"

echo "skill install: rerun is idempotent"
export FAKE_RATEL_CONFIGURED_JSON="$(printf '%s\n' "${SKILLS[@]}" | jq -Rsc 'split("\n")[:-1] | map({id: .})')"
HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" bash "$ROOT/install-agent-skills.sh" \
  --source "$SOURCE" > "$TMP/reinstall.out" 2>&1
RC=$?
assert_exit "$RC" "0" "rerun succeeds"
assert_eq "$(grep -c '^skill import ' "$FAKE_RATEL_LOG")" "7" "does not re-import configured Ratel skills"
assert_eq "$(grep -c '^skill list --discovered' "$FAKE_RATEL_LOG")" "1" "does not rediscover fully configured skills"

echo "skill install: recovers from native links left by an interrupted registration"
RETRY_HOME="$TMP/retry-home"
mkdir -p "$RETRY_HOME/.claude/skills" "$RETRY_HOME/.agents/skills" "$RETRY_HOME/.codex/skills"
for skill in "${SKILLS[@]}"; do
  ln -s "$SOURCE/$skill" "$RETRY_HOME/.claude/skills/$skill"
  ln -s "$SOURCE/$skill" "$RETRY_HOME/.agents/skills/$skill"
  ln -s "$SOURCE/$skill" "$RETRY_HOME/.codex/skills/$skill"
done
: > "$FAKE_RATEL_LOG"
export FAKE_RATEL_CONFIGURED_JSON='[]'
HOME="$RETRY_HOME" PATH="$FAKE_BIN:$PATH" bash "$ROOT/install-agent-skills.sh" \
  --source "$SOURCE" > "$TMP/retry.out" 2>&1
RC=$?
assert_exit "$RC" "0" "retry succeeds despite duplicate native discovery links"
for skill in "${SKILLS[@]}"; do
  assert_eq "$(readlink "$RETRY_HOME/.agents/skills/$skill")" "$SOURCE/$skill" "retry restores Codex $skill"
  assert_eq "$(readlink "$RETRY_HOME/.codex/skills/$skill")" "$SOURCE/$skill" "retry restores legacy Codex $skill"
done

echo "skill install: refuses conflicting native paths"
CONFLICT_HOME="$TMP/conflict-home"
mkdir -p "$CONFLICT_HOME/.agents/skills/pr-review-fix-all"
print -r -- keep > "$CONFLICT_HOME/.agents/skills/pr-review-fix-all/marker"
HOME="$CONFLICT_HOME" PATH="$FAKE_BIN:$PATH" bash "$ROOT/install-agent-skills.sh" \
  --source "$SOURCE" --no-ratel > "$TMP/conflict.out" 2>&1
RC=$?
assert_exit "$RC" "1" "conflicting skill fails safely"
assert_eq "$(cat "$CONFLICT_HOME/.agents/skills/pr-review-fix-all/marker")" "keep" "does not overwrite conflict"

test_summary
