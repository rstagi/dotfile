#!/bin/bash
set -e

# Ralph GitHub Integration
# Runs autonomous Claude iterations on a GitHub issue PRD

usage() {
  echo "Usage: $0 <issue-number> [max-iterations] [model]"
  echo ""
  echo "Models:"
  echo "  O/o - Opus (default)"
  echo "  S/s - Sonnet"
  echo "  H/h - Haiku"
  echo "  G/g - GLM (via Z.AI)"
  echo ""
  echo "Fallback chains on rate limit:"
  echo "  Opus → Sonnet → GLM"
  echo "  Sonnet → GLM"
  echo "  Haiku → Sonnet → GLM"
  echo "  GLM → Sonnet"
  exit 1
}

if [ -z "$1" ]; then
  usage
fi

ISSUE_NUMBER="$1"
MAX_ITERATIONS="${2:-20}"
MODEL_PARAM="${3:-O}"
WORKTREE_BASE="$HOME/.ralph-worktrees"

# Map model param to models (primary + fallback chain)
case "$MODEL_PARAM" in
  [Oo])  # Opus → Sonnet → GLM
    MODELS=("claude-opus-4-5" "claude-sonnet-4.5" "glm")
    ;;
  [Ss])  # Sonnet → GLM
    MODELS=("claude-sonnet-4.5" "glm")
    ;;
  [Hh])  # Haiku → Sonnet → GLM
    MODELS=("claude-haiku-4-5" "claude-sonnet-4.5" "glm")
    ;;
  [Gg])  # GLM → Sonnet
    MODELS=("glm" "claude-sonnet-4.5")
    ;;
  *)
    echo "Error: invalid model '$MODEL_PARAM'. Use O/o, S/s, H/h, or G/g."
    usage
    ;;
esac

CURRENT_MODEL="${MODELS[0]}"
MODEL_INDEX=0

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
  if [ -z "$ZAI_API_KEY" ]; then
    ZAI_API_KEY=$(op read "op://Private/Z.AI API Key/credential")
    export ZAI_API_KEY
  fi
}

# Model command wrapper
claude_model() {
  local model="$1"
  shift

  case "$model" in
    glm)
      ANTHROPIC_BASE_URL="https://api.z.ai/api/anthropic" \
      ANTHROPIC_AUTH_TOKEN="$ZAI_API_KEY" \
      ANTHROPIC_MODEL="glm-4.6" \
      claude "$@"
      ;;
    claude-*)
      # For Anthropic models, pass -m flag
      claude -m "$model" "$@"
      ;;
    *)
      claude "$@"
      ;;
  esac
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
- Model: $CURRENT_MODEL
- Max iterations: $MAX_ITERATIONS"

  # jq filter to extract streaming text from assistant messages
  stream_text='select(.type == "assistant").message.content[]? | select(.type == "text").text // empty | gsub("\n"; "\r\n") | . + "\r\n\n"'

  # jq filter to extract final result
  final_result='select(.type == "result").result // empty'

  for ((i=1; i<=MAX_ITERATIONS; i++)); do
    iter_start=$(date +%s)
    echo "=== Iteration $i/$MAX_ITERATIONS [$(date '+%H:%M:%S')] ==="

    # Run Claude with retry on transient errors and model fallback
    MAX_RETRIES=3
    RETRY_DELAY=5
    result=""
    rate_limit_detected=false

    for ((retry=1; retry<=MAX_RETRIES; retry++)); do
      tmpfile=$(mktemp)
      trap "rm -f $tmpfile" EXIT

      # Background timestamp printer (every 5 min)
      ( while true; do sleep 300; echo "[timestamp: $(date '+%H:%M:%S')]"; done ) &
      timestamp_pid=$!
      trap "rm -f $tmpfile; kill $timestamp_pid 2>/dev/null" EXIT

      claude_model "$CURRENT_MODEL" \
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

      # Check for rate limit - trigger model fallback
      if echo "$result" | grep -qE "rate.limit|429|too.many.requests"; then
        echo "=== Rate limit detected on $CURRENT_MODEL ==="
        rate_limit_detected=true
        break  # Exit retry loop to trigger fallback
      fi

      # Check for other transient errors (API issues, empty responses)
      if echo "$result" | grep -qE "No messages returned|ECONNRESET|ETIMEDOUT|503|502"; then
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

    # Model fallback on rate limit
    if [ "$rate_limit_detected" = true ]; then
      MODEL_INDEX=$((MODEL_INDEX + 1))
      if [ $MODEL_INDEX -lt ${#MODELS[@]} ]; then
        CURRENT_MODEL="${MODELS[$MODEL_INDEX]}"
        echo "=== Falling back to $CURRENT_MODEL ==="
        # Retry entire loop with new model
        ((i--))  # Redo this iteration
        continue
      else
        echo "=== All models exhausted, skipping iteration ==="
        continue
      fi
    fi

    # If all retries exhausted with no result (and not rate limited), skip iteration
    if [ -z "$result" ] && [ "$rate_limit_detected" = false ]; then
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
