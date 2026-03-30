#!/bin/zsh
# Ralph GitHub Source Adapter
# Implements the source interface for GitHub issues

ISSUE_NUMBER=""

source_init() {
  if [ -z "$ISSUE_NUMBER" ]; then
    read -r "ISSUE_NUMBER?Issue number: "
  fi

  if [ -z "$ISSUE_NUMBER" ]; then
    echo "Error: issue number required"
    exit 1
  fi
}

source_fetch_prd() {
  echo "Fetching PRD from GitHub issue #$ISSUE_NUMBER..."
  PRD=$(gh issue view "$ISSUE_NUMBER" --json body -q .body)
  if [ -z "$PRD" ]; then
    echo "Error: could not fetch issue body"
    exit 1
  fi
  SOURCE_LABEL="issue #$ISSUE_NUMBER"
}

source_comment() {
  gh issue comment "$ISSUE_NUMBER" --body "$1"
}

source_create_pr() {
  local branch="$1"
  git push -u origin "$branch"

  PR_URL=$(gh pr create --draft \
    --title "feat: resolve issue #$ISSUE_NUMBER" \
    --body "Resolves #$ISSUE_NUMBER

## Summary
Automated implementation by Ralph.

## Progress Log
\`\`\`
$(cat progress.txt)
\`\`\`")

  source_comment "Ralph completed all tasks!

PR: $PR_URL"
}

source_worktree_suffix() {
  echo "issue-$ISSUE_NUMBER"
}

source_branch_name() {
  echo "ralph/issue-$ISSUE_NUMBER"
}

source_on_start() {
  : # no-op for GitHub
}

source_on_complete() {
  : # no-op for GitHub
}
