#!/bin/zsh
# Sandbox HOME + agent config dirs for test suites and guard the real agent config files.
# Source it, then: home_guard_sandbox <dir>; snap="$(home_guard_snapshot)"; …;
# home_guard_check "$snap" || fail. TESTS_REAL_HOME survives nested sandboxes (run.sh → suite).

# Real-HOME files tests must never touch: agent hooks/config/settings (small, no session dirs).
HOME_GUARD_PATHS=(
  .grok/hooks .grok/settings.json .grok/config.toml
  .codex/config.toml .codex/hooks.json .codex/hooks
  .claude/settings.json .claude/settings.local.json .claude/hooks
  .ws/claude-settings.json
)

# home_guard_sandbox <dir> — points HOME and every agent config root at <dir>.
home_guard_sandbox() {
  export TESTS_REAL_HOME="${TESTS_REAL_HOME:-$HOME}"
  mkdir -p "$1"
  export HOME="$1"
  export GROK_HOME="$1/.grok" CODEX_HOME="$1/.codex" CLAUDE_CONFIG_DIR="$1/.claude"
  export XDG_CONFIG_HOME="$1/.config"
  unset GROK_HOOKS
  [[ -f "$1/.gitconfig" ]] || printf '[user]\n\tname = Test\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n' > "$1/.gitconfig"
}

# home_guard_snapshot — checksum listing of guarded real-HOME files (missing paths tolerated),
# plus the ws bin/ws link target (startup migration can re-point it).
home_guard_snapshot() {
  local root="${TESTS_REAL_HOME:-$HOME}" p
  {
    print -r -- "link .ws/bin/ws -> $(readlink "$root/.ws/bin/ws" 2>/dev/null)"
    for p in $HOME_GUARD_PATHS; do
      [[ -e "$root/$p" ]] || continue
      find "$root/$p" -type f -exec shasum {} + 2>/dev/null
    done
  } | LC_ALL=C sort
}

# home_guard_check <snapshot> — non-zero (and a diff on stderr) if guarded files changed.
home_guard_check() {
  local now
  now="$(home_guard_snapshot)"
  [[ "$now" == "$1" ]] && return 0
  print -u2 "FAIL: tests modified real agent config under ${TESTS_REAL_HOME:-$HOME}:"
  diff <(print -r -- "$1") <(print -r -- "$now") >&2
  return 1
}
