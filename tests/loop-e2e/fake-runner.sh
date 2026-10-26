#!/bin/sh
# Fake phase runner. Usage: fake-runner.sh <phase> <file>   (cwd = lane worktree)
# It writes <file>, runs the runner Verify, and commits. A Verify failure exits 1 and commits nothing.
set -u
phase="$1" file="$2"
printf 'phase %s\n' "$phase" > "$file"
if ! sh "$(dirname "$0")/verify.sh" runner; then
  rm -f "$file"
  exit 1
fi
git add "$file"
git -c user.email=e2e@example.com -c user.name=e2e commit -qm "phase $phase: add $file"
