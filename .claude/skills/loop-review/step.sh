#!/bin/zsh
# loop-review step tool. Mechanism only: it decides the attempt of one review step from the loop
# log and starts the keyed headless runner. It writes no loop event. wsd records them.
#
#   step.sh state <loop> <phase> <repo|none> <role> <round> <inner>
#       → "terminal<TAB><verdict>"        a loop.review {state: finished} record exists
#       → "run<TAB><step key>"            the attempt to create (or to create again)
#   step.sh run <loop> <phase> <repo|none> <role> <round> <inner> <provider> -- <runner args>
#       → "terminal<TAB><verdict>"        no process starts
#       → "started<TAB><step key><TAB><session id>"
#
# Step key: review:<phase>:<repo or none>:<role>:<round>:<inner>:attempt:<k>.
# The same key is sent again only while its attempt has no exit record. An exited attempt without
# a result (no report, checkpoint, crash) gets attempt k+1.

set -u -o pipefail

STEP_ROOT="${${(%):-%x}:A:h:h:h:h}"

step_main() {
  local cmd="${1:-}"
  (( $# )) && shift
  case "$cmd" in
    state) (( $# == 6 )) || { step_usage; return 1; }; step_state "$@" ;;
    run) step_run "$@" ;;
    *) step_usage; return 1 ;;
  esac
}

step_usage() {
  print -u2 -r -- 'usage: step.sh state <loop> <phase> <repo|none> <role> <round> <inner>'
  print -u2 -r -- '       step.sh run <loop> <phase> <repo|none> <role> <round> <inner> <provider> -- <runner args>'
}

step_state() {
  local loop="$1" phase="$2" repo="$3" role="$4" round="$5" inner="$6" prefix log verdict attempt
  prefix="review:${phase}:${repo}:${role}:${round}:${inner}:attempt:"
  log="$(ws log "$loop" --type loop.review)" || return 1
  # `ws log` prints {entries:[...]}. A bare array is accepted too.
  log="$(jq -c 'if type == "object" then .entries else . end' <<< "$log")" || return 1
  verdict="$(jq -r --arg prefix "$prefix" '
    [.[] | select(.body.state == "finished" and ((.body.key // "") | startswith($prefix)))
      | .body.verdict][0] // empty' <<< "$log")" || return 1
  if [[ -n "$verdict" ]]; then
    printf 'terminal\t%s\n' "$verdict"
    return 0
  fi
  attempt="$(jq -r --arg prefix "$prefix" '
    def attempt: (.body.key // "") | select(startswith($prefix)) | ltrimstr($prefix) | tonumber? // empty;
    ([.[] | select(.body.state == "started") | attempt] | max // 0) as $last
    | if $last == 0 then 1
      elif any(.[]; .body.state == "exited" and ((.body.key // "") == ($prefix + ($last | tostring)))) then $last + 1
      else $last end' <<< "$log")" || return 1
  printf 'run\t%s%s\n' "$prefix" "$attempt"
}

step_run() {
  (( $# >= 8 )) || { step_usage; return 1; }
  local loop="$1" phase="$2" repo="$3" role="$4" round="$5" inner="$6" provider="$7" state key session
  shift 7
  [[ "$1" == -- ]] || { step_usage; return 1; }
  shift
  state="$(step_state "$loop" "$phase" "$repo" "$role" "$round" "$inner")" || return 1
  [[ "$state" == run$'\t'* ]] || { print -r -- "$state"; return 0; }
  key="${state#run$'\t'}"
  session="$(ws session create --headless --key "$key" --loop "$loop" --phase "$phase" \
    --attempt "${key##*:}" --repo "$repo" --role "$role" --provider "$provider" \
    --round "$round" --inner "$inner" -- \
    zsh "${LOOP_REVIEW_RUNNER:-$STEP_ROOT/loop-runner.sh}" --review-key "$key" "$@")" || return 1
  printf 'started\t%s\t%s\n' "$key" "$session"
}

[[ "$ZSH_EVAL_CONTEXT" == *:file* ]] || step_main "$@"
