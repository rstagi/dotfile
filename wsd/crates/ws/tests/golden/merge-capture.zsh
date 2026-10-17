#!/bin/zsh
# Capture merge prompts and errors in the zsh test sandbox.
set -e
DEST="${0:A:h}"
source "${0:A:h}/../../../../../ws/tests/helpers.sh"
repo="$(make_repo merge-me)"
git init -q --bare "$TMP/origin.git"
git -C "$repo" remote add origin "$TMP/origin.git"
git -C "$repo" push -q origin main
git --git-dir="$TMP/origin.git" symbolic-ref HEAD refs/heads/main
git -C "$repo" fetch -q origin
git -C "$repo" remote set-head origin -a >/dev/null
"$WS" new --repo "$repo" --branch feat/ship --detach
wt="$WS_ROOT/merge-me/feat-ship"
cat > "$TMP/bin/gh" <<'GH'
#!/bin/sh
[ "$1 $2" != 'pr view' ] || echo '#7 Ship it (https://github.com/acme/merge-me/pull/7)'
exit 0
GH
chmod +x "$TMP/bin/gh"
(cd "$wt" && print n | "$WS" merge) > "$DEST/merge-declined.txt"
(cd "$wt" && printf 'y\nn\n' | "$WS" merge) > "$TMP/kept"
sed "s|$repo|<REPO>|g" "$TMP/kept" > "$DEST/merge-kept.txt"
print junk > "$wt/stray.txt"
(cd "$wt" && printf 'y\ny\nn\n' | "$WS" merge) > "$TMP/dirty"
sed "s|$repo|<REPO>|g" "$TMP/dirty" > "$DEST/merge-dirty-kept.txt"
(cd "$repo" && env -u TMUX "$WS" merge </dev/null) 2> "$DEST/merge-outside.txt" && exit 1
(cd "$repo" && printf '\n' | env -u TMUX "$WS" _merge_popup) > "$DEST/merge-popup.txt" 2> "$TMP/error"
cmp "$TMP/error" "$DEST/merge-outside.txt"
git -C "$wt" checkout -q --detach
(cd "$wt" && "$WS" merge </dev/null) 2> "$TMP/error" && exit 1
sed "s|$wt|<WT>|g" "$TMP/error" > "$DEST/merge-detached.txt"
git -C "$wt" checkout -q feat/ship
print '#!/bin/sh\nexit 1' > "$TMP/bin/gh"
(cd "$wt" && "$WS" merge </dev/null) 2> "$DEST/merge-no-pr.txt" && exit 1
exit 0
