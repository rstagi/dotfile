#!/bin/zsh
# loop-execute public review contract: tiered local adversarial rounds, fixes between rounds, one post.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

SKILL="$(cat "$ROOT/.claude/skills/loop-execute/SKILL.md")"
FIX_SKILL="$(cat "$ROOT/.claude/skills/pr-review-fix-all/SKILL.md")"
FIX_AGENT="$(cat "$ROOT/.claude/agents/pr-review-fixer.md")"
PROTOCOL="$(cat "$ROOT/.claude/skills/loop-execute/references/loop-protocol.md")"

echo "review workflow: tiered staged local reviews and remediation"
assert_contains "$SKILL" '`[review: shallow|medium|max]`' "reads the review tier from the phase tags"
assert_contains "$SKILL" '`loop-review.sh stages --tier <T> --rounds <N>`' "generates the run list"
assert_contains "$SKILL" '`--chain <chain> --review-tier <T>' "passes chain + tier to every run"
assert_contains "$SKILL" '**never posts or mutates GitHub**' "adversaries stay local"
assert_contains "$SKILL" 'shallow: none — it performs the' "shallow final reviews the code itself"
assert_contains "$PROTOCOL" '| `medium` (default) | Sol `xhigh` + Opus `xhigh` | Opus `medium` + Opus fixers | Opus `xhigh` |' "medium tier models + effort"
assert_contains "$PROTOCOL" '| `max` | Astra `ultra` + Fable `max` |' "max tier adversaries"
assert_contains "$PROTOCOL" '| `shallow` | Opus `high` (one reviewer) | Sonnet `medium`, no subagents |' "shallow tier"
assert_contains "$PROTOCOL" 'Every round is local-only.' "all adversarial rounds stay off GitHub"
assert_contains "$SKILL" '`/pr-review-fix-all`' "loop workflow invokes the remediation skill"
assert_contains "$PROTOCOL" '`/pr-review-fix-all`' "protocol invokes the remediation skill"

echo "review workflow: remediation skill owns Opus delegation"
assert_contains "$FIX_SKILL" 'Write `RUN_DIR/remediation-plan.md`' "records reconciliation before fixes"
assert_contains "$FIX_SKILL" 'accepted, rejected, or duplicate' "disposes every finding"
assert_contains "$FIX_SKILL" 'maximum of 3 concurrent subagents' "bounds parallel remediation"
assert_contains "$FIX_SKILL" '**shallow:** apply the fix groups yourself' "shallow coordinator fixes without subagents"
assert_contains "$FIX_SKILL" '`model: opus` (the latest Opus)' "remediation subagents run the latest Opus"
assert_contains "$FIX_SKILL" 'Agent tool with the `pr-review-fixer` subagent' "spawns through the skill"
assert_contains "$FIX_AGENT" 'model: opus' "fixer agents run the latest Opus"
assert_contains "$FIX_AGENT" 'must not commit, push, or post to GitHub' "restricts fixer mutations"
assert_contains "$FIX_SKILL" 'overlapping files or dependencies serially' "serializes conflicting groups"
assert_contains "$FIX_SKILL" 'must not commit, push, or post to GitHub' "keeps mutations with coordinator"
assert_contains "$FIX_SKILL" 'Run the repository verification command' "coordinator verifies integrated fixes"
assert_contains "$FIX_SKILL" 'Commit logical fix groups and fast-forward push' "coordinator owns commits and push"

echo "review workflow: one severity-gated GitHub review"
assert_contains "$PROTOCOL" 'one or more `[blocker]` findings → `REQUEST_CHANGES`' "blockers request changes"
assert_contains "$PROTOCOL" 'one or more `[major]` findings → `COMMENT`' "majors produce comment review"
assert_contains "$PROTOCOL" 'otherwise → `APPROVE`' "clean major/blocker result approves"
assert_contains "$PROTOCOL" '<!-- loop-review:<runId>:p<N>:<owner--repo>:final -->' "final post is phase-idempotent"
assert_contains "$PROTOCOL" 'review-p<N>-<owner--repo>-a<k>' "review artifacts are phase-scoped"

test_summary
