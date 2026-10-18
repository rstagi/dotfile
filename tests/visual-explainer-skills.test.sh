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
LOOP_SKILL="$(cat "$ROOT/.claude/skills/loop-execute/SKILL.md")"

echo "visual explainer: the skill text passes the STE linter"
python3 "$ROOT/.claude/skills/asd-ste100/scripts/ste-lint.py" "$EXPLAINER" > /dev/null
assert_exit "$?" "0" "visual-explainer SKILL.md has no hard STE violations"
assert_contains "$(cat "$EXPLAINER")" 'Do not write HTML, CSS or JS.' "agents write props only"
assert_contains "$(cat "$EXPLAINER")" '### 3. Check every hand-test command against the code' "ship commands are checked against code"

echo "visual explainer: the video step follows the page, and a failed video never blocks the page"
assert_contains "$(cat "$EXPLAINER")" '### 6. Render

```bash' "page render step"
assert_contains "$(cat "$EXPLAINER")" '### 7. Video' "video step after the page render"
assert_contains "$(cat "$EXPLAINER")" 'node <skill>/scripts/render-video.mjs --props <output-dir>/props.json --narration <output-dir>/narration.json --out <output-dir>' "video command"
assert_contains "$(cat "$EXPLAINER")" '--out <output-dir> --video video.mp4' "page renders again with the video"
assert_contains "$(cat "$EXPLAINER")" '`{"video.mp4": "<output-dir>/video.mp4"}`' "publish carries the video in the files map"
assert_contains "$(cat "$EXPLAINER")" 'A failed video never blocks the page.' "video failure never blocks the page"
assert_contains "$(cat "$EXPLAINER")" 'caller renders the page and the video' "a headless runner writes props only"
assert_contains "$(cat "$EXPLAINER")" 'Remotion is free for an individual and for a company of up to 3 people.' "license note"

echo "visual explainer: the narration explains, and the check binds it to the props"
assert_contains "$(cat "$EXPLAINER")" '### 4. Write the narration' "narration step before the check"
assert_contains "$(cat "$EXPLAINER")" '1. Explain. Do not read the slides aloud.' "narration explains"
assert_contains "$(cat "$EXPLAINER")" 'say what it shows, why it matters and what to look at' "narration structure"
assert_contains "$(cat "$EXPLAINER")" 'Use 30 to 180 words for each section and 300 to 900 words in total.' "narration length"
assert_contains "$(cat "$EXPLAINER")" 'It has no captions.' "no captions"

echo "visual explainer: multiphase-plan explains every plan except light ones"
assert_contains "$PLAN_SKILL" '### 5. Explain the plan (skipped in `light` mode)' "plan explainer step after registration"
assert_contains "$PLAN_SKILL" 'A `light` plan also skips the visual explainer (step 5).' "light skips the explainer"
assert_contains "$PLAN_SKILL" '### 6. Report' "report follows the explainer"

echo "visual explainer: pr-review Step 4 renders a ship page and its video through the skill"
assert_contains "$PR_REVIEW" '## Step 4: Visual report (`visual-explainer` ship page and video)' "Step 4 uses visual-explainer"
assert_contains "$PR_REVIEW" 'pr-review-output/<repo>-<pr>/' "Step 4 keeps the output location"
PR_REVIEW_FLAT="${(j: :)${=PR_REVIEW}}"
assert_contains "$PR_REVIEW_FLAT" '`process` and `setbacks`: optional. Take them from the review rounds, the fix rounds and the CI failures of the PR.' "Step 4: setbacks and process are optional"
assert_contains "$PR_REVIEW_FLAT" 'Do not write a retro file outside a Loop run.' "Step 4: no retro text outside a Loop"
assert_contains "$PR_REVIEW" 'video.mp4      # the narrated explainer video that render-video.mjs renders' "Step 4 lists the video output"
assert_contains "$PR_REVIEW" 'narration.json # the voiceover text that you write' "Step 4 writes the narration"
assert_contains "$PR_REVIEW" 'Do not build a Remotion project by hand.' "the skill owns the Remotion project"
assert_lacks "$PR_REVIEW" 'npx create-video' "no per-run Remotion scaffold"

echo "visual explainer: Loop explainers add the video, bounded at a park and a finish, detached at a resume"
assert_contains "$LOOP_SKILL" 'The step 5 notification waits for the video or its timeout.' "park notification waits for the video"
assert_contains "$LOOP_SKILL" 'The step 3 notification waits for the video or its timeout.' "finish notification waits for the video"
assert_contains "$LOOP_SKILL" 'The video never delays the respawn.' "resume video never delays the respawn"
assert_contains "$LOOP_SKILL" 'Video: the explainer runner also writes `narration.json`.' "the Loop runner writes the narration"

echo "visual explainer: multiphase-plan renders the narrated plan video, except for light plans"
assert_contains "$PLAN_SKILL" 'Write `.loop/explainers/plan/narration.json` beside `props.json`, as the skill tells you.' "plan explainer writes the narration"
assert_contains "$PLAN_SKILL" 'Render the page, then the video with its voiceover' "plan explainer renders the video"

test_summary
