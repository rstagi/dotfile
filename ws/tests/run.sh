#!/bin/zsh
# Run the picker shell smoke test. Cargo covers the Rust CLI and daemon.
set -eu
exec zsh "${0:A:h}/picker-smoke.sh" "$@"
