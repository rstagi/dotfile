#!/bin/zsh
set -u -o pipefail
zmodload zsh/system # zsystem flock (sessions file)

# ws — bare-minimum terminal Conductor: one git worktree + one tmux session per workspace,
# one tab (window) per agent session inside it. Status and resume ids come from Claude Code /
# Codex / Grok Build hooks — no screen scraping.
# Every launched/resumed agent gets its global identity (spawned ones: a reminder to run
# `ws done` when the task is complete); ws-sessions teaches messaging and dependent tasks.
#
#   ws                         picker (attaches when run outside tmux)
#   ws new [--repo P] [--branch B] [--agent claude|codex|grok] [--detach]
#   ws add [name|path] [--agent claude|codex|grok] [--detach]   another agent tab (default: current worktree)
#   ws open <workspace|path|id|name> [--detach]   focus or resume a workspace/session
#   ws pick                    grouped picker: enter open · ctrl-n new · ctrl-o new codex ·
#                              ctrl-r new grok · ctrl-a add claude tab · ctrl-t add codex tab ·
#                              ctrl-v add grok tab · ctrl-p pin · ctrl-x rm ·
#                              ctrl-s show/hide sessions · ctrl-u toggle unread (• = finished, not yet seen)
#   ws pin <name|path>         toggle pin (pinned workspaces are listed first)
#   ws list                    TSV: state name path agent window pinned active unread
#   ws whoami                  own global session id (inside an agent tab)
#   ws sessions [id|name]      TSV: id name workspace agent state status
#   ws name <label>            name your session (unique; usable in place of its id)
#   ws send [--force] <id|name> <msg|->   sender-prefixed message (or stdin): paste idle, queue busy
#                              done needs --force; stopped sessions resume before delivery
#                              idle paste awaits prompt confirmation; timeout requeues (WS_PASTE_ACK_TIMEOUT=10)
#   ws done [--output <file>|-]   declare own session done; replace output.md (default empty)
#                              --output - also reads stdin; repeat appends another done event
#                              pending inbox/in-flight messages refuse done; let Stop deliver first
#                              spawned tabs close after publication; user-opened tabs stay
#   ws show <id|name>           session metadata, lifecycle status and output path
#   ws on <id[,id…]> <done|exit> (--send-to <id|me> <msg> | --spawn <prompt> [--agent A])
#                              one-shot fan-in; spawn defaults to claude in current worktree
#                              done counts past events; exit counts only events after registration
#                              output-only templates reject registration; include text or {id}
#   ws triggers                TSV: pending trigger id, sources, event, action, target, template
#   ws on --rm <trigger-id>     cancel a pending trigger; cycles are rejected on registration
#   ws rm <name|path> [--force] [--delete-branch]
#   ws merge                   in a workspace: confirm, merge its PR (gh, $WS_MERGE_METHOD,
#                              default squash), fast-forward the main checkout, then on confirm
#                              remove worktree + tabs + local/remote branch (prefix+M); if
#                              dirty, lists the files and asks to force delete
#   ws claude|codex|grok [args]  run the agent in this tab as a tracked session (e.g. a prefix+c tab)
#   ws hook <event>            called by agent hooks inside a session window
#                              (working|idle|stop|cancel|waiting|subagent-start|subagent-stop)
#                              cancel requires the current turn id; unknown/stale ids are ignored
#   ws _dismiss <window|pane>  tab seen or agent exited: emit `closed` if a notification is open
#   ws _migrate-hooks          re-point claude-settings.json / Grok hooks / bin link / tmux bindings at this ws (install.sh)
#
# Workspace slots live in <worktree git dir>/ws-sessions (slot, agent, resume id, global id).
# Global identities live in $WS_HOME/sessions/<id>/. A killed tab resumes on `ws open`;
# a clean exit forgets its workspace slot while retaining its global record.
# Exited tabs stop accepting messages before exit triggers run; sends resume the session.

WS_BIN="${0:A}"
WS_ROOT="${WS_ROOT:-$HOME/.ws/worktrees}"
WS_HOME="${WS_HOME:-$HOME/.ws}"
WS_NOTIFY="${WS_NOTIFY:-1}"
WS_NOTIFIER="${WS_NOTIFIER:-auto}" # auto | osascript | terminal-notifier | boringnotch | executable path
WS_BORINGNOTCH_SOCKET="${WS_BORINGNOTCH_SOCKET:-$HOME/Library/Application Support/boringNotch/notify.sock}"
# Notification sound (file path or macOS sound name) + icon per state.
WS_NOTIFY_SOUND_DONE="${WS_NOTIFY_SOUND_DONE:-${WS_BIN:h:h}/assets/ws/sounds/done.wav}"
WS_NOTIFY_SOUND_WAITING="${WS_NOTIFY_SOUND_WAITING:-${WS_BIN:h:h}/assets/ws/sounds/waiting.wav}"
WS_NOTIFY_ICON_DONE="${WS_NOTIFY_ICON_DONE:-${WS_BIN:h:h}/assets/ws/done.png}"
WS_NOTIFY_ICON_WAITING="${WS_NOTIFY_ICON_WAITING:-${WS_BIN:h:h}/assets/ws/waiting.png}"
# Branded copy of terminal-notifier (macOS takes a notification's icon from the sending app).
WS_NOTIFIER_APP="$WS_HOME/ws.app"
WS_NOTIFIER_SOURCE="${WS_NOTIFIER_SOURCE:-/opt/homebrew/opt/terminal-notifier/terminal-notifier.app}"
WS_APP_ICON="${WS_BIN:h:h}/assets/ws/app.png"
WS_AGENT_SHELL="${WS_AGENT_SHELL:-zsh -ic}" # interactive: agents need .zshrc (secrets, PATH)
REGISTRY="${LOOP_REPO_REGISTRY:-$HOME/.loop/repos.json}"
WS_REPO_ROOTS="${WS_REPO_ROOTS:-$HOME/Dev $HOME/dotfile}" # scanned for repos by `ws new`
PINS="$WS_HOME/pins"
SHOW_SESSIONS="$WS_HOME/show-sessions" # exists → picker lists per-session rows (ctrl-s)
DEFAULT_INCLUDE_GLOBS=(".env*")
AGENTS=(claude codex grok)
GROK_HOOKS="${GROK_HOME:-$HOME/.grok}/hooks/ws.json"
source "${WS_BIN:h}/lib/core.sh"
source "${WS_BIN:h}/lib/workspace.sh"
source "${WS_BIN:h}/lib/tabs.sh"
source "${WS_BIN:h}/lib/hooks.sh"
source "${WS_BIN:h}/lib/notify.sh"
source "${WS_BIN:h}/lib/picker.sh"
source "${WS_BIN:h}/lib/bus.sh"
source "${WS_BIN:h}/lib/triggers.sh"

main() {
  local cmd="${1:-}"
  [[ $# -gt 0 ]] && shift
  case "$cmd" in "" | new | add | open | pick) hooks_migrate ;; esac # cheap no-op unless the ws these name is gone
  case "$cmd" in
  "") cmd_attach ;;
  new) cmd_new "$@" ;;
  add) cmd_add "$@" ;;
  open) cmd_open "$@" ;;
  pick) cmd_pick ;;
  pin) cmd_pin "$@" ;;
  list | ls) cmd_list ;;
  whoami) cmd_whoami ;;
  sessions) cmd_sessions "$@" ;;
  name) cmd_name "$@" ;;
  send) cmd_send "$@" ;;
  done) cmd_done "$@" ;;
  on) cmd_on "$@" ;;
  triggers) cmd_triggers "$@" ;;
  show) cmd_show "$@" ;;
  rm) cmd_rm "$@" ;;
  merge) cmd_merge ;;
  _merge_popup) (cmd_merge); print -n "\npress enter to close"; read -r _ ;; # subshell: die must not skip the pause
  claude | codex | grok) cmd_run "$cmd" "$@" ;;
  hook) cmd_hook "$@" ;;
  _dismiss) cmd_dismiss "$@" ;;
  _rows) picker_rows ;;
  _summary) picker_summary ;;
  _toggle_sessions) toggle_sessions ;;
  _unread) toggle_unread "$@" ;;
  _preview) preview "$@" ;;
  _end) session_remove "$@" ;;
  _agent) ws_bus_run_agent "$@" ;;
  _paste-timeout) ws_bus_paste_timeout "$@" ;;
  _evaluate) ws_bus_evaluate_event "$@" ;;
  _deliver-retry) ws_bus_deliver_retry "$@" ;;
  _hook-retry) hook_retry "$@" ;;
  _migrate-hooks) hooks_migrate --force ;;
  _build-notifier) build_notifier ;;
  -h | --help | help) sed -n '/^# ws —/,/^# Workspace slots/{ /^# Workspace slots/d; p; }' "$WS_BIN" | sed 's/^# \{0,1\}//' ;;
  *) die "unknown command: $cmd (try: ws help)" ;;
  esac
}

[[ "$ZSH_EVAL_CONTEXT" == *:file ]] || main "$@" # sourced (tests): functions only
