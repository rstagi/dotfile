#!/bin/zsh
# Exercise the embedded picker with a private tmux server and fake terminal input.
set -eu -o pipefail
ROOT="${0:A:h:h:h}"
source "$ROOT/tests/lib.sh"
TMP="$(mktemp -d /tmp/ws-picker.XXXXXX)"
export HOME="$TMP/home" WS_HOME="$TMP/home/.ws" WS_STATE="$TMP/state"
export WS_SOCK="$TMP/wsd.sock" WS_TMUX_SOCKET="$TMP/tmux.sock" WS_ROOT="$TMP/worktrees"
export WS_PICKER_RUST=1 WS_BIN="$TMP/bin/ws command" WS_NOTIFY=0
unset TMUX TMUX_PANE WS_SESSION WS_SESSION_ID
mkdir -p "$HOME" "$TMP/bin"
child_pid=""
cleanup() {
  trap '' INT TERM HUP
  [[ -z "$child_pid" ]] || kill -TERM "$child_pid" 2>/dev/null || true
  [[ -z "$child_pid" ]] || wait "$child_pid" 2>/dev/null || true
  tmux -S "$WS_TMUX_SOCKET" kill-server 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

cat > "$WS_BIN" <<'EOF_WS'
#!/bin/zsh
case "$1" in
  _rows) cat "$HOME/rows" ;;
  _summary) print 'Picker summary' ;;
  *) printf '%s\n' "$@" > "$HOME/action" ;;
esac
EOF_WS
cat > "$TMP/bin/fzf" <<'EOF_FZF'
#!/bin/zsh
cat > "$HOME/input"
printf '%s\n' "$@" > "$HOME/arguments"
[[ "${PICKER_CANCEL:-0}" == 0 ]] || exit 130
printf '%s\n' "${PICKER_KEY:-}" "${PICKER_ROW:-}"
EOF_FZF
chmod +x "$WS_BIN" "$TMP/bin/fzf"
export PATH="$TMP/bin:$PATH"
tmux -S "$WS_TMUX_SOCKET" -f /dev/null new-session -d -s picker '/bin/sleep 300'
window="$(tmux -S "$WS_TMUX_SOCKET" display -p -t picker '#{window_id}')"
print -r -- $'Workspaces\nWorkspace\t/tmp/a workspace\t'"$window" > "$HOME/rows"
export PICKER_ROW=$'Workspace\t/tmp/a workspace\t'"$window"
PICKER="$ROOT/wsd/crates/ws/src/cmd/picker.zsh"
run_picker() {
  zsh "$PICKER" "$@" &
  child_pid=$!
  wait "$child_pid"
  local result=$?
  child_pid=""
  return "$result"
}
run_picker pick
assert_eq "$(cat "$HOME/input")" "$(cat "$HOME/rows")" 'the picker passes rows to fzf'
assert_eq "$(cat "$HOME/action")" $'_pick_focus\n'"$window" 'Enter focuses the Rust session'
assert_contains "$(cat "$HOME/arguments")" '_summary' 'the picker loads its summary through ws'
PICKER_KEY=ctrl-t run_picker pick
assert_eq "$(cat "$HOME/action")" $'add\n/tmp/a workspace\n--agent\ncodex' 'Ctrl-T adds Codex to the selected workspace'
PICKER_KEY=ctrl-n run_picker pick
assert_eq "$(cat "$HOME/action")" $'new\n--scratch' 'Ctrl-N starts a Claude scratch session'
PICKER_KEY=ctrl-r run_picker pick
assert_eq "$(cat "$HOME/action")" $'new\n--scratch\n--agent\ngrok' 'Ctrl-R starts a Grok scratch session'
PICKER_KEY=alt-n run_picker pick
assert_eq "$(cat "$HOME/action")" 'new' 'Alt-N creates a Claude workspace'
PICKER_KEY=alt-o run_picker pick
assert_eq "$(cat "$HOME/action")" $'new\n--agent\ncodex' 'Alt-O creates a Codex workspace'
PICKER_CANCEL=1 run_picker pick
assert_exit "$?" 0 'cancel closes the picker without an error'

# Parse the actual bindings with tmux, without LANG or LC_* variables.
tmux -S "$WS_TMUX_SOCKET" set-hook -g 'session-window-changed[0]' 'display-message preserved'
env -i HOME="$HOME" PATH="$PATH" WS_BIN="$WS_BIN" WS_HOME="$WS_HOME" \
  WS_STATE="$WS_STATE" WS_SOCK="$WS_SOCK" WS_ROOT="$WS_ROOT" WS_NOTIFY=0 \
  WS_TMUX_SOCKET="$WS_TMUX_SOCKET" WS_PICKER_RUST=1 zsh "$PICKER" bindings
for key in w N M a A G; do
  binding="$(tmux -S "$WS_TMUX_SOCKET" list-keys -T prefix "$key")"
  assert_contains "$binding" "WS_TMUX_SOCKET=" "tmux parses the $key binding"
done
assert_contains "$(tmux -S "$WS_TMUX_SOCKET" show-hooks -g session-window-changed)" \
  'display-message preserved' 'picker bindings preserve user hooks'
tmux -S "$WS_TMUX_SOCKET" set-option -t picker @ws_path '/tmp/a workspace'
rm -f "$HOME/action"
binding="$(tmux -S "$WS_TMUX_SOCKET" list-keys -T prefix A)"
print -r -- "${binding#* A }" > "$TMP/binding.tmux"
tmux -S "$WS_TMUX_SOCKET" source-file -t "$window" "$TMP/binding.tmux"
deadline=$(( SECONDS + 30 ))
until [[ -s "$HOME/action" ]]; do
  (( SECONDS < deadline )) || { print -u2 'The binding did not run.'; exit 1; }
  sleep 0.02
done
assert_eq "$(cat "$HOME/action")" $'add\n/tmp/a workspace\n--agent\ncodex' 'the parsed binding preserves workspace spaces'
test_summary
