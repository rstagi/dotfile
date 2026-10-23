#!/bin/zsh
# ws-sessions naming steer + ws-orchestrate skill: literal asserts on the prose contract.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

SESSIONS="$(cat "$ROOT/.claude/skills/ws-sessions/SKILL.md")"
ORCH_FILE="$ROOT/.claude/skills/ws-orchestrate/SKILL.md"
ORCH="$(cat "$ORCH_FILE")"

echo "ws skills: ws-sessions steers session naming"
assert_contains "$SESSIONS" '## Name your session' "naming section"
assert_contains "$SESSIONS" 'session_rename' "MCP rename tool"
assert_contains "$SESSIONS" 'ws name <name>' "CLI rename"
assert_contains "$SESSIONS" 'Also use it to name your session' "description triggers naming"

echo "ws skills: ws-orchestrate contract"
assert_contains "$ORCH" 'name: ws-orchestrate' "frontmatter name"
assert_contains "$ORCH" 'Do not edit files in a repo checkout.' "no repo edits"
assert_contains "$ORCH" 'Start exactly one child for each repo' "one child per repo"
assert_contains "$ORCH" 'Do not start a second child for that repo.' "reuse child"
assert_contains "$ORCH" 'ws new --repo' "child start command"
assert_contains "$ORCH" '--prompt' "child task prompt"
assert_contains "$ORCH" 'session_create' "MCP start tool"
assert_contains "$ORCH" 'ws done -' "child reports with ws done"
assert_contains "$ORCH" 'ws send <orchestrator>' "child asks orchestrator"
assert_contains "$ORCH" 'Do not merge' "no merge"
assert_contains "$ORCH" 'loop-execute' "Loop plans go to loop-execute"
assert_eq "$(( $(wc -l < "$ORCH_FILE") < 150 ))" "1" "skill stays under 150 lines"

test_summary
