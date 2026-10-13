#!/bin/zsh
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
trap 'rm -rf "$TMP"' EXIT
export LOOP_REPO_REGISTRY="$TMP/home/repos.json"
export LOOP_WORKTREE_ROOT="$TMP/worktrees"
mkdir -p "$TMP/bin" "$TMP/home"

cat > "$TMP/bin/gh" <<'EOF'
#!/bin/sh
printf '%s\n' "${FAKE_DEFAULT_BRANCH:-main}"
EOF
chmod +x "$TMP/bin/gh"
export PATH="$TMP/bin:$PATH"

make_checkout() {
  local slug="$1" default_branch="$2"
  local remote="$TMP/${slug:t}.git" checkout="$TMP/${slug:t}"
  git init --bare -q "$remote"
  git init -q -b "$default_branch" "$checkout"
  git -C "$checkout" config user.email test@example.com
  git -C "$checkout" config user.name Test
  print one > "$checkout/file.txt"
  git -C "$checkout" add file.txt
  git -C "$checkout" commit -qm initial
  git -C "$checkout" remote add origin "https://github.com/$slug.git"
  git -C "$checkout" push -q "$remote" "$default_branch"
  git -C "$checkout" remote set-url origin "git@github.com:$slug.git"
  git --git-dir "$remote" symbolic-ref HEAD "refs/heads/$default_branch"
  print "$checkout|$remote"
}

pair="$(make_checkout acme/api trunk)"
CHECKOUT="${pair%%|*}"
REMOTE="${pair#*|}"

echo "loop-repo: map/get/list/unmap persists mappings"
out="$($ROOT/loop-repo.sh map acme/api "$CHECKOUT")"
assert_eq "$out" "$CHECKOUT" "map prints normalized checkout"
assert_eq "$($ROOT/loop-repo.sh get acme/api)" "$CHECKOUT" "get returns mapping"
assert_contains "$($ROOT/loop-repo.sh list)" $'acme/api\t' "list includes slug"
assert_eq "$(jq -r '.["acme/api"]' "$LOOP_REPO_REGISTRY")" "$CHECKOUT" "registry is valid JSON"
$ROOT/loop-repo.sh unmap acme/api
$ROOT/loop-repo.sh get acme/api >/dev/null 2>&1
assert_exit "$?" "2" "unmap removes mapping"

echo "loop-repo: check validates mapping, origin, default branch, collisions"
$ROOT/loop-repo.sh map acme/api "$CHECKOUT" >/dev/null
export FAKE_DEFAULT_BRANCH=trunk
out="$($ROOT/loop-repo.sh check --repo acme/api --integration-branch feat/api)"
assert_eq "$(print "$out" | jq -r .defaultBranch)" "trunk" "check resolves non-main default"
git -C "$CHECKOUT" remote set-url origin https://github.com/acme/wrong.git
$ROOT/loop-repo.sh check --repo acme/api --integration-branch feat/api >/dev/null 2>&1
assert_exit "$?" "3" "origin mismatch refused"
git -C "$CHECKOUT" remote set-url origin git@github.com:acme/api.git
assert_eq "$(print "$out" | jq -r .mode)" "create" "check reports create mode for a new branch"
assert_eq "$(print "$out" | jq -r .adoptWorktree)" "null" "create mode has no adopt worktree"
$ROOT/loop-repo.sh check --repo acme/api --integration-branch trunk >/dev/null 2>&1
assert_exit "$?" "4" "default branch refused as integration branch"

echo "loop-repo: check reports adopt mode for an existing branch, refuses dirty worktrees"
git -C "$CHECKOUT" branch feat/collision
out="$($ROOT/loop-repo.sh check --repo acme/api --integration-branch feat/collision)"
assert_eq "$(print "$out" | jq -r .mode)" "adopt" "existing branch is adopted"
assert_eq "$(print "$out" | jq -r .adoptWorktree)" "null" "unchecked-out branch has no adopt worktree"
git -C "$CHECKOUT" worktree add -q "$TMP/ws-check" feat/collision
print '.loop/' > "$TMP/ws-check/.gitignore"
git -C "$TMP/ws-check" add .gitignore
git -C "$TMP/ws-check" commit -qm ignore-loop
mkdir -p "$TMP/ws-check/.loop" && print '{}' > "$TMP/ws-check/.loop/state.json"
out="$($ROOT/loop-repo.sh check --repo acme/api --integration-branch feat/collision)"
assert_exit "$?" "0" "gitignored .loop/ is not dirty"
assert_eq "$(print "$out" | jq -r .adoptWorktree)" "$TMP/ws-check" "check reports checked-out worktree"
print scratch > "$TMP/ws-check/scratch.txt"
$ROOT/loop-repo.sh check --repo acme/api --integration-branch feat/collision >/dev/null 2>&1
assert_exit "$?" "4" "untracked file makes adopted worktree dirty"
rm "$TMP/ws-check/scratch.txt"
print edit >> "$TMP/ws-check/file.txt"
$ROOT/loop-repo.sh check --repo acme/api --integration-branch feat/collision >/dev/null 2>&1
assert_exit "$?" "4" "tracked edit makes adopted worktree dirty"
git -C "$CHECKOUT" worktree remove --force "$TMP/ws-check"
git -C "$CHECKOUT" branch -D feat/collision >/dev/null

echo "loop-repo: prepare fetches remote tip and creates exact-base integration worktree"
git --git-dir "$REMOTE" worktree add -q "$TMP/remote-edit" trunk
git -C "$TMP/remote-edit" config user.email test@example.com
git -C "$TMP/remote-edit" config user.name Test
print two >> "$TMP/remote-edit/file.txt"
git -C "$TMP/remote-edit" commit -qam remote-tip
REMOTE_SHA="$(git -C "$TMP/remote-edit" rev-parse HEAD)"
git --git-dir "$REMOTE" worktree remove "$TMP/remote-edit"

# Route the checkout's GitHub-looking origin to the local bare remote for fetches.
git -C "$CHECKOUT" config url."$REMOTE".insteadOf git@github.com:acme/api.git
out="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-1 --integration-branch feat/api)"
WT="$(print "$out" | jq -r .integrationWorktree)"
assert_eq "$(print "$out" | jq -r .baseSha)" "$REMOTE_SHA" "prepare records fresh remote SHA"
assert_eq "$(git -C "$WT" rev-parse HEAD)" "$REMOTE_SHA" "integration worktree starts at fetched SHA"
assert_eq "$WT" "$LOOP_WORKTREE_ROOT/run-1/acme--api/integration" "worktree path is run/repo scoped"
assert_eq "$(print "$out" | jq -r .adopted)" "false" "create mode is not adopted"
assert_eq "$(print "$out" | jq -r .aheadCount)" "0" "create mode has nothing ahead"

echo "loop-repo: prepare adopts existing integration branches"
BASE_PARENT="$(git -C "$CHECKOUT" rev-parse "$REMOTE_SHA^")"
# new_ws <branch> <start> — ws-style worktree on <branch>
new_ws() {
  git -C "$CHECKOUT" worktree add -q -b "$1" "$TMP/ws-${1:t}" "$2"
  git -C "$TMP/ws-${1:t}" config user.email test@example.com
  git -C "$TMP/ws-${1:t}" config user.name Test
  print -r -- "$TMP/ws-${1:t}"
}
ws_commit() { print "$2" >> "$1/file.txt"; git -C "$1" commit -qam "$2"; }

WS="$(new_ws feat/same "$REMOTE_SHA")"
out="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-2 --integration-branch feat/same)"
assert_eq "$(print "$out" | jq -r .integrationWorktree)" "$WS" "adopts checked-out worktree"
assert_eq "$(print "$out" | jq -r .adopted)" "true" "adopted flag set"
assert_eq "$(print "$out" | jq -r .aheadCount)" "0" "branch equal to default has nothing ahead"
assert_eq "$(print "$out" | jq -r .baseSha)" "$REMOTE_SHA" "adopt records fetched default SHA"
[[ -e "$LOOP_WORKTREE_ROOT/run-2" ]]
assert_exit "$?" "1" "adopt creates no run-scoped worktree"

WS="$(new_ws feat/behind "$BASE_PARENT")"
out="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-3 --integration-branch feat/behind)"
assert_eq "$(git -C "$WS" rev-parse HEAD)" "$REMOTE_SHA" "behind branch fast-forwarded to fetched default"
assert_eq "$(git -C "$WS" status --porcelain)" "" "fast-forwarded worktree stays clean"
git -C "$WS" diff --quiet "$REMOTE_SHA"
assert_exit "$?" "0" "fast-forwarded worktree files match fetched default"
assert_eq "$(print "$out" | jq -r .aheadCount)" "0" "fast-forwarded branch has nothing ahead"

WS="$(new_ws feat/diverged "$BASE_PARENT")"
ws_commit "$WS" diverged
DIVERGED_SHA="$(git -C "$WS" rev-parse HEAD)"
err="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-4 --integration-branch feat/diverged 2>&1 >/dev/null)"
assert_exit "$?" "4" "branch diverged from default refused"
assert_contains "$err" "diverged from origin/trunk" "divergence refused by the guard"
assert_eq "$(git -C "$WS" rev-parse HEAD)" "$DIVERGED_SHA" "diverged branch left untouched"

WS="$(new_ws feat/ahead "$REMOTE_SHA")"
ws_commit "$WS" ahead-1
git -C "$WS" push -q "$REMOTE" feat/ahead
ws_commit "$WS" ahead-2
out="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-5 --integration-branch feat/ahead)"
assert_eq "$(print "$out" | jq -r .aheadCount)" "2" "aheadCount counts commits beyond default"
assert_eq "$(print "$out" | jq -r .integrationWorktree)" "$WS" "remote branch behind local is accepted"

git -C "$CHECKOUT" branch feat/loose "$BASE_PARENT"
out="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-6 --integration-branch feat/loose)"
WT6="$(print "$out" | jq -r .integrationWorktree)"
assert_eq "$WT6" "$LOOP_WORKTREE_ROOT/run-6/acme--api/integration" "unchecked-out branch gets run-scoped worktree"
assert_eq "$(git -C "$WT6" symbolic-ref --short HEAD)" "feat/loose" "run-scoped worktree is on adopted branch"
assert_eq "$(git -C "$WT6" rev-parse HEAD)" "$REMOTE_SHA" "unchecked-out behind branch fast-forwarded"
assert_eq "$(print "$out" | jq -r .adopted)" "true" "unchecked-out branch adopted"

WS="$(new_ws feat/remote-ahead "$REMOTE_SHA")"
ws_commit "$WS" remote-only
git -C "$WS" push -q "$REMOTE" feat/remote-ahead
git -C "$WS" reset -q --hard "$REMOTE_SHA"
$ROOT/loop-repo.sh prepare --repo acme/api --run-id run-7 --integration-branch feat/remote-ahead >/dev/null 2>&1
assert_exit "$?" "4" "remote integration branch ahead of local refused"

git -C "$CHECKOUT" branch feat/loose-diverged "$DIVERGED_SHA"
err="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-8 --integration-branch feat/loose-diverged 2>&1 >/dev/null)"
assert_exit "$?" "4" "unchecked-out diverged branch refused"
assert_contains "$err" "diverged from origin/trunk" "unchecked-out divergence refused by the guard"
assert_eq "$(git -C "$CHECKOUT" rev-parse feat/loose-diverged)" "$DIVERGED_SHA" "unchecked-out diverged branch untouched"
[[ -e "$LOOP_WORKTREE_ROOT/run-8" ]]
assert_exit "$?" "1" "refused adopt creates no run-scoped worktree"

# Paused rebase detaches HEAD: the branch looks unchecked-out but is still in use.
WS="$(new_ws feat/rebasing "$BASE_PARENT")"
GIT_SEQUENCE_EDITOR="sh -c 'echo break > \"\$1\"' --" git -C "$WS" rebase -q -i HEAD >/dev/null 2>&1
$ROOT/loop-repo.sh prepare --repo acme/api --run-id run-9 --integration-branch feat/rebasing >/dev/null 2>&1
assert_exit "$?" "4" "branch under paused rebase refused"
assert_eq "$(git -C "$CHECKOUT" rev-parse feat/rebasing)" "$BASE_PARENT" "branch under paused rebase not moved"
git -C "$WS" rebase --abort

echo "loop-repo: two repositories merge into independent integration worktrees"
web_pair="$(make_checkout acme/web stable)"
WEB_CHECKOUT="${web_pair%%|*}"
WEB_REMOTE="${web_pair#*|}"
$ROOT/loop-repo.sh map acme/web "$WEB_CHECKOUT" >/dev/null
git -C "$WEB_CHECKOUT" config url."$WEB_REMOTE".insteadOf git@github.com:acme/web.git
export FAKE_DEFAULT_BRANCH=stable
web_out="$($ROOT/loop-repo.sh prepare --repo acme/web --run-id run-1 --integration-branch feat/web)"
WEB_WT="$(print "$web_out" | jq -r .integrationWorktree)"

git -C "$CHECKOUT" worktree add -q -b feat/api-change "$TMP/api-lane" feat/api
print api > "$TMP/api-lane/result.txt"
git -C "$TMP/api-lane" add result.txt
git -C "$TMP/api-lane" commit -qm api-change
$ROOT/loop-merge.sh --worktree "$WT" --lane-branch feat/api-change \
  --verify-cmd 'test "$(cat result.txt)" = api' --no-push >/dev/null

git -C "$WEB_CHECKOUT" worktree add -q -b feat/web-change "$TMP/web-lane" feat/web
print web > "$TMP/web-lane/result.txt"
git -C "$TMP/web-lane" add result.txt
git -C "$TMP/web-lane" commit -qm web-change
$ROOT/loop-merge.sh --worktree "$WEB_WT" --lane-branch feat/web-change \
  --verify-cmd 'test "$(cat result.txt)" = web' --no-push >/dev/null

assert_eq "$(cat "$WT/result.txt")" "api" "API verify/merge lands in API integration"
assert_eq "$(cat "$WEB_WT/result.txt")" "web" "web verify/merge lands in web integration"
git -C "$WT" rev-parse --verify feat/web-change >/dev/null 2>&1
assert_exit "$?" "128" "repository histories stay separate"

test_summary
