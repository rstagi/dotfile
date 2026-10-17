#!/bin/zsh
# Run from the Rust picker test. The caller owns the sandbox daemon and tmux server.
set -eu -o pipefail
: ${WS_RUST_BIN:?} ${WS_SOCK:?} ${WS_STATE:?} ${WS_TMUX_SOCKET:?}
[[ "$HOME" == /tmp/* && "$WS_SOCK" == "$HOME"/* && "$WS_STATE" == "$HOME"/* ]] || exit 2
WS="$WS_RUST_BIN"
mkdir -p "$WS_ROOT"
export WS_ROOT="$(cd "$WS_ROOT" && pwd -P)"
T() { command tmux -L "$WS_TMUX_SOCKET" "$@"; }
check_eq() { [[ "$1" == "$2" ]] || { print -u2 "Expected $2. Received $1. Check: $3"; exit 1; }; }

# The fake replaces terminal input and client attachment. Data commands remain Rust commands.
real_tmux="$(command -v tmux)"
mkdir -p "$HOME/fake-agents"
cat > "$HOME/fake-agents/tmux" <<EOF_TMUX
#!/bin/sh
case "\$3" in
  attach-session|switch-client) printf '%s\\n' "\$@" > "\$HOME/focus"; exit 0 ;;
esac
exec ${(q)real_tmux} "\$@"
EOF_TMUX
chmod +x "$HOME/fake-agents/tmux"
cat > "$HOME/fake-agents/fzf" <<'EOF_FZF'
#!/bin/zsh
set -eu
cat > "$HOME/fzf-rows"
printf '%s\n' "$@" > "$HOME/fzf-args"
for argument in "$@"; do
  if [[ "$argument" == 'load:transform-header('* ]]; then
    callback="${argument#'load:transform-header('}"
    eval "${callback%')'}" > "$HOME/fzf-summary"
  fi
  if [[ "${WS_TEST_CALLBACK:-}" == sessions && "$argument" == 'ctrl-s:execute-silent('* ]]; then
    callback="${argument#'ctrl-s:execute-silent('}"
    eval "${callback%%\)+reload\(*}"
    callback="${argument##*'+reload('}"
    eval "${callback%')'}" > "$HOME/fzf-reloaded"
  fi
done
[[ "${WS_TEST_SELECT:-}" == 1 ]] || exit 130
printf '%s\n' "${WS_TEST_KEY:-}"
awk -F '\t' -v p="$WS_TEST_WORKSPACE" -v w="${WS_TEST_WINDOW:-}" \
  '$2 == p && (w == "" || $3 == w) { print; exit }' "$HOME/fzf-rows"
EOF_FZF
chmod +x "$HOME/fake-agents/fzf"

git init -q -b main "$HOME/api"
git -C "$HOME/api" -c user.name=Test -c user.email=test@example.com commit -qm initial --allow-empty
"$WS" new --repo "$HOME/api" --branch alpha --detach
wt="$WS_ROOT/api/alpha"
git -C "$HOME/api" worktree add -qb orphan "$WS_ROOT/api/orphan"
"$WS" pin api/orphan
rows="$("$WS" _rows)"
check_eq "$(print -r -- "$rows" | wc -l | tr -d ' ')" 2 'one row per workspace'
check_eq "$(print -r -- "$rows" | head -1 | cut -f2)" "$WS_ROOT/api/orphan" 'pinned workspace first'
"$WS" pick
check_eq "$(cat "$HOME/fzf-rows")" "$rows" 'fzf receives Rust rows'
check_eq "$(cat "$HOME/fzf-summary")" "$("$WS" _summary)" 'fzf header uses Rust summary'

# These checks use the same first-row shortcuts as picker.test.sh.
export WS_TEST_SELECT=1 WS_TEST_WORKSPACE="$WS_ROOT/api/orphan"
for key in ctrl-a ctrl-t ctrl-v; do
  WS_TEST_KEY="$key" TMUX=fake "$WS" pick
done
for agent in claude codex grok; do
  check_eq "$("$WS" list | awk -F '\t' -v p="$WS_TEST_WORKSPACE" -v a="$agent" '$3 == p && $4 == a { n++ } END { print n+0 }')" 1 "picker adds $agent"
done
unset WS_TEST_SELECT
WS_TEST_CALLBACK=sessions "$WS" pick
check_eq "$(grep -cF "$WS_TEST_WORKSPACE" "$HOME/fzf-reloaded")" 4 'Rust reload expands session rows'
"$WS" _toggle_sessions
check_eq "$("$WS" _rows | grep -cF "$WS_TEST_WORKSPACE")" 1 'second toggle collapses rows'

"$WS" add api/alpha --agent codex --detach
"$WS" _toggle_sessions
window="$("$WS" list | awk -F '\t' -v p="$wt" '$3 == p && $4 == "codex" {print $5}')"
pane="$(T display -p -t "$window" '#{pane_id}')"
TMUX=fake TMUX_PANE="$pane" "$WS" pick
position="$(sed -n 's/^start:.*pos(\([0-9]*\)).*/\1/p' "$HOME/fzf-args")"
check_eq "$(sed -n "${position}p" "$HOME/fzf-rows" | cut -f3)" "$window" 'cursor selects current tab'
"$WS" _unread "$window"
check_eq "$(T show -wqv -t "$window" @ws_unread)" 1 'Rust command marks the tab unread'
WS_TEST_SELECT=1 WS_TEST_WORKSPACE="$wt" WS_TEST_WINDOW="$window" TMUX=fake "$WS" pick
check_eq "$(T display -p -t "$window" '#{window_active}')" 1 'Enter focuses the selected tab'
check_eq "$(T show -wqv -t "$window" @ws_unread)" '' 'focus clears unread'

# Execute the installed tmux binding through its parser, with its target pane.
# The workspace path contains a quote to check shell argument handling.
"$WS" new --repo "$HOME/api" --branch "quoted'branch" --detach
quoted="$WS_ROOT/api/quoted'branch"
window="$("$WS" list | awk -F '\t' -v p="$quoted" '$3 == p {print $5}')"
binding="$(T list-keys -T prefix A)"
print -r -- "${binding#* A }" > "$HOME/binding.tmux"
T source-file -t "$window" "$HOME/binding.tmux"
check_eq "$("$WS" list | awk -F '\t' -v p="$quoted" '$3 == p && $4 == "codex" {n++} END {print n+0}')" 1 'tmux binding runs Rust add'
print 'Rust picker shell checks passed.'
