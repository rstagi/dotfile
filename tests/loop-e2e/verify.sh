#!/bin/sh
# Verify command. $1 = runner|merged. Each context fails while its budget file holds a count > 0.
set -u
ctx="${1:-merged}"
budget="${E2E_DIR:?}/fail-budget-$ctx"
if [ -f "$budget" ] && [ "$(cat "$budget")" -gt 0 ]; then
  echo $(( $(cat "$budget") - 1 )) > "$budget"
  echo "verify($ctx): scripted failure" >&2
  exit 1
fi
exit 0
