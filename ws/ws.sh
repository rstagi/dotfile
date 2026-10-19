#!/bin/zsh
# Run the Rust CLI. Legacy backend selectors do not apply.
set -u -o pipefail
export WS_HOME="${WS_HOME:-$HOME/.ws}"
export WS_ROOT="${WS_ROOT:-$HOME/.ws/worktrees}"
exec "${WS_RUST_BIN:-$WS_HOME/bin/ws-rs}" "$@"
