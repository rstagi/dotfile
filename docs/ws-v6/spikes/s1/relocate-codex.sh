#!/bin/sh
# Use the shared JSONL recipe for a new rollout copy.
if [ "${1:-}" = --rewrite ]; then
  exec "$(dirname "$0")/relocate-claude.sh" "$@"
fi
printf '%s\n' 'The experiment is not implemented.' >&2
exit 1
