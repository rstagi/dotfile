#!/bin/zsh
# loop-execute public review contract: three local adversarial passes, two fixes, one post.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

SKILL="$(cat "$ROOT/.claude/skills/loop-execute/SKILL.md")"
PROTOCOL="$(cat "$ROOT/.claude/skills/loop-execute/references/loop-protocol.md")"

echo "review workflow: staged local reviews and Opus remediation"
assert_contains "$SKILL" '`review-astra`, `a1`' "round 1 starts with Astra"
assert_contains "$SKILL" '`review-fable`, `a2`' "round 1 pairs Fable"
assert_contains "$SKILL" '`review-fix`, `a3`' "round 1 ends with Opus remediation"
assert_contains "$SKILL" '`review-fix`, `a6`' "round 2 ends with Opus remediation"
assert_contains "$SKILL" '`review-final`, `a9`' "round 3 ends with Opus final review"
assert_contains "$PROTOCOL" 'Round 2 is local-only just like rounds 1 and 3.' "all adversarial rounds stay off GitHub"
assert_contains "$PROTOCOL" 'registered loop-opus-fixer' "remediation delegates to Opus fixers"

echo "review workflow: one severity-gated GitHub review"
assert_contains "$PROTOCOL" 'one or more `[blocker]` findings → `REQUEST_CHANGES`' "blockers request changes"
assert_contains "$PROTOCOL" 'one or more `[major]` findings → `COMMENT`' "majors produce comment review"
assert_contains "$PROTOCOL" 'otherwise → `APPROVE`' "clean major/blocker result approves"
assert_contains "$PROTOCOL" '<!-- loop-review:<runId>:<owner--repo>:final -->' "final post is idempotent"

test_summary
