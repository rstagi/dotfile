#!/bin/bash
set -e

if [ -z "$1" ]; then
  echo "Usage: $0 <prd-file> [max-iterations]"
  exit 1
fi

PRD_FILE="$1"
MAX_ITERATIONS="${2:-20}"

if [ ! -f "$PRD_FILE" ]; then
  echo "Error: PRD file '$PRD_FILE' not found"
  exit 1
fi

# Retrieve Perplexity API key from 1password if not set
if [ -z "$PERPLEXITY_API_KEY" ]; then
  PERPLEXITY_API_KEY=$(op read "op://Private/Perplexity API Key/credential")
  export PERPLEXITY_API_KEY
fi

for ((i=1; i<=$MAX_ITERATIONS; i++)); do
  echo "=== Iteration $i/$MAX_ITERATIONS ==="

  result=$(docker sandbox run claude --permission-mode acceptEdits -p "@$PRD_FILE @progress.txt \
1. Read the PRD and progress file. \
2. Find the next incomplete task and implement it. \
3. Commit your changes. \
4. Update progress.txt with what you did. \
ONLY DO ONE TASK AT A TIME. \
When ALL tasks are complete, output: <promise>COMPLETE</promise>")

  echo "$result"

  if echo "$result" | grep -q "<promise>COMPLETE</promise>"; then
    echo "=== All tasks complete! ==="
    exit 0
  fi
done

echo "=== Reached max iterations ($MAX_ITERATIONS) ==="
