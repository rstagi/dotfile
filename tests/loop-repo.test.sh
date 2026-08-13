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
git -C "$CHECKOUT" branch feat/collision
$ROOT/loop-repo.sh check --repo acme/api --integration-branch feat/collision >/dev/null 2>&1
assert_exit "$?" "4" "existing integration branch refused"
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
