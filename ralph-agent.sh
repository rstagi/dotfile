#!/bin/zsh
set -e

# Ralph Agent — autonomous Claude iterations on issue PRDs
# Supports GitHub and Linear as sources

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$HOME/dotfile/.zshrc_claude_ext"

# --- Argument parsing & interactive prompt ---

usage() {
  echo "Usage: $0 [--github <issue-number>|--linear <id>] [max-iterations] [model]"
  echo ""
  echo "Interactive mode (no args): prompts for source and ID"
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

SOURCE=""
MAX_ITERATIONS=20
MODEL_PARAM="O"

# Parse flags
while [[ $# -gt 0 ]]; do
  case "$1" in
  --github | -g)
    SOURCE="github"
    shift
    # Next arg is the issue number (if present and not a flag)
    if [[ $# -gt 0 && "$1" != --* && "$1" != -* ]]; then
      ISSUE_NUMBER="$1"
      shift
    fi
    ;;
  --linear | -l)
    SOURCE="linear"
    shift
    if [[ $# -gt 0 && "$1" != --* && "$1" != -* ]]; then
      LINEAR_ID="$1"
      shift
    fi
    ;;
  --help | -h)
    usage
    ;;
  *)
    # Positional args: max-iterations, model
    if [[ "$1" =~ ^[0-9]+$ ]]; then
      MAX_ITERATIONS="$1"
    elif [[ "$1" =~ ^[OoSsHhGg]$ ]]; then
      MODEL_PARAM="$1"
    else
      echo "Error: unknown argument '$1'"
      usage
    fi
    shift
    ;;
  esac
done

# Interactive source selection if not specified
if [ -z "$SOURCE" ]; then
  echo "Source?"
  echo "  1) GitHub"
  echo "  2) Linear"
  read -r "source_choice?> "
  case "$source_choice" in
  1 | g | G | github) SOURCE="github" ;;
  2 | l | L | linear) SOURCE="linear" ;;
  *)
    echo "Invalid choice"
    exit 1
    ;;
  esac
fi

# Load source adapter
source "$SCRIPT_DIR/ralph-source-$SOURCE.sh"

# --- Model setup ---

case "$MODEL_PARAM" in
[Oo]) MODELS=("opus" "sonnet" "glm") ;;
[Ss]) MODELS=("sonnet" "glm") ;;
[Hh]) MODELS=("haiku" "sonnet" "glm") ;;
[Gg]) MODELS=("glm" "sonnet") ;;
*)
  echo "Error: invalid model '$MODEL_PARAM'. Use O/o, S/s, H/h, or G/g."
  usage
  ;;
esac

CURRENT_MODEL="${MODELS[1]}"
MODEL_INDEX=1

# --- Repo info ---

REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null)
if [ -z "$REPO" ]; then
  echo "Error: not in a git repo or gh not configured"
  exit 1
fi
REPO_NAME=$(basename "$REPO")
WORKTREE_BASE="$HOME/.ralph-worktrees"

# --- Ralph tools ---

setup_ralph_tools() {
  setup_claude_tools
  load_secret ZAI_API_KEY "op://Private/Z.AI API Key/credential"
}

# --- Model command wrapper ---

claude_model() {
  local model="$1"
  shift
  case "$model" in
  glm) claudeg "$@" ;;
  claude-*) claude --model "$model" "$@" ;;
  *) claude "$@" ;;
  esac
}

# --- Worktree setup ---

setup_worktree() {
  local suffix
  suffix=$(source_worktree_suffix)
  WORKTREE_PATH="$WORKTREE_BASE/$REPO_NAME-$suffix"
  BRANCH_NAME=$(source_branch_name)

  if [ -d "$WORKTREE_PATH" ]; then
    echo "Worktree already exists at $WORKTREE_PATH"
    cd "$WORKTREE_PATH"
    return
  fi

  echo "Creating worktree at $WORKTREE_PATH..."
  mkdir -p "$WORKTREE_BASE"

  if ! git show-ref --verify --quiet "refs/heads/$BRANCH_NAME"; then
    git branch "$BRANCH_NAME"
  fi

  git worktree add "$WORKTREE_PATH" "$BRANCH_NAME"
  cd "$WORKTREE_PATH"

  {
    echo "# Progress for $SOURCE_LABEL"
    echo ""
    echo "Started: $(date)"
    echo ""
  } >progress.txt
}

# --- Cleanup on error ---

cleanup_on_error() {
  local msg="$1"
  source_comment "Ralph encountered an error: $msg

Worktree: \`$WORKTREE_PATH\`
Branch: \`$BRANCH_NAME\`

Manual intervention required."
  exit 1
}

# --- Main execution ---

main() {
  setup_ralph_tools

  # Initialize source (prompts for ID if needed)
  source_init

  # Fetch PRD
  source_fetch_prd

  # Write PRD to temp file
  local prd_file
  prd_file=$(mktemp)
  echo "$PRD" >"$prd_file"

  setup_worktree

  # Copy PRD to worktree
  cp "$prd_file" PRD.md
  rm "$prd_file"

  # Notify source we're starting
  source_on_start

  source_comment "Ralph starting on $SOURCE_LABEL

- Worktree: \`$WORKTREE_PATH\`
- Branch: \`$BRANCH_NAME\`
- Model: $CURRENT_MODEL
- Max iterations: $MAX_ITERATIONS"

  # jq filters
  local stream_text='select(.type == "assistant").message.content[]? | select(.type == "text").text // empty | gsub("\n"; "\r\n") | . + "\r\n\n"'
  local final_result='select(.type == "result").result // empty'

  for ((i = 1; i <= MAX_ITERATIONS; i++)); do
    local iter_start
    iter_start=$(date +%s)
    echo "=== Iteration $i/$MAX_ITERATIONS [$(date '+%H:%M:%S')] ==="

    local MAX_RETRIES=3
    local RETRY_DELAY=5
    local result=""
    local rate_limit_detected=false

    for ((retry = 1; retry <= MAX_RETRIES; retry++)); do
      local tmpfile
      tmpfile=$(mktemp)
      trap "rm -f $tmpfile" EXIT

      # Background timestamp printer (every 5 min)
      (while true; do
        sleep 300
        echo "[timestamp: $(date '+%H:%M:%S')]"
      done) &
      local timestamp_pid=$!
      trap "rm -f $tmpfile; kill $timestamp_pid 2>/dev/null" EXIT

      claude_model "$CURRENT_MODEL" \
        --verbose \
        --print \
        --output-format stream-json \
        --allow-dangerously-skip-permissions \
        --permission-mode bypassPermissions \
        "You are running inside a ralph session (autonomous issue worker). See global CLAUDE.md for ralph-specific instructions.

@PRD.md @progress.txt

Find next incomplete task and execute it. ONLY DO ONE TASK PER ITERATION.
When ALL tasks complete: <promise>COMPLETE</promise>
On blocking error: <error>DESCRIPTION</error>" |
        grep --line-buffered '^{' |
        tee "$tmpfile" |
        jq --unbuffered -rj "$stream_text"

      kill $timestamp_pid 2>/dev/null

      result=$(jq -r "$final_result" "$tmpfile")
      rm -f "$tmpfile"

      # Rate limit → model fallback
      if echo "$result" | grep -qE "rate.limit|429|too.many.requests"; then
        echo "=== Rate limit detected on $CURRENT_MODEL ==="
        rate_limit_detected=true
        break
      fi

      # Transient errors → retry
      if echo "$result" | grep -qE "No messages returned|ECONNRESET|ETIMEDOUT|503|502"; then
        echo "=== Transient error (attempt $retry/$MAX_RETRIES), retrying in ${RETRY_DELAY}s ==="
        sleep "$RETRY_DELAY"
        RETRY_DELAY=$((RETRY_DELAY * 2))
        continue
      fi

      # Empty result → retry
      if [ -z "$result" ]; then
        echo "=== Empty result (attempt $retry/$MAX_RETRIES), retrying in ${RETRY_DELAY}s ==="
        sleep "$RETRY_DELAY"
        RETRY_DELAY=$((RETRY_DELAY * 2))
        continue
      fi

      break
    done

    # Model fallback on rate limit
    if [ "$rate_limit_detected" = true ]; then
      MODEL_INDEX=$((MODEL_INDEX + 1))
      if [ $MODEL_INDEX -le ${#MODELS[@]} ]; then
        CURRENT_MODEL="${MODELS[$MODEL_INDEX]}"
        echo "=== Falling back to $CURRENT_MODEL ==="
        ((i--))
        continue
      else
        echo "=== All models exhausted, skipping iteration ==="
        continue
      fi
    fi

    # All retries exhausted
    if [ -z "$result" ] && [ "$rate_limit_detected" = false ]; then
      echo "=== All retries exhausted, skipping iteration ==="
      continue
    fi

    # Iteration timing
    local iter_end
    iter_end=$(date +%s)
    local iter_duration=$((iter_end - iter_start))
    local iter_mins=$((iter_duration / 60))
    local iter_secs=$((iter_duration % 60))
    echo "=== Iteration $i done [$(date '+%H:%M:%S')] (${iter_mins}m ${iter_secs}s) ==="

    # Completion
    if echo "$result" | grep -q "<promise>COMPLETE</promise>"; then
      echo "=== All tasks complete! ==="
      source_on_complete
      source_create_pr "$BRANCH_NAME"
      exit 0
    fi

    # Error
    if echo "$result" | grep -q "<error>"; then
      local error_msg
      error_msg=$(echo "$result" | grep -o "<error>.*</error>" | sed 's/<[^>]*>//g')
      cleanup_on_error "$error_msg"
    fi

    # Progress update every 5 iterations
    if [ $((i % 5)) -eq 0 ]; then
      source_comment "Ralph progress update (iteration $i/$MAX_ITERATIONS):

\`\`\`
$(tail -20 progress.txt)
\`\`\`"
    fi
  done

  echo "=== Reached max iterations ($MAX_ITERATIONS) ==="
  source_comment "Ralph reached max iterations ($MAX_ITERATIONS) without completing.

Last progress:
\`\`\`
$(tail -20 progress.txt)
\`\`\`

Manual intervention may be needed."
}

main
