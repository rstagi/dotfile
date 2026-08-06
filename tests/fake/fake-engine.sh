#!/bin/zsh
# Fake `claude` engine for loop-orchestrator tests. Ignores all CLI args. Emits one
# usage-bearing stream-json line to stdout (loop-orchestrator redirects stdout → transcript),
# then writes sub/status.json per the FAKE_OUTCOMES sequence (space-separated, 1 per instance).
#   crash  → exit 1 without writing status.json
#   hang   → sleep (never writes status.json) so the stall watchdog must kill it
#   <word> → status.json {outcome:<word>, tokens, recycleIndex:<instance-n>}
set -u
SUB="${LOOP_SUB_DIR:?LOOP_SUB_DIR required}"
cf="$SUB/.fake-count"
n=$(( $(cat "$cf" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$cf"

print -r -- '{"type":"assistant","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":20,"cache_creation_input_tokens":5}}}'

raw="${FAKE_OUTCOMES:-complete}"
outcomes=(${(s: :)raw})
outcome="${outcomes[$n]:-complete}"

[[ "$outcome" == "crash" ]] && exit 1
[[ "$outcome" == "hang" ]] && { sleep 20; exit 0; }
# recycle-noidx: a recycle status.json that OMITS recycleIndex → exercises the orchestrator's
# local-counter fallback.
[[ "$outcome" == "recycle-noidx" ]] && { jq -cn '{outcome:"recycle", summary:"no idx", tokens:151000}' > "$SUB/status.json"; exit 0; }

jq -cn --arg o "$outcome" --argjson n "$n" \
  '{outcome: $o, summary: ("fake instance " + ($n | tostring)), tokens: 151000, recycleIndex: $n}' \
  > "$SUB/status.json"
exit 0
