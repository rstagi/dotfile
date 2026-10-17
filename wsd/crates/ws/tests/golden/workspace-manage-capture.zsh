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
"$WS" add api/feat-login --agent codex --detach
T kill-session -t api/feat-login
"$WS" open api/feat-login --detach
"$WS" list | sed -E -e "s|$WS_ROOT|<ROOT>|g" -e 's/@[0-9]+/<WINDOW>/g' > "$DEST/workspace-reopened.txt"
"$WS" pin missing 2> "$TMP/error" && exit 1
cp "$TMP/error" "$DEST/workspace-missing.txt"
print dirty >> "$WS_ROOT/api/feat-login/file.txt"
"$WS" rm api/feat-login 2> "$TMP/error" && exit 1
sed "s|$WS_ROOT|<ROOT>|g" "$TMP/error" > "$DEST/workspace-dirty.txt"
"$WS" open missing --detach 2> "$TMP/error" && exit 1
cmp "$TMP/error" "$DEST/workspace-missing.txt"
"$WS" rm api/feat-login --force --delete-branch
for branch in alpha zeta; do "$WS" new --repo "$repo" --branch "$branch" --detach; done
"$WS" pin api/zeta
"$WS" _rows | cut -f2 | sed "s|$WS_ROOT|<ROOT>|g" > "$DEST/workspace-pick-order.txt"
git -C "$repo" worktree add -qb orphan "$WS_ROOT/api/orphan"
"$WS" pin api/orphan
"$WS" list | grep $'\tapi/orphan\t' | sed "s|$WS_ROOT|<ROOT>|g" > "$DEST/workspace-orphan-pinned.txt"
"$WS" new --repo "$repo" --branch keep --detach
print change > "$WS_ROOT/api/keep/new-file"
git -C "$WS_ROOT/api/keep" add .
git -C "$WS_ROOT/api/keep" commit -qm unmerged
"$WS" rm api/keep --delete-branch > /dev/null 2> "$DEST/workspace-unmerged.txt"
