#!/bin/zsh
# loop-execute public review contract: three local adversarial passes, two fixes, one post.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

SKILL="$(cat "$ROOT/.claude/skills/loop-execute/SKILL.md")"
FIX_SKILL="$(cat "$ROOT/.claude/skills/pr-review-fix-all/SKILL.md")"
FIX_AGENT="$(cat "$ROOT/.claude/agents/pr-review-fixer.md")"
PROTOCOL="$(cat "$ROOT/.claude/skills/loop-execute/references/loop-protocol.md")"

echo "review workflow: staged local reviews and Opus remediation"
assert_contains "$SKILL" '`review-astra`, `a1`' "round 1 starts with Astra"
assert_contains "$SKILL" '`review-fable`, `a2`' "round 1 pairs Fable"
assert_contains "$SKILL" '`review-fix`, `a3`' "round 1 ends with Opus remediation"
assert_contains "$SKILL" '`review-fix`, `a6`' "round 2 ends with Opus remediation"
assert_contains "$SKILL" '`review-final`, `a9`' "round 3 ends with Opus final review"
assert_contains "$PROTOCOL" 'Round 2 is local-only just like rounds 1 and 3.' "all adversarial rounds stay off GitHub"
assert_contains "$SKILL" '`/pr-review-fix-all`' "loop workflow invokes the remediation skill"
assert_contains "$PROTOCOL" '`/pr-review-fix-all`' "protocol invokes the remediation skill"

echo "review workflow: remediation skill owns Opus delegation"
assert_contains "$FIX_SKILL" 'Write `RUN_DIR/remediation-plan.md`' "records reconciliation before fixes"
assert_contains "$FIX_SKILL" 'accepted, rejected, or duplicate' "disposes every finding"
assert_contains "$FIX_SKILL" 'maximum of 3 concurrent subagents' "bounds parallel remediation"
assert_contains "$FIX_SKILL" 'Opus 5' "pins remediation subagents"
assert_contains "$FIX_SKILL" 'Agent tool with the `pr-review-fixer` subagent' "spawns through the skill"
assert_contains "$FIX_AGENT" 'model: claude-opus-5' "pins fixer agents to Opus 5"
assert_contains "$FIX_AGENT" 'must not commit, push, or post to GitHub' "restricts fixer mutations"
assert_contains "$FIX_SKILL" 'overlapping files or dependencies serially' "serializes conflicting groups"
assert_contains "$FIX_SKILL" 'must not commit, push, or post to GitHub' "keeps mutations with coordinator"
assert_contains "$FIX_SKILL" 'Run the repository verification command' "coordinator verifies integrated fixes"
assert_contains "$FIX_SKILL" 'Commit logical fix groups and fast-forward push' "coordinator owns commits and push"

echo "review workflow: one severity-gated GitHub review"
assert_contains "$PROTOCOL" 'one or more `[blocker]` findings → `REQUEST_CHANGES`' "blockers request changes"
assert_contains "$PROTOCOL" 'one or more `[major]` findings → `COMMENT`' "majors produce comment review"
assert_contains "$PROTOCOL" 'otherwise → `APPROVE`' "clean major/blocker result approves"
assert_contains "$PROTOCOL" '<!-- loop-review:<runId>:<owner--repo>:final -->' "final post is idempotent"

test_summary
