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
if [ "$1" = pr ] && [ "$2" = view ]; then
  [ "$3" = https://github.com/acme/api/pull/123 ] && [ "$4" = --json ] && [ "$5" = state ] || exit 99
  [ "${FAKE_PR_FAILURE:-0}" = 0 ] || exit "$FAKE_PR_FAILURE"
  printf '{"state":"%s"}\n' "${FAKE_PR_STATE:-OPEN}"
  exit 0
fi
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

echo "loop-repo: later milestones start fresh in independent worktrees"
export FAKE_DEFAULT_BRANCH=trunk
git --git-dir "$REMOTE" worktree add -q "$TMP/remote-edit" trunk
print milestone-two >> "$TMP/remote-edit/file.txt"
git -C "$TMP/remote-edit" commit -qam milestone-two-base
M2_BASE="$(git -C "$TMP/remote-edit" rev-parse HEAD)"
git --git-dir "$REMOTE" worktree remove "$TMP/remote-edit"
m2_out="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-1 \
  --milestone 2 --integration-branch feat/api-milestone-2)"
assert_exit "$?" "0" "second milestone prepares in the same run"
M2_WT="$(print "$m2_out" | jq -r .integrationWorktree)"
assert_eq "$M2_WT" "$LOOP_WORKTREE_ROOT/run-1/acme--api/integration-feat-api-milestone-2" "later milestone path includes branch slug"
assert_eq "$(print "$m2_out" | jq -r .baseSha)" "$M2_BASE" "later milestone fetches the latest default"
assert_eq "$(print "$m2_out" | jq -r .adopted)" "false" "later milestone creates rather than adopts"
if [[ -d "$M2_WT" ]]; then
  assert_eq "$(git -C "$M2_WT" rev-parse HEAD)" "$M2_BASE" "later milestone starts exactly at default"
  [[ -e "$M2_WT/result.txt" ]]
  assert_exit "$?" "1" "previous integration changes do not leak into new milestone"
fi
assert_eq "$(cat "$WT/result.txt")" "api" "first milestone worktree remains available"

echo "loop-repo: later milestones never adopt existing branches"
git -C "$CHECKOUT" branch feat/milestone-existing "$M2_BASE"
$ROOT/loop-repo.sh prepare --repo acme/api --run-id run-existing \
  --milestone 2 --integration-branch feat/milestone-existing >/dev/null 2>&1
assert_exit "$?" "4" "later milestone refuses an unchecked-out existing branch"
WS="$(new_ws feat/milestone-worktree "$M2_BASE")"
$ROOT/loop-repo.sh prepare --repo acme/api --run-id run-existing \
  --milestone 3 --integration-branch feat/milestone-worktree >/dev/null 2>&1
assert_exit "$?" "4" "later milestone refuses an existing external worktree"
assert_eq "$(git -C "$WS" rev-parse HEAD)" "$M2_BASE" "refused adoption leaves existing worktree untouched"

echo "loop-repo: prepare retries reuse their own worktrees"
retry_out="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-1 \
  --milestone 2 --integration-branch feat/api-milestone-2)"
assert_exit "$?" "0" "later milestone prepare is retryable"
assert_eq "$retry_out" "$m2_out" "retry returns the same milestone integration record"
retry_out="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-1 \
  --milestone 1 --integration-branch feat/api)"
assert_exit "$?" "0" "first milestone retry survives default advancing"
assert_eq "$(print "$retry_out" | jq -r .integrationWorktree)" "$WT" "first milestone retry keeps legacy path"
assert_eq "$(print "$retry_out" | jq -r .baseSha)" "$REMOTE_SHA" "retry retains original base after default advances"
assert_eq "$(print "$retry_out" | jq -r .adopted)" "false" "retry does not relabel owned worktrees as adopted"
assert_eq "$(print "$retry_out" | jq -r .aheadCount)" "2" "retry counts lane and merge commits already in integration"
retry_out="$($ROOT/loop-repo.sh prepare --repo acme/api --run-id run-6 --integration-branch feat/loose)"
assert_exit "$?" "0" "retry also supports adopted run-scoped worktrees"
assert_eq "$(print "$retry_out" | jq -r .adopted)" "true" "retry retains adopted ownership"

echo "loop-repo: retries refuse a different branch occupying the expected path"
git -C "$M2_WT" switch -q -c feat/milestone-other
$ROOT/loop-repo.sh prepare --repo acme/api --run-id run-1 \
  --milestone 2 --integration-branch feat/api-milestone-2 >/dev/null 2>&1
assert_exit "$?" "4" "retry refuses a worktree switched to a different branch"
assert_eq "$(git -C "$M2_WT" branch --show-current)" "feat/milestone-other" "retry leaves the other branch untouched"
git -C "$M2_WT" switch -q feat/api-milestone-2

echo "loop-repo: retire removes clean owned worktrees and is retryable"
PRE_RETIRE_SHA="$(git -C "$WT" rev-parse HEAD)"
$ROOT/loop-repo.sh retire --repo acme/api --run-id run-1 --worktree "$WT" --adopted false
assert_exit "$?" "0" "retire removes a clean non-adopted worktree"
[[ -e "$WT" ]]
assert_exit "$?" "1" "retired worktree directory is gone"
assert_eq "$(git -C "$CHECKOUT" rev-parse feat/api)" "$PRE_RETIRE_SHA" "retire keeps the integration branch"
$ROOT/loop-repo.sh retire --repo acme/api --run-id run-1 --worktree "$WT" --adopted false
assert_exit "$?" "0" "retire succeeds when worktree is already gone"

echo "loop-repo: retire refuses adopted, dirty, outside-root, and coordinator worktrees"
refuse_retire() {
  local target="$1" adopted="$2" message="$3"
  $ROOT/loop-repo.sh retire --repo acme/api --run-id run-1 --worktree "$target" --adopted "$adopted" >/dev/null 2>&1
  assert_exit "$?" "4" "$message"
  [[ -e "$target" ]]
  assert_exit "$?" "0" "$message leaves worktree intact"
}
refuse_retire "$M2_WT" true "explicitly adopted worktree refused"
print dirty >> "$M2_WT/file.txt"
refuse_retire "$M2_WT" false "tracked changes refused"
git -C "$M2_WT" restore file.txt
print scratch > "$M2_WT/scratch.txt"
refuse_retire "$M2_WT" false "untracked changes refused"
rm "$M2_WT/scratch.txt"
print staged > "$M2_WT/staged.txt"
git -C "$M2_WT" add staged.txt
refuse_retire "$M2_WT" false "staged changes refused"
git -C "$M2_WT" restore --staged staged.txt
rm "$M2_WT/staged.txt"
refuse_retire "$WS" false "outside-root worktree refused"
refuse_retire "$WT6" false "another run's worktree refused"
mkdir -p "$M2_WT/.loop"
print '{}' > "$M2_WT/.loop/state.json"
print '.loop/' >> "$(git -C "$M2_WT" rev-parse --path-format=absolute --git-common-dir)/info/exclude"
assert_eq "$(git -C "$M2_WT" status --porcelain)" "" "coordinator state is gitignored"
refuse_retire "$M2_WT" false "clean coordinator worktree refused"
rm -r "$M2_WT/.loop"
$ROOT/loop-repo.sh retire --repo acme/api --run-id run-6 --worktree "$WT6" --adopted true >/dev/null 2>&1
assert_exit "$?" "4" "adopted run-scoped worktree refused"
$ROOT/loop-repo.sh retire --repo acme/api --run-id run-6 --worktree "$WT6" --adopted false >/dev/null 2>&1
assert_exit "$?" "4" "recorded adopted ownership cannot be overridden by caller"

echo "loop-repo: retire guards path aliases and repository ownership"
ln -s "$WS" "$LOOP_WORKTREE_ROOT/run-1/escape"
refuse_retire "$LOOP_WORKTREE_ROOT/run-1/escape" false "symlink escaping run root refused"
$ROOT/loop-repo.sh retire --repo acme/api --run-id run-1 \
  --worktree "$LOOP_WORKTREE_ROOT/run-1/../run-6/acme--api/integration" --adopted false >/dev/null 2>&1
assert_exit "$?" "4" "path traversal into another run refused"
FOREIGN_WT="$LOOP_WORKTREE_ROOT/run-1/acme--web/retire-test"
git -C "$WEB_CHECKOUT" worktree add -q -b feat/retire-test "$FOREIGN_WT" feat/web
refuse_retire "$FOREIGN_WT" false "another repository's worktree refused"
refuse_retire "$LOOP_WORKTREE_ROOT/run-1" false "run root itself refused"
$ROOT/loop-repo.sh retire --repo acme/api --run-id .. --worktree "$M2_WT" --adopted false >/dev/null 2>&1
assert_exit "$?" "1" "parent-directory run id refused"
$ROOT/loop-repo.sh retire --repo acme/api --run-id run-1 --worktree "$M2_WT" >/dev/null 2>&1
assert_exit "$?" "1" "missing adopted flag refused"
$ROOT/loop-repo.sh retire --repo acme/api --run-id run-1 --worktree "$M2_WT" --adopted yes >/dev/null 2>&1
assert_exit "$?" "1" "invalid adopted flag refused"
$ROOT/loop-repo.sh retire --repo acme/api --run-id run-1 --worktree "$TMP/missing" --adopted false >/dev/null 2>&1
assert_exit "$?" "4" "missing path outside run root still refused"

echo "loop-repo: pr-state reports GitHub state through the gh boundary"
PR_URL=https://github.com/acme/api/pull/123
for pr_state in OPEN MERGED CLOSED; do
  out="$(FAKE_PR_STATE="$pr_state" $ROOT/loop-repo.sh pr-state --pr "$PR_URL")"
  assert_exit "$?" "0" "pr-state accepts $pr_state"
  assert_eq "$out" "${(L)pr_state}" "pr-state normalizes $pr_state"
done
out="$(FAKE_PR_FAILURE=7 $ROOT/loop-repo.sh pr-state --pr "$PR_URL" 2>/dev/null)"
assert_exit "$?" "3" "pr-state fails when gh fails"
assert_eq "$out" "" "gh failure never prints a success state"
$ROOT/loop-repo.sh pr-state >/dev/null 2>&1
assert_exit "$?" "1" "pr-state requires a PR URL"
out="$(FAKE_PR_STATE=UNKNOWN $ROOT/loop-repo.sh pr-state --pr "$PR_URL" 2>/dev/null)"
assert_exit "$?" "3" "pr-state refuses unexpected GitHub state"
assert_eq "$out" "" "unknown state never prints a success state"

test_summary
