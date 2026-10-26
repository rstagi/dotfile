#!/bin/zsh
# Scripted fake orchestrator of the skills engine e2e harness. It stands in for the `loop` skill.
# Env: WS_LOOP_ID, E2E_DIR, E2E_ROOT (repo root), E2E_SCENARIO, E2E_BROKEN (1 = write loop.phase itself).
# It makes the choices of the skill; the tools (ws, loop-repo.sh, loop-merge.sh, loop-milestone.sh) record the facts.
# A crash-and-resume run starts it again: every step is keyed, so a second run starts no step twice.
set -u
[[ -z "${E2E_TRACE:-}" ]] || set -x
[[ -z "${E2E_TRACE:-}" ]] || set -x
LOOP_DIR="$E2E_DIR/work/.loop"
REPO=acme/e2e
FAKES="$E2E_ROOT/tests/loop-e2e"
MERGE_VERIFY="sh $FAKES/verify.sh merged"

main() {
  "$E2E_ROOT/loop-milestone.sh" open --dir "$LOOP_DIR" || return $?
  RUN_ID="$(jq -er .runId "$LOOP_DIR/state.json")"
  INTEG="$(jq -er --arg r "$REPO" '.repositories[$r].integrationWorktree' "$LOOP_DIR/state.json")"
  if [[ "${E2E_BROKEN:-0}" == 1 ]]; then
    "$WS_BIN" event loop.phase --body '{"n":1,"status":"merged","session":null}' --target "$WS_LOOP_ID" || return $?
    return 0
  fi
  work_phase 1 feat/e2e-alpha alpha.txt || return $?
  work_phase 2 feat/e2e-beta beta.txt || return $?
  work_phase 3 feat/e2e-gamma gamma.txt || return $?
  review_phase 4 || return $?
  "$E2E_ROOT/loop-milestone.sh" park --dir "$LOOP_DIR" --milestone 1 \
    --prs '{"acme/e2e":"https://github.com/acme/e2e/pull/1"}'
}

# One work phase on its own lane worktree. A failed attempt or a failed merge starts the next attempt.
work_phase() {
  local n="$1" branch="$2" file="$3" attempt lane wt session rc
  for attempt in 1 2 3; do
    if merged $n; then return 0; fi
    lane="$("$E2E_ROOT/loop-repo.sh" lane --repo "$REPO" --run-id "$RUN_ID" --branch "$branch" --from "$INTEG")" || return $?
    wt="$(jq -er .worktree <<< "$lane")" || return 1
    session="$("$WS_BIN" session create --headless --json --inherit-env --loop "$WS_LOOP_ID" --phase "$n" \
      --attempt "$attempt" --key "phase:$n:attempt:$attempt" --cwd "$wt" \
      --env "E2E_DIR=$E2E_DIR" -- sh "$FAKES/fake-runner.sh" "$n" "$file" | jq -er .id)" || return $?
    "$WS_BIN" wait "$session" --for exit > /dev/null
    # The runner records no result for a failed Verify: the branch has no new commit.
    [[ "$(git -C "$wt" rev-parse HEAD)" != "$(git -C "$wt" rev-parse "$(jq -r .base <<< "$lane")")" ]] || continue
    "$E2E_ROOT/loop-merge.sh" --worktree "$INTEG" --lane-branch "$branch" --verify-cmd "$MERGE_VERIFY" \
      --phase "$n" --repository "$REPO"
    rc=$?
    case $rc in
    0) return 0 ;;
    4) "$E2E_ROOT/loop-merge.sh" --worktree "$INTEG" --undo --phase "$n" --repository "$REPO" || return $? ;;
    *) return $rc ;;
    esac
  done
  return 1
}

merged() {
  [[ "$("$WS_BIN" loop get "$WS_LOOP_ID" --json | jq -r --arg n "$1" '.phases[$n].status')" == merged ]]
}

review_phase() {
  local n="$1" step role provider
  for step in review:gpt critic:claude final:claude; do
    role="${step%%:*}" provider="${step##*:}"
    "$WS_BIN" session create --headless --json --inherit-env --loop "$WS_LOOP_ID" --phase "$n" --attempt 1 \
      --key "review:$n:$REPO:$role:1:1:attempt:1" --repo "$REPO" --role "$role" --provider "$provider" \
      --round 1 --inner 1 --stdout "$E2E_DIR/review-$role.out" --stderr "$E2E_DIR/review-$role.err" --env "E2E_DIR=$E2E_DIR" --env "E2E_SCENARIO=${E2E_SCENARIO:-happy}" \
      -- sh "$FAKES/fake-review.sh" "$role" "$n" "$provider" | jq -er .id | xargs -I{} "$WS_BIN" wait {} --for exit > /dev/null
  done
}

main "$@"
