# shellcheck shell=bash
# Loop Engineering — daemon emitter. Sourced by loop-state.sh / loop-runner.sh /
# loop-merge.sh / loop-notify.sh to push best-effort lifecycle to the central Loop
# Observatory daemon (http://localhost:7717). Every function is best-effort and NEVER
# fails the caller — a down daemon must not affect a loop run. JSON bodies are built by the
# callers with `jq -cn --arg` and piped in on stdin (injection-safe; no shell interpolation).

: "${LOOP_DAEMON_URL:=http://localhost:7717}"

# loop_emit <runId> <action>   — POST the stdin JSON body to /api/loops/<runId>/<action>.
loop_emit() {
  loop_emit_run_id="$1"
  loop_emit_action="$2"
  [ -n "$loop_emit_run_id" ] || return 0
  curl -sf -m 2 -X POST --data-binary @- \
    "${LOOP_DAEMON_URL}/api/loops/${loop_emit_run_id}/${loop_emit_action}" >/dev/null 2>&1 || true
  return 0
}

# loop_ensure_daemon   — make the central daemon reachable: health probe → launchctl
# kickstart/bootstrap → nohup the installed loop-web.sh --daemon. Best-effort, returns 0.
loop_ensure_daemon() {
  curl -sf -m 2 "${LOOP_DAEMON_URL}/api/health" >/dev/null 2>&1 && return 0
  loop_emit_label="com.rstagi.loop-web"
  if command -v launchctl >/dev/null 2>&1; then
    launchctl kickstart -k "gui/$(id -u)/${loop_emit_label}" >/dev/null 2>&1 ||
      launchctl bootstrap "gui/$(id -u)" "${HOME}/Library/LaunchAgents/${loop_emit_label}.plist" >/dev/null 2>&1 || true
  fi
  loop_ensure_wait && return 0
  if [ -x "${HOME}/dotfile/loop-web.sh" ]; then
    nohup "${HOME}/dotfile/loop-web.sh" --daemon >"${HOME}/.kestral-loop-web.log" 2>&1 &
    disown 2>/dev/null || true
  fi
  loop_ensure_wait
  return 0
}

# Poll /api/health up to ~3s; return 0 as soon as it answers.
loop_ensure_wait() {
  loop_emit_i=0
  while [ "$loop_emit_i" -lt 6 ]; do
    curl -sf -m 2 "${LOOP_DAEMON_URL}/api/health" >/dev/null 2>&1 && return 0
    sleep 0.5
    loop_emit_i=$((loop_emit_i + 1))
  done
  return 1
}
