#!/bin/bash
set -e

# Ralph GitHub Integration
# Runs autonomous Claude iterations on a GitHub issue PRD

usage() {
  echo "Usage: $0 <issue-number> [max-iterations]"
  echo ""
  echo "Runs autonomous Claude on a GitHub issue PRD."
  echo "Creates worktree, logs progress as comments, opens draft PR."
  exit 1
}

if [ -z "$1" ]; then
  usage
fi

ISSUE_NUMBER="$1"
MAX_ITERATIONS="${2:-20}"
WORKTREE_BASE="$HOME/.ralph-worktrees"

# Get repo info
REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null)
if [ -z "$REPO" ]; then
  echo "Error: not in a git repo or gh not configured"
  exit 1
fi
REPO_NAME=$(basename "$REPO")

# Load secrets from 1password
load_secrets() {
  if [ -z "$PERPLEXITY_API_KEY" ]; then
    PERPLEXITY_API_KEY=$(op read "op://Private/Perplexity API Key/credential")
    export PERPLEXITY_API_KEY
  fi
  if [ -z "$CONTEXT7_API_KEY" ]; then
    CONTEXT7_API_KEY=$(op read "op://Private/Context7 API Key/credential")
    export CONTEXT7_API_KEY
  fi
  if [ -z "$BROWSER_USE_API_KEY" ]; then
    BROWSER_USE_API_KEY=$(op read "op://Private/Browser-use API Key/credential")
    export BROWSER_USE_API_KEY
  fi
}

# Fetch PRD from issue body
fetch_prd() {
  echo "Fetching PRD from issue #$ISSUE_NUMBER..."
  PRD=$(gh issue view "$ISSUE_NUMBER" --json body -q .body)
  if [ -z "$PRD" ]; then
    echo "Error: could not fetch issue body"
    exit 1
  fi
  echo "$PRD"
}

# Setup git worktree for isolation
setup_worktree() {
  WORKTREE_PATH="$WORKTREE_BASE/$REPO_NAME-issue-$ISSUE_NUMBER"
  BRANCH_NAME="ralph/issue-$ISSUE_NUMBER"

  if [ -d "$WORKTREE_PATH" ]; then
    echo "Worktree already exists at $WORKTREE_PATH"
    cd "$WORKTREE_PATH"
    return
  fi

  echo "Creating worktree at $WORKTREE_PATH..."
  mkdir -p "$WORKTREE_BASE"

  # Create branch from current HEAD if doesn't exist
  if ! git show-ref --verify --quiet "refs/heads/$BRANCH_NAME"; then
    git branch "$BRANCH_NAME"
  fi

  git worktree add "$WORKTREE_PATH" "$BRANCH_NAME"
  cd "$WORKTREE_PATH"

  # Initialize progress file
  {
    echo "# Progress for Issue #$ISSUE_NUMBER"
    echo ""
    echo "Started: $(date)"
    echo ""
  } > progress.txt
}

# Post comment to issue
comment() {
  gh issue comment "$ISSUE_NUMBER" --body "$1"
}

# Cleanup on error
cleanup_on_error() {
  local msg="$1"
  comment "Ralph encountered an error: $msg

Worktree: \`$WORKTREE_PATH\`
Branch: \`$BRANCH_NAME\`

Manual intervention required."
  exit 1
}

# Main execution
main() {
  load_secrets

  PRD=$(fetch_prd)

  # Write PRD to temp file
  PRD_FILE=$(mktemp)
  echo "$PRD" > "$PRD_FILE"

  setup_worktree

  # Copy PRD to worktree
  cp "$PRD_FILE" PRD.md
  rm "$PRD_FILE"

  comment "Ralph starting on issue #$ISSUE_NUMBER

- Worktree: \`$WORKTREE_PATH\`
- Branch: \`$BRANCH_NAME\`
- Max iterations: $MAX_ITERATIONS"

  # jq filter to extract streaming text from assistant messages
  stream_text='select(.type == "assistant").message.content[]? | select(.type == "text").text // empty | gsub("\n"; "\r\n") | . + "\r\n\n"'

  # jq filter to extract final result
  final_result='select(.type == "result").result // empty'

  for ((i=1; i<=MAX_ITERATIONS; i++)); do
    iter_start=$(date +%s)
    echo "=== Iteration $i/$MAX_ITERATIONS [$(date '+%H:%M:%S')] ==="

    # Run Claude with retry on transient errors
    MAX_RETRIES=3
    RETRY_DELAY=5
    result=""

    for ((retry=1; retry<=MAX_RETRIES; retry++)); do
      tmpfile=$(mktemp)
      trap "rm -f $tmpfile" EXIT

      # Background timestamp printer (every 5 min)
      ( while true; do sleep 300; echo "[timestamp: $(date '+%H:%M:%S')]"; done ) &
      timestamp_pid=$!
      trap "rm -f $tmpfile; kill $timestamp_pid 2>/dev/null" EXIT

      claude \
        --verbose \
        --print \
        --output-format stream-json \
        --allow-dangerously-skip-permissions \
        --permission-mode bypassPermissions \
        "You are running inside a ralphg session (autonomous GitHub issue worker). See global CLAUDE.md for ralphg-specific instructions.

@PRD.md @progress.txt

Find next incomplete task and execute it. ONLY DO ONE TASK PER ITERATION.
When ALL tasks complete: <promise>COMPLETE</promise>
On blocking error: <error>DESCRIPTION</error>" \
      | grep --line-buffered '^{' \
      | tee "$tmpfile" \
      | jq --unbuffered -rj "$stream_text"

      # Stop timestamp printer
      kill $timestamp_pid 2>/dev/null

      result=$(jq -r "$final_result" "$tmpfile")
      rm -f "$tmpfile"

      # Check for transient errors (API issues, empty responses)
      if echo "$result" | grep -qE "No messages returned|ECONNRESET|ETIMEDOUT|rate.limit|503|502"; then
        echo "=== Transient error detected (attempt $retry/$MAX_RETRIES), retrying in ${RETRY_DELAY}s ==="
        sleep "$RETRY_DELAY"
        RETRY_DELAY=$((RETRY_DELAY * 2))
        continue
      fi

      # Empty result might indicate failure
      if [ -z "$result" ]; then
        echo "=== Empty result (attempt $retry/$MAX_RETRIES), retrying in ${RETRY_DELAY}s ==="
        sleep "$RETRY_DELAY"
        RETRY_DELAY=$((RETRY_DELAY * 2))
        continue
      fi

      # Success - break retry loop
      break
    done

    # If all retries exhausted with no result, skip iteration
    if [ -z "$result" ]; then
      echo "=== All retries exhausted, skipping iteration ==="
      continue
    fi

    # Iteration timing
    iter_end=$(date +%s)
    iter_duration=$((iter_end - iter_start))
    iter_mins=$((iter_duration / 60))
    iter_secs=$((iter_duration % 60))
    echo "=== Iteration $i done [$(date '+%H:%M:%S')] (${iter_mins}m ${iter_secs}s) ==="

    # Check for completion
    if echo "$result" | grep -q "<promise>COMPLETE</promise>"; then
      echo "=== All tasks complete! ==="

      # Push and create PR
      git push -u origin "$BRANCH_NAME"

      PR_URL=$(gh pr create --draft \
        --title "feat: resolve issue #$ISSUE_NUMBER" \
        --body "Resolves #$ISSUE_NUMBER

## Summary
Automated implementation by Ralph.

## Progress Log
\`\`\`
$(cat progress.txt)
\`\`\`")

      comment "Ralph completed all tasks!

PR: $PR_URL"

      exit 0
    fi

    # Check for errors
    if echo "$result" | grep -q "<error>"; then
      error_msg=$(echo "$result" | grep -o "<error>.*</error>" | sed 's/<[^>]*>//g')
      cleanup_on_error "$error_msg"
    fi

    # Progress update every 5 iterations
    if [ $((i % 5)) -eq 0 ]; then
      comment "Ralph progress update (iteration $i/$MAX_ITERATIONS):

\`\`\`
$(tail -20 progress.txt)
\`\`\`"
    fi
  done

  echo "=== Reached max iterations ($MAX_ITERATIONS) ==="
  comment "Ralph reached max iterations ($MAX_ITERATIONS) without completing.

Last progress:
\`\`\`
$(tail -20 progress.txt)
\`\`\`

Manual intervention may be needed."
}

main
