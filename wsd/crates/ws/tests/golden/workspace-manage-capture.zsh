#!/bin/zsh
# Capture workspace management behavior in the zsh test sandbox.
set -e
DEST="${0:A:h}"
source "${0:A:h}/../../../../../ws/tests/helpers.sh"
repo="$(make_repo api)"
"$WS" new --repo "$repo" --branch feat/login --detach
"$WS" pin api/feat-login
"$WS" list | sed -E -e "s|$WS_ROOT|<ROOT>|g" -e 's/@[0-9]+/<WINDOW>/g' > "$DEST/workspace-pinned.txt"
"$WS" pin api/feat-login
"$WS" pin missing 2> "$TMP/error" && exit 1
cp "$TMP/error" "$DEST/workspace-missing.txt"
print dirty >> "$WS_ROOT/api/feat-login/file.txt"
"$WS" rm api/feat-login 2> "$TMP/error" && exit 1
sed "s|$WS_ROOT|<ROOT>|g" "$TMP/error" > "$DEST/workspace-dirty.txt"
"$WS" open missing --detach 2> "$TMP/error" && exit 1
cmp "$TMP/error" "$DEST/workspace-missing.txt"
"$WS" rm api/feat-login --force --delete-branch
