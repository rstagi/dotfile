#!/bin/zsh
# converge + assess skill contracts and their wiring into loop-execute and multiphase-plan.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

read_skill() { cat "$ROOT/.claude/skills/$1" 2>/dev/null || true; }

CONVERGE="$(read_skill converge/SKILL.md)"
EXECUTE="$(read_skill loop-execute/SKILL.md)"
PROTOCOL="$(read_skill loop-execute/references/loop-protocol.md)"

echo "converge: Done-when check against the built code"
assert_contains "$CONVERGE" 'name: converge' "skill exists"
assert_contains "$CONVERGE" '`met`, `partial` or `missing`' "three criterion states"
assert_contains "$CONVERGE" 'Do not edit code, the plan or the state.' "converge is read-only"
assert_contains "$CONVERGE" '.loop/converge/p<N>.md' "writes one report per review phase"
assert_contains "$CONVERGE" 'Verdict: converged|gaps' "last line is a parsable verdict"
assert_contains "$CONVERGE" 'Every `met` needs evidence' "met is proven, not assumed"

echo "converge: loop-execute wiring"
assert_contains "$EXECUTE" '**Converge (once per review phase).**' "§8 runs converge before reviewers"
assert_contains "$EXECUTE" 'state.converge[<reviewPhase>]' "converge pass is persisted for resume"
assert_contains "$EXECUTE" 'one gap phase per `missing` criterion' "gaps become inserted phases"
assert_contains "$EXECUTE" 'never a second converge pass' "converge is bounded to one pass"
assert_contains "$PROTOCOL" 'Converge runner prompt skeleton:' "protocol has the runner skeleton"
assert_contains "$PROTOCOL" 'runs/converge-p<N>-a<k>/' "protocol lists the run dir"

ASSESS="$(read_skill assess/SKILL.md)"
PLAN_SKILL="$(read_skill multiphase-plan/SKILL.md)"

echo "assess: go / clarify / kill before a plan"
assert_contains "$ASSESS" 'name: assess' "skill exists"
assert_contains "$ASSESS" '`go`, `clarify` or `kill`' "three decisions"
assert_contains "$ASSESS" '.loop/assessments/<slug>.md' "writes one decision file per idea"
assert_contains "$ASSESS" '## Handoff' "go decision carries a handoff for multiphase-plan"
assert_contains "$ASSESS" 'Every claim cites its source' "evidence-backed claims"
assert_contains "$ASSESS" 'Do not write code or a plan.' "assess stops before planning"

echo "assess: multiphase-plan wiring"
assert_contains "$PLAN_SKILL" 'Offer `assess` when the goal is an idea' "plan offers assess for raw ideas"
assert_contains "$PLAN_SKILL" 'read its `## Handoff` as the goal' "plan consumes the handoff"
assert_contains "$PLAN_SKILL" 'A `kill` or `clarify` decision stops the plan' "plan respects a non-go decision"

test_summary
