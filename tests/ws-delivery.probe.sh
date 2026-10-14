#!/bin/zsh
# Live delivery findings (2026-10-14; Claude 2.1.296, Codex 0.159.2, Grok 1.0.50):
# | Agent | a idle multiline | b during turn | c Stop block | d initial argv | Delivery |
# | --- | --- | --- | --- | --- | --- |
# | claude | pass | pass (steering) | pass | pass | idle paste + inbox drained by Stop block |
# | codex | pass | pass (steering) | pass | pass | idle paste + inbox drained by Stop block |
# | grok | pass | pass (queued) | pass | pass | idle paste + inbox drained by Stop block |
# All three accept {"decision":"block","reason":"..."} from a Stop command hook.
# Claude/Codex incorporate running Enter into the active turn; Grok queues a new
# turn. Prefer our inbox over TUI-specific running input. Fallback: next idle paste.
# Idle/running: Stop / UserPromptSubmit+PreToolUse+PostToolUse; PermissionRequest
# means waiting. Keep working when Stop emits block; guard against infinite nudges.
# Grok's Stop fields are stopHookActive/lastAssistantMessage (camelCase).
# a = idle multiline paste; b = paste during a turn; c = Stop continuation;
# d = positional initial prompt. This manual probe is excluded from *.test.sh.
# Usage: zsh tests/ws-delivery.probe.sh [--output-dir DIR] [claude|codex|grok ...]
# Costs real agent tokens. Uses existing auth, per-run hooks, a private tmux server,
# and bounded waits (whole run <= 540s). Logs can contain local paths/transcripts.
set -u -o pipefail
umask 077

ROOT="${0:A:h:h}"
OUT=""
SOCKET="ws-delivery-$$-$RANDOM"
DEADLINE=$(( $(date +%s) + 540 ))
WAIT=40
AGENTS=()
PANE="" AGENT_DIR=""
typeset -A RESULTS NOTES

main() {
  while (( $# )); do
    case "$1" in
    --output-dir) [[ $# -ge 2 ]] || return 2; OUT="$2"; shift 2 ;;
    claude|codex|grok) AGENTS+=("$1"); shift ;;
    *) print -u2 -- "usage: $0 [--output-dir DIR] [claude|codex|grok ...]"; return 2 ;;
    esac
  done
  (( ${#AGENTS} )) || AGENTS=(claude codex grok)
  local required agent
  for required in tmux jq rg; do
    command -v "$required" >/dev/null || { print -u2 -- "missing $required"; return 1; }
  done
  if [[ -n "$OUT" ]]; then
    mkdir -p "$OUT" || return 1
    OUT="$(mktemp -d "$OUT/run.XXXXXX")" || return 1
  else
    OUT="$(mktemp -d "$ROOT/.ws-delivery.XXXXXX")" || return 1
  fi
  OUT="${OUT:A}"
  print -r -- "$SOCKET" > "$OUT/tmux-socket.txt"
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM HUP
  T -f /dev/null new-session -d -s probe -x 220 -y 60 || return 1
  T set -g history-limit 20000
  T set -g default-size 220x60
  for agent in "${AGENTS[@]}"; do
    probe_agent "$agent"
  done
  report > "$OUT/results.md"
  cat "$OUT/results.md"
  print -r -- "Evidence: $OUT"
}

probe_agent() {
  local agent="$1" check before submits prompt initial busy_before kind
  for check in a b c d; do RESULTS[$agent.$check]=unverified; done
  NOTES[$agent]="not reached before deadline"
  (( $(date +%s) < DEADLINE )) || return
  if ! command -v "$agent" >/dev/null; then
    NOTES[$agent]="absent; skipped"
    return
  fi
  AGENT_DIR="$OUT/$agent"
  mkdir -p "$AGENT_DIR/events"
  "$agent" --version > "$AGENT_DIR/version.txt" 2>&1
  write_hooks "$agent"
  initial="Reply with only the concatenation of INIT and IAL. Do not use tools."
  launch_agent "$agent" "$initial" || { NOTES[$agent]="launch failed"; return; }
  print -r -- "Probing $agent ($PANE)"
  if ! wait_for startup; then
    capture startup
    NOTES[$agent]="no initial reply within ${WAIT}s; inspect startup.txt for auth/trust/onboarding/errors"
    T kill-window -t "$PANE" 2>/dev/null
    return
  fi
  RESULTS[$agent.d]=pass
  # A Stop marker is authoritative; a reply alone may still be streaming.
  if ! wait_for idle; then
    NOTES[$agent]="initial reply observed but Stop hook did not signal idle"
    capture no-idle
    T kill-window -t "$PANE" 2>/dev/null
    return
  fi
  NOTES[$agent]=""

  before="$(event_count UserPromptSubmit)"
  prompt=$'Reply with only the concatenation of MULTI and LINE.\nTreat both lines as one prompt. Do not use tools.'
  print -rn -- "$prompt" > "$AGENT_DIR/idle-prompt.txt"
  paste_prompt "$prompt"
  if wait_for reply MULTILINE && wait_for idle; then
    submits=$(( $(event_count UserPromptSubmit) - before ))
    if (( submits == 1 )) && submitted_exact "$prompt"; then
      RESULTS[$agent.a]=pass
    else
      RESULTS[$agent.a]=unverified
      NOTES[$agent]+="idle reply seen; prompt hook count=$submits or payload differs. "
    fi
  else
    RESULTS[$agent.a]=fail
    NOTES[$agent]+="idle paste did not complete within ${WAIT}s. "
  fi
  capture idle-paste

  # Arm the one-shot continuation only after the previous turn has stopped.
  if is_idle; then
    print -r -- 'Reply with only the concatenation of HOOK and RESUMED. Do not use tools.' > "$AGENT_DIR/arm"
    paste_prompt 'Reply with only the concatenation of HOOK and READY. Do not use tools.'
    if wait_for reply HOOKRESUMED && wait_for idle; then
      [[ -f "$AGENT_DIR/block-emitted" ]] && RESULTS[$agent.c]=pass
    elif [[ -f "$AGENT_DIR/block-emitted" ]]; then
      RESULTS[$agent.c]=fail
      NOTES[$agent]+="Stop block emitted, no continuation reply. "
    else
      NOTES[$agent]+="Stop block not emitted. "
    fi
    rm -f "$AGENT_DIR/arm"
    capture stop-continuation
  fi

  if is_idle; then
    # The marker proves a real tool is executing; UserPromptSubmit alone cannot.
    busy_before="$(wc -l < "$AGENT_DIR/order.log")"
    prompt="Run this shell command exactly: touch ${(q)AGENT_DIR}/busy; sleep 8; touch ${(q)AGENT_DIR}/busy-finished. Then reply with only the concatenation of BUSY and FINISHED."
    paste_prompt "$prompt"
    if wait_for test -f "$AGENT_DIR/busy" && [[ ! -f "$AGENT_DIR/busy-finished" ]]; then
      capture running-before-paste
      paste_prompt 'Reply with only the concatenation of QUEUE and RECEIVED. Do not use tools.'
      if wait_for reply QUEUERECEIVED && wait_for idle; then
        kind="$(delivery_kind "$busy_before")"
        RESULTS[$agent.b]="pass ($kind)"
        NOTES[$agent]+="running paste delivered automatically ($kind). "
      elif [[ -f "$AGENT_DIR/busy-finished" ]] && is_idle; then
        RESULTS[$agent.b]=fail
        NOTES[$agent]+="running paste not delivered automatically (lost or left in composer). "
      else
        NOTES[$agent]+="running paste inconclusive within timeout. "
      fi
    else
      NOTES[$agent]+="could not establish running tool window. "
    fi
    capture running-paste
  fi
  T kill-window -t "$PANE" 2>/dev/null
}

launch_agent() {
  local agent="$1" initial="$2" ev command hook
  local -a argv
  case "$agent" in
  claude)
    argv=(claude --dangerously-skip-permissions --setting-sources ''
      --settings "$AGENT_DIR/settings.json" --strict-mcp-config --mcp-config '{"mcpServers":{}}'
      --effort low --system-prompt 'Follow the user request exactly. Keep replies minimal.' "$initial")
    ;;
  codex)
    argv=(codex --no-daemon --no-alt-screen --dangerously-bypass-approvals-and-sandbox
      --dangerously-bypass-hook-trust -c 'model_reasoning_effort="low"')
    for ev in UserPromptSubmit PreToolUse PostToolUse Stop PermissionRequest Interrupt; do
      hook="/bin/zsh ${(q)AGENT_DIR}/hook.sh ${(q)AGENT_DIR} $ev"
      command="$(jq -rn --arg c "$hook" '$c | @json')"
      argv+=(-c "hooks.$ev=[{hooks=[{type=\"command\",command=$command}]}]")
    done
    argv+=("$initial")
    ;;
  grok)
    mkdir -p "$AGENT_DIR/grok-home/hooks"
    # Copy auth only into an owner-private directory; remove it on every exit.
    [[ ! -f "${GROK_HOME:-$HOME/.grok}/auth.json" ]] ||
      cp "${GROK_HOME:-$HOME/.grok}/auth.json" "$AGENT_DIR/grok-home/auth.json"
    cp "$AGENT_DIR/settings.json" "$AGENT_DIR/grok-home/hooks/probe.json"
    print -r -- $'[cli]\nuse_leader = false\n[folder_trust]\nenabled = false\n[compat.claude]\nhooks = false\n[compat.codex]\nhooks = false\n[compat.cursor]\nhooks = false' > "$AGENT_DIR/grok-home/config.toml"
    argv=(env "GROK_HOME=$AGENT_DIR/grok-home" grok --always-approve
      --fullscreen --no-subagents --disable-web-search --no-plan --reasoning-effort low "$initial")
    ;;
  esac
  # Record shell-quoted argv, including empty args, for exact reproduction.
  print -r -- "${(j: :)${(@q)argv}}" > "$AGENT_DIR/argv.txt"
  {
    print -r -- '#!/bin/zsh'
    print -r -- 'unset CLAUDECODE WS_WORKSPACE WS_SESSION CODEX_THREAD_ID'
    print -r -- "${(j: :)${(@q)argv}}"
    print -r -- 'print -r -- "PROBE_AGENT_EXIT=$?"; sleep 2'
  } > "$AGENT_DIR/launch.sh"
  PANE="$(T new-window -d -P -F '#{pane_id}' -t probe: -n "$agent" -c "$ROOT" \
    /bin/zsh -f "$AGENT_DIR/launch.sh")"
  [[ "$PANE" == %<-> ]] || return 1
  print -r -- "$PANE" > "$AGENT_DIR/pane.txt"
}

write_hooks() {
  cat > "$AGENT_DIR/hook.sh" <<'HOOK'
#!/bin/zsh
set -u
dir="$1" event="$2"
umask 077
payload="$(cat)"
file="$(mktemp "$dir/events/$event.XXXXXXXX")"
print -r -- "$payload" > "$file"
print -r -- "$event ${file:t}" >> "$dir/order.log"
case "$event" in
UserPromptSubmit|PreToolUse|PostToolUse) print working > "$dir/state" ;;
PermissionRequest) print waiting > "$dir/state" ;;
Stop|Interrupt|StopCancelled)
  if [[ "$event" == Stop && -f "$dir/arm" ]] &&
      [[ "$(print -r -- "$payload" | jq -r '.stop_hook_active // .stopHookActive // false')" != true ]]; then
    # Rename first: the hook can never emit this continuation twice.
    if mv "$dir/arm" "$dir/block-emitted" 2>/dev/null; then
      print working > "$dir/state"
      jq -n --arg reason "$(cat "$dir/block-emitted")" '{decision:"block",reason:$reason}'
      exit 0
    fi
  fi
  print idle > "$dir/state"
  ;;
esac
# Codex Stop requires JSON even when allowing stop; accepted by the other agents.
print '{}'
HOOK
  jq -n --arg d "$AGENT_DIR" --arg agent "$1" '
    def run($e): [{hooks:[{type:"command", timeout:5,
      command:("/bin/zsh " + ($d + "/hook.sh" | @sh) + " " + ($d | @sh) + " " + $e)}]}];
    {hooks:{UserPromptSubmit:run("UserPromptSubmit"),PreToolUse:run("PreToolUse"),
      PostToolUse:run("PostToolUse"),Stop:run("Stop"),PermissionRequest:run("PermissionRequest")}}
    | if $agent == "grok" then .hooks.StopCancelled=run("StopCancelled") else . end' > "$AGENT_DIR/settings.json"
}

paste_prompt() {
  print -rn -- "$1" | T load-buffer -b delivery -
  T paste-buffer -d -p -b delivery -t "$PANE"
  # Separate the Enter from the paste event; don't race TUI bracketed-paste parsing.
  sleep 0.3
  T send-keys -t "$PANE" Enter
}

wait_for() {
  local until=$(( $(date +%s) + WAIT ))
  (( until > DEADLINE )) && until=$DEADLINE
  while (( $(date +%s) < until )); do
    case "$1" in
    startup) acknowledge_notice; has_reply INITIAL && return 0 ;;
    reply) has_reply "$2" && return 0 ;;
    idle) is_idle && return 0 ;;
    *) "$@" && return 0 ;;
    esac
    [[ "$(T display -p -t "$PANE" '#{pane_dead}' 2>/dev/null)" == 0 ]] || return 1
    sleep 0.25
  done
  return 1
}

acknowledge_notice() {
  # This informational acknowledgement is confined to the temporary Grok home.
  # Login, unknown onboarding, and trust/permission dialogs remain unverified.
  [[ -f "$AGENT_DIR/notice-acknowledged.txt" ]] && return
  local screen="$(T capture-pane -p -t "$PANE" 2>/dev/null)"
  if [[ "$screen" == *'Updates to our Enterprise Terms of Service and Acceptable Use Policy'* &&
      "$screen" == *'Got it'* && "$screen" == *'Quit'* ]]; then
    print -r -- "$screen" > "$AGENT_DIR/notice-acknowledged.txt"
    T send-keys -t "$PANE" a
  fi
}

has_reply() {
  local token="$1" file
  # The combined token never occurs in submitted text, so pane echo can't pass.
  for file in "$AGENT_DIR"/events/Stop.*(N); do
    jq -e --arg t "$token" '(.last_assistant_message // .lastAssistantMessage)? | strings | contains($t)' "$file" >/dev/null 2>&1 && return 0
  done
  T capture-pane -p -S - -t "$PANE" 2>/dev/null | rg -q -- "$token"
}

is_idle() { [[ -f "$AGENT_DIR/state" && "$(cat "$AGENT_DIR/state")" == idle ]]; }

event_count() {
  local -a files=("$AGENT_DIR"/events/"$1".*(N))
  print ${#files}
}

submitted_exact() {
  local file
  for file in "$AGENT_DIR"/events/UserPromptSubmit.*(N); do
    jq -e --arg p "$1" '.prompt == $p' "$file" >/dev/null 2>&1 && return 0
  done
  return 1
}

delivery_kind() {
  local file
  for file in "$AGENT_DIR"/events/UserPromptSubmit.*(N); do
    if jq -e '.prompt == "Reply with only the concatenation of QUEUE and RECEIVED. Do not use tools."' "$file" >/dev/null 2>&1; then
      awk -v start="$1" -v file="${file:t}" '
        NR <= start {next}
        $1 == "Stop" {stopped=1}
        $2 == file {print stopped ? "queued" : "steering"; found=1; exit}
        END {if (!found) print "delivery observed; hook order unknown"}' "$AGENT_DIR/order.log"
      return
    fi
  done
  print 'delivery observed; prompt hook missing'
}

capture() { T capture-pane -p -S - -t "$PANE" > "$AGENT_DIR/$1.txt" 2>/dev/null; }

report() {
  local agent chosen
  print '| Agent | a idle multiline | b during turn | c Stop block | d initial argv | Delivery |'
  print '| --- | --- | --- | --- | --- | --- |'
  for agent in "${AGENTS[@]}"; do
    chosen='paste on next idle'
    [[ "${RESULTS[$agent.c]}" == pass ]] && chosen='idle paste + inbox drained by Stop block'
    print -r -- "| $agent | ${RESULTS[$agent.a]} | ${RESULTS[$agent.b]} | ${RESULTS[$agent.c]} | ${RESULTS[$agent.d]} | $chosen |"
  done
  print
  for agent in "${AGENTS[@]}"; do print -r -- "- $agent: ${NOTES[$agent]}"; done
}

cleanup() {
  T kill-server 2>/dev/null || true
  local agent
  for agent in "${AGENTS[@]}"; do
    rm -f "$OUT/$agent/grok-home/auth.json" "$OUT/$agent/grok-home/auth.json.lock"
  done
}

T() { tmux -L "$SOCKET" "$@"; }

main "$@"
