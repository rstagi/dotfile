#!/bin/zsh
# Skills engine end to end: tests/loop-e2e/run.sh --scenario <name>|--all [--model <id>]
# Scenarios: happy, verify-fail, crash, broken (a broken fake orchestrator must fail ws loop check). Everything runs in a sandbox: temporary HOME, WS_HOME, WS_STATE and
# WS_SOCK, a `tmux -L` server, a wsd built from this branch, a bare git origin and fake engines.
# Without --model a scripted fake orchestrator runs the loop. With --model the `loop` skill runs on that model.
set -u
HERE="${0:A:h}"
ROOT="${HERE:h:h}"
source "$ROOT/tests/lib.sh"
source "$ROOT/tests/home-guard.sh"

SCENARIOS=(happy verify-fail crash broken)
HANG_GUARD=60 # seconds. Only a hang guard: every wait ends on an event.
TMP="" DAEMON_PID="" ORCH_ID="" MODEL=""

main() {
  local scenario="" arg
  while (( $# )); do
    case "$1" in
    --scenario) scenario="$2"; shift 2 ;;
    --all) scenario=all; shift ;;
    --model) MODEL="$2"; shift 2 ;;
    *) print -u2 "usage: run.sh --scenario <name>|--all [--model <id>]"; return 2 ;;
    esac
  done
  [[ -n "$scenario" ]] || { print -u2 "usage: run.sh --scenario <name>|--all [--model <id>]"; return 2; }
  require_ws_test_binary || return 1
  # Traps live here: a zsh EXIT trap set inside a function runs when that function returns.
  trap teardown EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  trap 'exit 129' HUP
  if [[ "$scenario" == all ]]; then
    for arg in $SCENARIOS; do run_scenario "$arg"; done
  else
    (( ${SCENARIOS[(Ie)$scenario]} )) || { print -u2 "unknown scenario '$scenario'"; return 2; }
    run_scenario "$scenario"
  fi
  test_summary
}

run_scenario() {
  local name="$1"
  echo "loop-e2e: scenario $name"
  setup_sandbox "$name" || { TESTS_FAIL=$((TESTS_FAIL + 1)); return 1; }
  case "$name" in
  happy) scenario_happy ;;
  verify-fail) scenario_verify_fail ;;
  crash) scenario_crash ;;
  broken) scenario_broken ;;
  esac || { TESTS_FAIL=$((TESTS_FAIL + 1)); _red "  FAIL: scenario $name stopped early"; }
  teardown
}

scenario_happy() {
  run_orchestrator || return 1
  assert_parked
  assert_lane_isolation
  assert_check_clean
}

scenario_verify_fail() {
  echo 1 > "$E2E_DIR/fail-budget-runner"
  echo 1 > "$E2E_DIR/fail-budget-merged"
  run_orchestrator || return 1
  assert_parked
  assert_eq "$(cat "$E2E_DIR/fail-budget-runner")" 0 "the runner Verify failed one time"
  assert_eq "$(cat "$E2E_DIR/fail-budget-merged")" 0 "the merged-tree Verify failed one time"
  assert_origin_has_all_lanes
  assert_check_clean
}

scenario_crash() {
  export E2E_SCENARIO=crash
  start_orchestrator
  wait_for_file "$E2E_DIR/review-started" || return 1
  "$WS_BIN" session stop "$ORCH_ID" > /dev/null 2>&1
  "$WS_BIN" wait "$ORCH_ID" --for exit > /dev/null 2>&1
  : > "$E2E_DIR/release"
  run_orchestrator || return 1
  assert_parked
  assert_eq "$(sort "$E2E_DIR/review-runs.log" | uniq -d | wc -l | tr -d ' ')" 0 "no review step ran two times"
  assert_check_clean
}

# A broken orchestrator writes loop.phase {status: merged} itself, with no loop.merge event.
scenario_broken() {
  export E2E_BROKEN=1
  run_orchestrator || return 1
  local out
  out="$("$WS_BIN" loop check "$WS_LOOP_ID" --json)"
  assert_eq "$(jq -r '[.violations[] | select(.rule == "merged-needs-verify" and .severity == "hard")] | length' <<< "$out")" 1 \
    "a merged phase without loop.merge is a hard violation"
  assert_eq "$(jq -r '[.violations[] | select(.rule == "agent-source")] | length' <<< "$out")" 1 \
    "an agent loop.phase is a soft violation"
}

# --- sandbox -----------------------------------------------------------------------------------------------

setup_sandbox() {
  TMP="$(mktemp -d /tmp/loop-e2e.XXXXXX)"; TMP="${TMP:A}"
  home_guard_sandbox "$TMP/home"
  unset WS_SESSION_ID WS_SESSION WS_LOOP_ID WS_LOOP_PHASE WS_LOOP_ATTEMPT LOOP_DAEMON_URL TMUX TMUX_PANE E2E_BROKEN E2E_SCENARIO
  export LOOP_DAEMON_URL="http://127.0.0.1:9" # no Loop daemon; skills loops never use it
  export WS_HOME="$TMP/ws" WS_STATE="$TMP/state" WS_SOCK="$TMP/ws.sock" WS_TMUX_SOCKET="$TMP/tmux.sock" WS_NOTIFY=0
  export LOOP_REPO_REGISTRY="$TMP/home/repos.json" LOOP_WORKTREE_ROOT="$TMP/worktrees"
  export E2E_DIR="$TMP/e2e" E2E_ROOT="$ROOT" LOOP_JEV_MODE=off LOOP_JEV_KEY_FILE="$TMP/no-key"
  mkdir -p "$E2E_DIR/bin" "$E2E_DIR/work/.loop" "$TMP/bin"
  WS_BIN="$ROOT/wsd/target/debug/ws"
  print -r -- "#!/bin/sh
exec \"$WS_BIN\" \"\$@\"" > "$E2E_DIR/bin/ws"
  chmod +x "$E2E_DIR/bin/ws"
  printf '#!/bin/sh\nexit 99\n' > "$TMP/bin/launchctl"; chmod +x "$TMP/bin/launchctl"
  printf '#!/bin/sh\nprintf "main\\n"\n' > "$TMP/bin/gh"; chmod +x "$TMP/bin/gh"
  export PATH="$E2E_DIR/bin:$TMP/bin:$PATH"
  export WS_BIN
  make_origin || return 1
  WS_TEST_DAEMON_PID=""
  start_ws_test_daemon || return 1
  DAEMON_PID="$WS_TEST_DAEMON_PID"
  cp "$HERE/plan.md" "$E2E_DIR/work/.loop/plan.md"
  WS_LOOP_ID="$("$WS_BIN" loop create --dir "$E2E_DIR/work/.loop" --plan-id e2e-skills-plan --json | jq -er .id)" || return 1
  export WS_LOOP_ID
}

make_origin() {
  local remote="$TMP/e2e.git" checkout="$TMP/e2e-checkout"
  git init --bare -q "$remote"
  git init -q -b main "$checkout"
  git -C "$checkout" config user.email e2e@example.com
  git -C "$checkout" config user.name e2e
  print base > "$checkout/base.txt"
  git -C "$checkout" add base.txt
  git -C "$checkout" commit -qm initial
  git -C "$checkout" remote add origin "git@github.com:acme/e2e.git"
  git -C "$checkout" config url."$remote".insteadOf "git@github.com:acme/e2e.git"
  git -C "$checkout" push -q origin main
  "$ROOT/loop-repo.sh" map acme/e2e "$checkout" > /dev/null
}

# Stop only the processes of this sandbox: the daemon and the tmux server that the sandbox socket names.
teardown() {
  [[ -n "$DAEMON_PID" ]] && { kill "$DAEMON_PID" 2>/dev/null; wait "$DAEMON_PID" 2>/dev/null; }
  DAEMON_PID=""
  [[ -n "$TMP" && -S "$TMP/tmux.sock" ]] && tmux -S "$TMP/tmux.sock" kill-server 2>/dev/null
  if [[ -n "$TMP" ]]; then
    [[ -z "${E2E_KEEP:-}" ]] || print -u2 "loop-e2e: kept sandbox $TMP"
    [[ -n "${E2E_KEEP:-}" ]] || rm -rf "$TMP"
  fi
  TMP=""
}

# --- orchestrator ------------------------------------------------------------------------------------------

start_orchestrator() {
  ORCH_ID="$("$WS_BIN" session create --headless --json --inherit-env --parent "$WS_LOOP_ID" \
    --cwd "$E2E_DIR/work" --stdout "$E2E_DIR/orchestrator.out" --stderr "$E2E_DIR/orchestrator.err" -- zsh "$HERE/orchestrator.sh" | jq -er .id)"
}

run_orchestrator() {
  if [[ -n "$MODEL" ]]; then print -u2 "loop-e2e: --model runs are not wired yet"; return 1; fi
  start_orchestrator || return 1
  "$WS_BIN" wait "$ORCH_ID" --for exit > "$E2E_DIR/wait.out" 2>&1
}

wait_for_file() {
  local deadline=$(( SECONDS + HANG_GUARD ))
  until [[ -e "$1" ]]; do
    (( SECONDS < deadline )) || { print -u2 "loop-e2e: hang guard on $1"; return 1; }
    /bin/sleep 0.05
  done
}

# --- assertions --------------------------------------------------------------------------------------------

assert_parked() {
  local loop; loop="$("$WS_BIN" loop get "$WS_LOOP_ID" --json)"
  assert_eq "$(jq -r '[.gates[]? | select(.kind == "merge" and .state == "open")] | length' <<< "$loop")" 1 "the loop parks at the merge gate"
  assert_eq "$(jq -r '[.phases | to_entries[] | select(.value.kind != "review" and .value.status != "merged")] | length' <<< "$loop")" 0 "every work phase is merged"
  assert_eq "$(jq -r '.awaitingMerge.milestone' "$E2E_DIR/work/.loop/state.json")" 1 "state records awaitingMerge"
}

assert_check_clean() {
  local out; out="$("$WS_BIN" loop check "$WS_LOOP_ID" --json)"
  assert_eq "$(jq -r .hard <<< "$out")" 0 "ws loop check: no hard violation"
  assert_eq "$(jq -r '[.violations[] | select(.severity == "soft")] | length' <<< "$out")" 0 "ws loop check: no soft violation"
}

assert_lane_isolation() {
  local integ; integ="$(jq -er '.repositories["acme/e2e"].integrationWorktree' "$E2E_DIR/work/.loop/state.json")"
  local n branch
  for n in alpha beta gamma; do
    branch="feat/e2e-$n"
    assert_eq "$(git -C "$integ" worktree list --porcelain | grep -c "branch refs/heads/$branch")" 1 "branch $branch has its own worktree"
  done
  assert_eq "$(git -C "$integ" log --first-parent --format=%s | grep -vc '^Merge\|^merge\|initial')" 0 "integration HEAD changes only through merges"
}

assert_origin_has_all_lanes() {
  local clone="$TMP/origin-check"
  git clone -q --branch feat/e2e-m1 "$TMP/e2e.git" "$clone" || { TESTS_FAIL=$((TESTS_FAIL + 1)); return; }
  local f
  for f in alpha beta gamma; do
    assert_eq "$([[ -f "$clone/$f.txt" ]] && echo yes)" yes "origin integration branch has $f.txt"
  done
  assert_exit "$(cd "$clone" && sh "$HERE/verify.sh" merged; echo $?)" 0 "origin integration branch passes Verify"
}

main "$@"
