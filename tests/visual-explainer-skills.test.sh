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

echo "visual explainer: the page renders with its walkthrough, then the screenshots; there is no video"
assert_contains "$(cat "$EXPLAINER")" '### 6. Render the page and the screenshots' "page render step"
assert_contains "$(cat "$EXPLAINER")" 'node <skill>/scripts/render.mjs --props <output-dir>/props.json --narration <output-dir>/narration.json --out <output-dir>' "page render with the walkthrough"
assert_contains "$(cat "$EXPLAINER")" 'node <skill>/scripts/shoot.mjs --page <output-dir>/index.html --out <output-dir>/shots' "screenshot command"
assert_contains "$(cat "$EXPLAINER")" '`{"graph.html": "<output-dir>/graph.html"}`' "publish carries the graph page"
assert_contains "$(cat "$EXPLAINER")" 'caller renders the page and the screenshots, and publishes them' "a headless runner writes props only"
assert_lacks "$(cat "$EXPLAINER")" 'render-video' "no video renderer"
assert_lacks "$(cat "$EXPLAINER")" 'Remotion' "no Remotion"
[[ ! -e "$ROOT/.claude/skills/visual-explainer/video" && ! -e "$ROOT/.claude/skills/visual-explainer/scripts/render-video.mjs" ]]
assert_exit "$?" "0" "the video project and its renderer are gone"

echo "visual explainer: the narration has one commentary text for each walkthrough step"
assert_contains "$(cat "$EXPLAINER")" '### 4. Write the narration' "narration step before the check"
assert_contains "$(cat "$EXPLAINER")" '1. Explain. Do not repeat the section.' "narration explains"
assert_contains "$(cat "$EXPLAINER")" 'Use 10 to 90 words for each step and 80 to 450 words in total.' "narration length"
assert_contains "$(cat "$EXPLAINER")" '`ship`: `intro`, `tldr`, `done-when`, `changes`, `findings`, `hand-test`, `rollback`, `outro`.' "ship scene order"
assert_lacks "$(cat "$EXPLAINER")" 'narration-v1' "no legacy narration"

echo "visual explainer: the report sections, checkpoints, relations and GitHub media"
assert_contains "$(cat "$EXPLAINER")" '**Relation fields (optional, both modes):**' "relation fields are documented"
assert_contains "$(cat "$EXPLAINER")" '**Checkpoint (`ship` mode):**' "checkpoints are documented"
assert_contains "$(cat "$EXPLAINER")" 'Only one
  group can have a sequence.' "one sequence diagram"
assert_contains "$(cat "$EXPLAINER")" '### 8. GitHub media' "GitHub media step"

echo "visual explainer: multiphase-plan shows a visual checkpoint for every plan except light ones"
assert_contains "$PLAN_SKILL" '### 5. Visual checkpoint (skipped in `light` mode)' "plan checkpoint step after registration"
assert_contains "$PLAN_SKILL" 'A `light` plan also skips the visual checkpoint (step 5).' "light skips the checkpoint"
assert_contains "$PLAN_SKILL" '### 6. Report' "report follows the checkpoint"
assert_contains "$PLAN_SKILL" 'Write `.loop/explainers/plan/narration.json` beside `props.json`, as the skill tells you.' "plan explainer writes the narration"
assert_contains "$PLAN_SKILL" '`rabbitHoles`: the `## Deferred / Not now` items' "rabbit holes come from the plan"

echo "visual explainer: pr-review Step 4 is a visual checkpoint before the post"
assert_contains "$PR_REVIEW" '## Step 4: Visual checkpoint (`visual-explainer` ship page, walkthrough and screenshots)' "Step 4 uses visual-explainer"
assert_contains "$PR_REVIEW" 'pr-review-output/<repo>-<pr>/' "Step 4 keeps the output location"
PR_REVIEW_FLAT="${(j: :)${=PR_REVIEW}}"
assert_contains "$PR_REVIEW_FLAT" '`process` and `setbacks`: optional. Take them from the review rounds, the fix rounds and the CI failures of the PR.' "Step 4: setbacks and process are optional"
assert_contains "$PR_REVIEW_FLAT" 'Do not write a retro file outside a Loop run.' "Step 4: no retro text outside a Loop"
assert_contains "$PR_REVIEW_FLAT" 'Wait for the approval before you post.' "the checkpoint waits for approval"
assert_contains "$PR_REVIEW" 'scripts/gh-media.mjs --repo <owner>/<repo> --key pr-<n>' "the posted review carries the media"
assert_lacks "$PR_REVIEW" 'video' "Step 4 makes no video"
[[ "${PR_REVIEW%%'## Step 4'*}" != *'## Posting to GitHub'* ]]
assert_exit "$?" "0" "Step 4 comes before the post"

echo "visual explainer: Loop explainers add the walkthrough, GitHub media and progress checkpoints"
assert_contains "$LOOP_SKILL" 'The explainer runner also writes `narration.json`, the walkthrough commentary.' "the Loop runner writes the narration"
assert_contains "$LOOP_SKILL" 'render the page with
     `--narration`' "resume renders the plan page with its walkthrough"
assert_lacks "$LOOP_SKILL" 'render-video' "the Loop makes no video"
assert_contains "$LOOP_SKILL" '**Progress checkpoint (best-effort, in parallel).**' "supervise runs progress checkpoints"
LOOP_FLAT="${(j: :)${=LOOP_SKILL}}"
assert_contains "$LOOP_FLAT" 'never pause or delay a phase for it' "checkpoints never delay a phase"
assert_contains "$(cat "$ROOT/.claude/skills/loop-execute/references/loop-protocol.md")" '### Progress checkpoints' "protocol defines the checkpoint trigger"

test_summary
