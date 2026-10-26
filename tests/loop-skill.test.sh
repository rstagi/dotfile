#!/bin/zsh
# loop skill: prose contract (start, lane, output kinds, merge exits, parks, recycle) + installer.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
SKILL="$(cat "$ROOT/.claude/skills/loop/SKILL.md" 2>/dev/null)"

echo "loop skill: frontmatter"
assert_contains "$SKILL" 'name: loop' "frontmatter name"
assert_contains "$SKILL" 'description:' "frontmatter description"

echo "loop skill: start rule"
assert_contains "$SKILL" 'loop-milestone.sh open --dir <loop dir>' "starts with open"
assert_contains "$SKILL" 'ws loop get <loop> --json' "reads the loop"
assert_contains "$SKILL" 'first milestone that has open phases' "starts at the first open milestone"
assert_contains "$SKILL" 'milestone 7' "milestone 7 example"

echo "loop skill: spawn and wait"
assert_contains "$SKILL" '`depends_on`' "depends_on edges"
assert_contains "$SKILL" 'mode `any`' "multi-target wait"
assert_contains "$SKILL" 'loop-runner.sh --loop-dir <loop dir>' "runner records the result"
assert_contains "$SKILL" 'phase:<n>:attempt:<k>' "work attempt key"

echo "loop skill: lane rule"
assert_contains "$SKILL" 'loop-repo.sh lane' "lane worktree"
assert_contains "$SKILL" 'as `repo`' "lane worktree is the repo"
assert_contains "$SKILL" 'the printed `base`' "printed base"
assert_contains "$SKILL" 'A retry uses the same worktree.' "retry reuses the lane"

echo "loop skill: start rule for each output kind"
assert_contains "$SKILL" '`file`, `doc` or `output.md`' "non-pr kinds"
assert_contains "$SKILL" '<loop dir>/outputs/p<N>/' "outputs cwd"
assert_contains "$SKILL" 'no repository and no base' "no repo for non-pr"

echo "loop skill: action for each loop-merge.sh exit"
assert_contains "$SKILL" 'loop-merge.sh --worktree <integration worktree> --lane-branch <Suggested branch>' "merge command"
assert_contains "$SKILL" 'Start no other merge while a merge failure is open.' "one merge failure at a time"
assert_contains "$SKILL" '`[status: done]`' "done after exit 0"
assert_contains "$SKILL" 'loop-plan.sh push' "plan push after exit 0"
assert_contains "$SKILL" 'before you spawn a phase that depends on it' "push before dependents"
assert_contains "$SKILL" '--finish --verify-cmd' "exit 2 finish"
assert_contains "$SKILL" '<integration worktree> --abort' "exit 2 abort"
assert_contains "$SKILL" '<integration worktree> --undo' "exit 4 undo"
assert_contains "$SKILL" 'Exit 3' "exit 3 row"
assert_contains "$SKILL" 'Exit 1' "exit 1 row"

echo "loop skill: reviews and PRs"
assert_contains "$SKILL" '`merged` (output `pr`) or `done` (other outputs)' "review start rule"
assert_contains "$SKILL" 'gh pr create' "opens one PR per repository"
assert_contains "$SKILL" 'loop-review' "uses loop-review"

echo "loop skill: park rules"
assert_contains "$SKILL" 'loop-milestone.sh park --dir <loop dir> --milestone <N> --prs' "merge park"
assert_contains "$SKILL" 'loop-milestone.sh resume --dir <loop dir>' "resume command"
assert_contains "$SKILL" 'ws loop start <loop dir>' "start command"
assert_contains "$SKILL" 'Register no continuation.' "no continuation at a merge park"
assert_contains "$SKILL" 'loop-milestone.sh park --kind accept --phase <n> --artifacts' "accept park"
assert_contains "$SKILL" 'ws loop accept' "accept"
assert_contains "$SKILL" 'ws loop reject' "reject"

echo "loop skill: quota, checks and decisions"
assert_contains "$SKILL" 'ws usage --preflight --plan <loop dir>/plan.md' "quota preflight"
assert_contains "$SKILL" 'ws loop check <loop>' "check before a merge"
assert_contains "$SKILL" 'ws loop ack' "human acknowledgment"
assert_contains "$SKILL" '`loop.decision`' "decision events"

echo "loop skill: recycle rule"
assert_contains "$SKILL" 'ws on me exit --spawn' "exit trigger on me"
assert_contains "$SKILL" 'Never register the exit trigger on the loop session' "never on the loop session"
assert_contains "$SKILL" 'ws done --output handoff.md' "done with handoff"

echo "loop skill: installer shares the skill"
assert_contains "$(<"$ROOT/install-agent-skills.sh")" $'\n  loop\n' "SHARED_SKILLS lists loop"

test_summary
