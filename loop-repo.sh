#!/bin/zsh
set -u -o pipefail

# Persistent GitHub repository mapping and fresh-remote integration bootstrap.
# Registry paths stay outside plans so the same owner/repo plan is portable across machines.

CMD="${1:-}"
[[ -n "$CMD" ]] && shift

REGISTRY="${LOOP_REPO_REGISTRY:-${HOME}/.loop/repos.json}"
WORKTREE_ROOT="${LOOP_WORKTREE_ROOT:-${HOME}/.loop/worktrees}"
REPO="" RUN_ID="" INTEGRATION_BRANCH="" POSITIONAL=()

while [[ $# -gt 0 ]]; do
  case "$1" in
  --repo) REPO="$2"; shift 2 ;;
  --run-id) RUN_ID="$2"; shift 2 ;;
  --integration-branch) INTEGRATION_BRANCH="$2"; shift 2 ;;
  *) POSITIONAL+=("$1"); shift ;;
  esac
done

die() { print -u2 "loop-repo: $1"; exit "${2:-1}"; }

validate_slug() {
  local slug_re='^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$'
  [[ "$1" =~ $slug_re ]] \
    || die "invalid repository '$1' (expected owner/repo)"
}

ensure_registry() {
  mkdir -p "${REGISTRY:h}"
  if [[ ! -f "$REGISTRY" ]]; then
    print '{}' > "$REGISTRY"
  fi
  jq -e 'type == "object"' "$REGISTRY" >/dev/null 2>&1 \
    || die "registry is not a JSON object: $REGISTRY"
}

registry_write() {
  local filter="$1" arg1="$2" arg2="${3:-}"
  local tmp
  tmp="$(mktemp "${REGISTRY:h}/.repos.XXXXXX")" || die "cannot create registry temp file"
  if jq --arg a "$arg1" --arg b "$arg2" "$filter" "$REGISTRY" > "$tmp"; then
    mv "$tmp" "$REGISTRY"
  else
    rm -f "$tmp"
    die "could not update registry"
  fi
}

mapping_get() {
  ensure_registry
  jq -er --arg repo "$1" '.[$repo] | select(type == "string" and length > 0)' "$REGISTRY" 2>/dev/null \
    || return 2
}

normalize_checkout() {
  local checkout="$1" resolved
  [[ -d "$checkout" ]] || die "checkout does not exist: $checkout" 3
  resolved="$(cd "$checkout" 2>/dev/null && pwd -P)" || die "cannot resolve checkout: $checkout" 3
  git -C "$resolved" rev-parse --git-dir >/dev/null 2>&1 \
    || die "not a Git checkout: $resolved" 3
  print -r -- "$resolved"
}

origin_slug() {
  local url="$1" slug=""
  case "$url" in
    git@github.com:*) slug="${url#git@github.com:}" ;;
    ssh://git@github.com/*) slug="${url#ssh://git@github.com/}" ;;
    https://github.com/*) slug="${url#https://github.com/}" ;;
    http://github.com/*) slug="${url#http://github.com/}" ;;
  esac
  slug="${slug%.git}"
  print -r -- "$slug"
}

check_repo() {
  validate_slug "$REPO"
  [[ -n "$INTEGRATION_BRANCH" ]] || die "check requires --integration-branch" 1
  local checkout origin actual default_branch collision
  checkout="$(mapping_get "$REPO")" || die "no mapping for $REPO" 2
  checkout="$(normalize_checkout "$checkout")"
  origin="$(git -C "$checkout" config --get remote.origin.url 2>/dev/null)" \
    || die "$REPO checkout has no origin remote" 3
  actual="$(origin_slug "$origin")"
  [[ "$actual" == "$REPO" ]] \
    || die "$REPO mapping origin mismatch: ${actual:-$origin}" 3
  default_branch="$(gh repo view "$REPO" --json defaultBranchRef --jq '.defaultBranchRef.name' 2>/dev/null)" \
    || die "could not resolve GitHub default branch for $REPO" 3
  [[ -n "$default_branch" ]] || die "GitHub returned no default branch for $REPO" 3

  if git -C "$checkout" show-ref --verify --quiet "refs/heads/$INTEGRATION_BRANCH"; then
    die "integration branch already exists in $REPO: $INTEGRATION_BRANCH" 4
  fi
  collision="$(git -C "$checkout" worktree list --porcelain | \
    awk -v target="refs/heads/$INTEGRATION_BRANCH" '$1 == "branch" && $2 == target { print "collision"; exit }')"
  [[ -z "$collision" ]] || die "integration branch is already checked out: $INTEGRATION_BRANCH" 4

  jq -cn --arg repository "$REPO" --arg sourceRoot "$checkout" \
    --arg defaultBranch "$default_branch" --arg integrationBranch "$INTEGRATION_BRANCH" \
    '{repository:$repository, sourceRoot:$sourceRoot, defaultBranch:$defaultBranch,
      integrationBranch:$integrationBranch}'
}

case "$CMD" in
map)
  repo="${POSITIONAL[1]:-}" checkout="${POSITIONAL[2]:-}"
  [[ -n "$repo" && -n "$checkout" ]] || die "map requires <owner/repo> <checkout-root>"
  validate_slug "$repo"
  checkout="$(normalize_checkout "$checkout")"
  ensure_registry
  # shellcheck disable=SC2016 # jq filter, not shell interpolation
  registry_write '.[$a] = $b' "$repo" "$checkout"
  print -r -- "$checkout"
  ;;
get)
  repo="${POSITIONAL[1]:-}"
  [[ -n "$repo" ]] || die "get requires <owner/repo>"
  validate_slug "$repo"
  mapping_get "$repo" || die "no mapping for $repo" 2
  ;;
list)
  ensure_registry
  jq -r 'to_entries | sort_by(.key)[] | "\(.key)\t\(.value)"' "$REGISTRY"
  ;;
unmap)
  repo="${POSITIONAL[1]:-}"
  [[ -n "$repo" ]] || die "unmap requires <owner/repo>"
  validate_slug "$repo"
  ensure_registry
  # shellcheck disable=SC2016 # jq filter, not shell interpolation
  registry_write 'del(.[$a])' "$repo"
  ;;
check)
  check_repo
  ;;
prepare)
  [[ -n "$REPO" ]] || die "prepare requires --repo"
  [[ -n "$RUN_ID" ]] || die "prepare requires --run-id"
  run_id_re='^[A-Za-z0-9._-]+$'
  [[ "$RUN_ID" =~ $run_id_re ]] || die "invalid run id '$RUN_ID'"
  checked="$(check_repo)" || exit $?
  checkout="$(print -r -- "$checked" | jq -r .sourceRoot)"
  default_branch="$(print -r -- "$checked" | jq -r .defaultBranch)"
  git -C "$checkout" fetch origin \
    "+refs/heads/${default_branch}:refs/remotes/origin/${default_branch}" >/dev/null \
    || die "fetch failed for $REPO:$default_branch" 3
  base_sha="$(git -C "$checkout" rev-parse "refs/remotes/origin/$default_branch")" \
    || die "fetched default ref is missing for $REPO" 3
  repo_key="${REPO//\//--}"
  integration_worktree="$WORKTREE_ROOT/$RUN_ID/$repo_key/integration"
  [[ ! -e "$integration_worktree" ]] || die "integration worktree path exists: $integration_worktree" 4
  mkdir -p "${integration_worktree:h}"
  git -C "$checkout" worktree add -q -b "$INTEGRATION_BRANCH" "$integration_worktree" "$base_sha" \
    || die "could not create integration worktree for $REPO" 4
  jq -cn --arg repository "$REPO" --arg sourceRoot "$checkout" \
    --arg defaultBranch "$default_branch" --arg baseSha "$base_sha" \
    --arg integrationBranch "$INTEGRATION_BRANCH" --arg integrationWorktree "$integration_worktree" \
    '{repository:$repository, sourceRoot:$sourceRoot, defaultBranch:$defaultBranch,
      baseSha:$baseSha, integrationBranch:$integrationBranch,
      integrationWorktree:$integrationWorktree, prUrl:null, review:null}'
  ;;
*)
  die "unknown command '$CMD' (map|get|list|unmap|check|prepare)"
  ;;
esac
