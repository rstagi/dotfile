#!/bin/sh
# Fake AR reviewer. Usage: fake-review.sh <role> <phase> <provider>.
# It records the run, then writes the terminal loop.review result as the runner tool does.
# Scenario crash: the reviewer role waits for $E2E_DIR/release, so the harness can stop the orchestrator mid-review.
set -u
role="$1" phase="$2" provider="$3"
printf '%s\n' "$role" >> "${E2E_DIR:?}/review-runs.log"
if [ "$role" = review ] && [ "${E2E_SCENARIO:-}" = crash ]; then
  : > "$E2E_DIR/review-started"
  while [ ! -f "$E2E_DIR/release" ]; do sleep 0.05; done
fi
step="review:$phase:acme/e2e:$role:1:1:attempt:1"
body=$(jq -cn --argjson p "$phase" --arg role "$role" --arg prov "$provider" --arg key "$step" --arg sid "$WS_SESSION_ID" \
  '{key:$key, session:$sid, phase:$p, repo:"acme/e2e", role:$role, provider:$prov, round:1, inner:1, state:"finished", verdict:"APPROVE"}')
ws event loop.review --body "$body" --source tool --target "$WS_LOOP_ID" \
  --key "review-result:$step"
