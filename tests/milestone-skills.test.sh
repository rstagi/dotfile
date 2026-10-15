#!/bin/zsh
# Planning + lane skills + docs know about milestones: literal asserts on the prose contract.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

FORMAT="$(cat "$ROOT/.claude/skills/multiphase-plan/references/plan-format.md")"
PLAN_SKILL="$(cat "$ROOT/.claude/skills/multiphase-plan/SKILL.md")"
PICKUP="$(cat "$ROOT/.claude/skills/loop-pickup/SKILL.md")"
HANDOFF="$(cat "$ROOT/.claude/skills/loop-handoff/SKILL.md")"
CLAUDE_MD="$(cat "$ROOT/CLAUDE.md")"
README="$(cat "$ROOT/loop-web/README.md")"

echo "milestone skills: plan-format documents the Approach grammar"
assert_contains "$FORMAT" '## Milestone N — <outcome>' "milestone section heading"
assert_contains "$FORMAT" 'a plain `## Phases` plan is one implicit milestone 1' "plain Phases = implicit milestone 1"
assert_contains "$FORMAT" '- **Integration branch (owner/repo):**' "per-repository branch line"
assert_contains "$FORMAT" '- **PR (owner/repo):**' "per-repository PR line"
assert_contains "$FORMAT" 'milestone repository line > milestone line > (milestone 1 only) `## Repositories` override > Loop config' "branch precedence"
assert_contains "$FORMAT" 'Milestones ≥2 must declare their branch' "M>=2 branch required"
assert_contains "$FORMAT" 'may not repeat for the same repository across milestones' "branch uniqueness"
assert_contains "$FORMAT" 'applies only to milestone-less plans' "Repositories PR line scope"
assert_contains "$FORMAT" '`[gate: merge]`' "gate tag"
assert_contains "$FORMAT" 'must list the gate in `Depends on` explicitly' "explicit gate deps"
assert_contains "$FORMAT" "A milestone's participants are the repositories of its work phases" "participants"
assert_contains "$FORMAT" '`awaiting merge`' "Status awaiting merge"
assert_contains "$FORMAT" '`…/<owner--repo>/integration-<branch-slug>`' "milestone >=2 worktree path"
assert_contains "$FORMAT" '`…/<owner--repo>/integration`' "milestone 1 worktree path"
assert_contains "$FORMAT" '## Milestone 1 — ' "two-milestone template: M1"
assert_contains "$FORMAT" '## Milestone 2 — ' "two-milestone template: M2"

echo "milestone skills: multiphase-plan proposes milestones sparingly"
assert_contains "$PLAN_SKILL" 'Propose milestones only when the effort is too big for one PR' "milestones only when too big"
assert_contains "$PLAN_SKILL" 'each milestone leaves main shippable' "each milestone shippable"
assert_contains "$PLAN_SKILL" '  - milestones: one PR, or several' "step-1 question gate asks about milestones"

echo "milestone skills: loop-pickup respects the current milestone"
assert_contains "$PICKUP" 'never offer or claim a phase of a later milestone than `state.milestone`' "pickup never claims later milestone"
assert_contains "$PICKUP" 'REFUSED: phase belongs to a later milestone' "pickup auto refusal"

echo "milestone skills: loop-handoff never completes a later milestone"
assert_contains "$HANDOFF" 'never mark a phase of a later milestone than `state.milestone` `done`' "handoff never marks later milestone done"

echo "milestone skills: CLAUDE.md + README"
assert_contains "$CLAUDE_MD" '**Milestones:**' "CLAUDE.md milestones paragraph"
assert_contains "$CLAUDE_MD" '`awaiting-merge`' "CLAUDE.md awaiting-merge"
assert_contains "$CLAUDE_MD" '/loop-execute resume' "CLAUDE.md resume command"
assert_contains "$CLAUDE_MD" 'restart the Loop daemon after upgrading' "CLAUDE.md daemon restart"
assert_contains "$README" '### Milestones' "README milestones section"
assert_contains "$README" '`awaiting-merge`' "README awaiting-merge"
assert_contains "$README" 'restart the Loop daemon after upgrading' "README daemon restart"

test_summary
