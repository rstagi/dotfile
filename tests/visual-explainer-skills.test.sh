#!/bin/zsh
# Skills that call visual-explainer: literal asserts on the prose contract, plus the STE lint gate.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

# assert_lacks <haystack> <needle> <msg>
assert_lacks() { assert_eq "$([[ "$1" == *"$2"* ]] && echo present || echo absent)" absent "$3"; }

EXPLAINER="$ROOT/.claude/skills/visual-explainer/SKILL.md"
PLAN_SKILL="$(cat "$ROOT/.claude/skills/multiphase-plan/SKILL.md")"
PR_REVIEW="$(cat "$ROOT/.claude/skills/pr-review/SKILL.md")"

echo "visual explainer: the skill text passes the STE linter"
python3 "$ROOT/.claude/skills/asd-ste100/scripts/ste-lint.py" "$EXPLAINER" > /dev/null
assert_exit "$?" "0" "visual-explainer SKILL.md has no hard STE violations"
assert_contains "$(cat "$EXPLAINER")" 'Do not write HTML, CSS or JS.' "agents write props only"
assert_contains "$(cat "$EXPLAINER")" '### 3. Check every hand-test command against the code' "ship commands are checked against code"

echo "visual explainer: multiphase-plan explains every plan except light ones"
assert_contains "$PLAN_SKILL" '### 5. Explain the plan (skipped in `light` mode)' "plan explainer step after registration"
assert_contains "$PLAN_SKILL" 'A `light` plan also skips the visual explainer (step 5).' "light skips the explainer"
assert_contains "$PLAN_SKILL" '### 6. Report' "report follows the explainer"

echo "visual explainer: pr-review Step 4 renders a ship page, with no video"
assert_contains "$PR_REVIEW" '## Step 4: Visual report (`visual-explainer` ship page)' "Step 4 uses visual-explainer"
assert_contains "$PR_REVIEW" 'pr-review-output/<repo>-<pr>/' "Step 4 keeps the output location"
assert_lacks "$PR_REVIEW" 'Remotion' "no Remotion"
assert_lacks "$PR_REVIEW" 'video.mp4' "no video output"
assert_lacks "$PR_REVIEW" 'ffprobe' "no ffmpeg step"
PR_REVIEW_FLAT="${(j: :)${=PR_REVIEW}}"
assert_contains "$PR_REVIEW_FLAT" '`process` and `setbacks`: optional. Take them from the review rounds, the fix rounds and the CI failures of the PR.' "Step 4: setbacks and process are optional"
assert_contains "$PR_REVIEW_FLAT" 'Do not write a retro file outside a Loop run.' "Step 4: no retro text outside a Loop"

test_summary
