#!/bin/zsh
# Fake claude/codex/grok engine for the --model path. loop-runner.sh starts it as a phase or review runner.
# It finds its job from the argv of the runner above it (--run-dir, --review-key, --publish-output)
# and from the lane branch. It does the job of the role without a model and writes status.json.
set -u
engine="${0:t}"
# loop-runner.sh asks codex for its model catalog to resolve a family such as @sol.
if [[ "$engine" == codex && "${1:-}" == debug ]]; then
  print -r -- '{"models":[{"slug":"gpt-5.5-sol","visibility":"list"}]}'
  exit 0
fi
print -r -- '{"type":"system","subtype":"init","session_id":"fake-'"$engine"'","model":"fake-'"$engine"'"}'

run_dir="" key="" report=""
pid=$$
while (( pid > 1 )); do
  args="$(ps -o args= -p "$pid" 2>/dev/null)"
  [[ -n "$run_dir" || "$args" != *--run-dir* ]] || run_dir="${${args#*--run-dir }%% *}"
  [[ -n "$key" || "$args" != *--review-key* ]] || key="${${args#*--review-key }%% *}"
  [[ -n "$report" || "$args" != *--publish-output* ]] || report="${${args#*--publish-output }%% *}"
  pid="$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ')"
  [[ -n "$pid" ]] || break
done
# The runner reads the session of its result from RUN_DIR/ws-session. The fake engine is a direct child of the
# keyed runner session, so WS_SESSION_ID is that session.
[[ -z "$run_dir" || -z "${WS_SESSION_ID:-}" || -s "$run_dir/ws-session" ]] || print -r -- "$WS_SESSION_ID" > "$run_dir/ws-session"
# Under a ws session the engine is a child of wsd. Its parent session is the runner, and the command of the
# runner session holds the same arguments.
if [[ -z "$run_dir" && -n "${WS_SESSION_ID:-}" ]]; then
  parent="$(ws session get "$WS_SESSION_ID" --json | jq -r '.parent_session_id // empty')"
  command_json="$([[ -z "$parent" ]] || ws session get "$parent" --json | jq -c '.body.command // []')"
  arg_after() { jq -r --arg flag "$1" '. as $a | (index($flag)) as $i | if $i == null then "" else $a[$i + 1] end' <<< "$command_json"; }
  run_dir="$(arg_after --run-dir)"; key="$(arg_after --review-key)"; report="$(arg_after --publish-output)"
fi
[[ -n "$run_dir" ]] || { print -u2 "fake engine: no --run-dir above me"; exit 1; }

done_status() { jq -cn --arg summary "$1" '{outcome:"done", summary:$summary, checkpoint:false}' > "$run_dir/status.json"; }

if [[ -n "$key" ]]; then
  role="${${(s.:.)key}[4]}"
  phase="${${(s.:.)key}[2]}"
  mkdir -p "${report:h}"
  case "$role" in
  review|respond) print -r -- 'No blocking finding. The lane files match the plan.' > "$report"; print -r -- APPROVE >> "$report" ;;
  critic) print -r -- 'The review is complete and evidence based.' > "$report"; print -r -- AGREE >> "$report" ;;
  final)
    print -r -- 'Final review of the e2e milestone: no blocking finding.' > "$report"
    jq -cn '{outcome:"done", summary:"final review posted", event:"APPROVE", commentUrl:"https://github.com/acme/e2e/pull/1#fake-review", checkpoint:false}' > "$run_dir/status.json"
    print -r -- '{"type":"result","result":"done"}'; exit 0 ;;
  *) print -r -- 'Nothing to do.' > "$report" ;;
  esac
  done_status "fake $role for phase $phase"
  print -r -- '{"type":"result","result":"done"}'; exit 0
fi

branch="$(git branch --show-current 2>/dev/null)"
if [[ "$branch" == feat/e2e-* ]]; then
  file="${branch#feat/e2e-}.txt"
  print -r -- "phase on $branch" > "$file"
  git add "$file"
  git -c user.email=e2e@example.com -c user.name=e2e commit -qm "add $file"
  done_status "added $file"
else
  done_status "fake engine: no job for this run"
fi
print -r -- '{"type":"result","result":"done"}'
