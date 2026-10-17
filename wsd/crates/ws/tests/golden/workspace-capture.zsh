#!/bin/zsh
# Capture workspace rows from the zsh sandbox scenarios.
set -e
DEST="${0:A:h}"
source "${0:A:h}/../../../../../ws/tests/helpers.sh"
repo="$(make_repo api)"
"$WS" new --repo "$repo" --branch feat/login --detach
"$WS" list | sed -E -e "s|$WS_ROOT|<ROOT>|g" -e 's/@[0-9]+/<WINDOW>/g' > "$DEST/workspace-live.txt"
"$WS" add api/feat-login --agent codex --detach
"$WS" ls | sed -E -e "s|$WS_ROOT|<ROOT>|g" -e 's/@[0-9]+/<WINDOW>/g' > "$DEST/workspace-added.txt"
T kill-session -t api/feat-login
"$WS" list | sed -E "s|$WS_ROOT|<ROOT>|g" > "$DEST/workspace-stopped.txt"
