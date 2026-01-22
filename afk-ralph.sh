#!/bin/bash
set -e

if [ -z "$1" ]; then
  echo "Usage: $0 <iterations>"
  exit 1
fi

# Retrieve Perplexity API key from 1password if not set
if [ -z "$PERPLEXITY_API_KEY" ]; then
  PERPLEXITY_API_KEY=$(op read "op://Private/Perplexity API Key/credential")
  export PERPLEXITY_API_KEY
fi

for ((i=1; i<=$1; i++)); do
  echo "=== Iteration $i/$1 ==="

  result=$(docker sandbox run claude --permission-mode acceptEdits -p "@PRD.md @progress.txt \
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

echo "=== Reached max iterations ($1) ==="
