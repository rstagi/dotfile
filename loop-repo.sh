#!/bin/zsh
set -u -o pipefail

# Persistent GitHub repository mapping and fresh-remote integration bootstrap.
# Registry paths stay outside plans so the same owner/repo plan is portable across machines.
# check/prepare create the integration branch from fetched origin/<default>, or adopt an
# existing non-default one (clean worktree, ff-only to origin/<default>, never behind its remote).
# prepare --milestone N defaults to 1; later milestones create branch-scoped worktrees,
# reusing only their own path on retry. retire removes clean, non-adopted run worktrees;
# pr-state --pr <url> prints open|merged|closed.

CMD="${1:-}"
[[ -n "$CMD" ]] && shift

REGISTRY="${LOOP_REPO_REGISTRY:-${HOME}/.loop/repos.json}"
WORKTREE_ROOT="${LOOP_WORKTREE_ROOT:-${HOME}/.loop/worktrees}"
REPO="" RUN_ID="" INTEGRATION_BRANCH="" MILESTONE=1 WORKTREE="" ADOPTED="" PR="" POSITIONAL=()

while [[ $# -gt 0 ]]; do
  case "$1" in
  --repo) REPO="$2"; shift 2 ;;
  --run-id) RUN_ID="$2"; shift 2 ;;
  --integration-branch) INTEGRATION_BRANCH="$2"; shift 2 ;;
  --milestone) MILESTONE="$2"; shift 2 ;;
  --worktree) WORKTREE="$2"; shift 2 ;;
  --adopted) ADOPTED="$2"; shift 2 ;;
  --pr) PR="$2"; shift 2 ;;
  *) POSITIONAL+=("$1"); shift ;;
  esac
done

die() { print -u2 "loop-repo: $1"; exit "${2:-1}"; }

milestone_re='^[1-9][0-9]*$'
[[ "$MILESTONE" =~ $milestone_re ]] || die "invalid milestone '$MILESTONE'"

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
  [[ "$INTEGRATION_BRANCH" != "$default_branch" ]] \
    || die "integration branch cannot be the default branch ($default_branch)" 4

  local mode=create adopt_worktree="" expected
  if git -C "$checkout" show-ref --verify --quiet "refs/heads/$INTEGRATION_BRANCH"; then
    mode=adopt
    adopt_worktree="$(branch_worktree "$checkout" "$INTEGRATION_BRANCH")" || exit $?
    expected="$(integration_path)"
    if [[ "$CMD" == prepare && -n "$adopt_worktree" && "$adopt_worktree" == "${expected:A}" ]]; then
      mode=reuse
    elif (( MILESTONE > 1 )); then
      die "milestone $MILESTONE cannot adopt existing branch $INTEGRATION_BRANCH" 4
    fi
    if [[ -n "$adopt_worktree" && -n "$(git -C "$adopt_worktree" status --porcelain)" ]]; then
      die "integration branch $INTEGRATION_BRANCH has uncommitted changes in $adopt_worktree; commit or stash first" 4
    fi
  else
    collision="$(branch_worktree "$checkout" "$INTEGRATION_BRANCH")" || exit $?
    [[ -z "$collision" ]] || die "integration branch is already checked out: $INTEGRATION_BRANCH" 4
  fi

  jq -cn --arg repository "$REPO" --arg sourceRoot "$checkout" \
    --arg defaultBranch "$default_branch" --arg integrationBranch "$INTEGRATION_BRANCH" \
    --arg mode "$mode" --arg adoptWorktree "$adopt_worktree" \
    '{repository:$repository, sourceRoot:$sourceRoot, defaultBranch:$defaultBranch,
      integrationBranch:$integrationBranch, mode:$mode,
      adoptWorktree:(if $adoptWorktree == "" then null else $adoptWorktree end)}'
}

# Normalized path of the worktree that has <branch> checked out, or empty.
branch_worktree() {
  local checkout="$1" target="refs/heads/$2" line wt="" resolved
  while IFS= read -r line; do
    case "$line" in
      "worktree "*) wt="${line#worktree }" ;;
      "branch $target")
        resolved="$(cd "$wt" 2>/dev/null && pwd -P)" \
          || die "stale worktree for $2 at $wt; run git worktree prune" 4
        print -r -- "$resolved"
        return 0
        ;;
    esac
  done < <(git -C "$checkout" worktree list --porcelain)
}

# Adopt mode: refuse a local branch behind its remote or diverged from base_sha; set
# integration_worktree (existing, else new run-scoped), then ff-only it to base_sha when the
# branch has no unique commits. The worktree comes first so a refused add moves no ref.
adopt_branch() {
  local checkout="$1" branch="$2" base_sha="$3" default_branch="$4" adopt_worktree="$5"
  local ref="refs/heads/$branch" remote_rc needs_ff=false
  git -C "$checkout" ls-remote --exit-code --heads origin "$ref" >/dev/null 2>&1
  remote_rc=$?
  if [[ $remote_rc -eq 0 ]]; then
    git -C "$checkout" fetch -q origin "+${ref}:refs/remotes/origin/${branch}" >/dev/null \
      || die "fetch failed for $REPO:$branch" 3
    git -C "$checkout" merge-base --is-ancestor "refs/remotes/origin/$branch" "$ref" \
      || die "integration branch $branch is behind or diverged from origin/$branch; pull it first" 4
  elif [[ $remote_rc -ne 2 ]]; then
    die "could not query origin for $REPO:$branch" 3
  fi

  if ! git -C "$checkout" merge-base --is-ancestor "$base_sha" "$ref"; then
    git -C "$checkout" merge-base --is-ancestor "$ref" "$base_sha" \
      || die "integration branch $branch has diverged from origin/$default_branch; rebase or merge it first" 4
    needs_ff=true
  fi

  if [[ -n "$adopt_worktree" ]]; then
    integration_worktree="$adopt_worktree"
  else
    new_integration_worktree "$checkout" "$branch"
  fi
  if $needs_ff; then
    git -C "$integration_worktree" merge -q --ff-only "$base_sha" >/dev/null \
      || die "could not fast-forward $branch in $integration_worktree" 4
  fi
}

# Run-scoped integration worktree path guard + creation; extra args go to worktree add.
new_integration_worktree() {
  local checkout="$1"; shift
  integration_worktree="$(integration_path)"
  [[ ! -e "$integration_worktree" ]] || die "integration worktree path exists: $integration_worktree" 4
  mkdir -p "${integration_worktree:h}"
  git -C "$checkout" worktree add -q "$integration_worktree" "$@" \
    || die "could not create integration worktree for $REPO" 4
}

integration_path() {
  local suffix="" slug
  if (( MILESTONE > 1 )); then
    slug="$(printf '%s' "$INTEGRATION_BRANCH" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//')"
    [[ -n "$slug" ]] || die "integration branch has no usable path slug" 4
    # Hash of the exact branch keeps distinct same-slug branches (feat/m2, feat-m2) apart.
    suffix="-$slug-$(printf '%s' "$INTEGRATION_BRANCH" | shasum | cut -c1-8)"
  fi
  print -r -- "$WORKTREE_ROOT/$RUN_ID/${REPO//\//--}/integration$suffix"
}

retire_worktree() {
  validate_slug "$REPO"
  validate_run_id
  [[ -n "$WORKTREE" ]] || die "retire requires --worktree"
  [[ "$ADOPTED" == true || "$ADOPTED" == false ]] || die "retire requires --adopted true|false"
  [[ "$ADOPTED" == false ]] || die "cannot retire an adopted worktree" 4
  local target="${WORKTREE:A}" run_root="${WORKTREE_ROOT:A}/$RUN_ID" checkout common target_common record
  [[ "$target" == "$run_root/"* ]] || die "cannot retire worktree outside run root: $target" 4
  [[ -e "$target" ]] || return 0
  [[ ! -e "$target/.loop" ]] || die "cannot retire a coordinator worktree: $target" 4
  checkout="$(mapping_get "$REPO")" || die "no mapping for $REPO" 2
  checkout="$(normalize_checkout "$checkout")" || exit $?
  [[ -f "$target/.git" && "$(git -C "$target" rev-parse --show-toplevel 2>/dev/null)" == "$target" ]] \
    || die "not a linked worktree: $target" 4
  common="$(git -C "$checkout" rev-parse --path-format=absolute --git-common-dir)" || exit $?
  target_common="$(git -C "$target" rev-parse --path-format=absolute --git-common-dir)" || exit $?
  [[ "${common:A}" == "${target_common:A}" ]] || die "worktree does not belong to $REPO: $target" 4
  record="$(git -C "$target" rev-parse --absolute-git-dir)/loop-repo-prepare.json"
  if [[ -f "$record" ]]; then
    jq -e '.adopted == false' "$record" >/dev/null 2>&1 \
      || die "cannot retire an adopted worktree: $target" 4
  fi
  [[ -z "$(git -C "$target" status --porcelain --untracked-files=all)" ]] \
    || die "cannot retire dirty worktree: $target" 4
  git -C "$checkout" worktree remove -- "$target" || die "could not retire worktree: $target" 4
}

validate_run_id() {
  local run_id_re='^[A-Za-z0-9._-]+$'
  [[ -n "$RUN_ID" && "$RUN_ID" != . && "$RUN_ID" != .. && "$RUN_ID" =~ $run_id_re ]] \
    || die "invalid run id '$RUN_ID'"
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
  validate_run_id
  checked="$(check_repo)" || exit $?
  checkout="$(print -r -- "$checked" | jq -r .sourceRoot)"
  default_branch="$(print -r -- "$checked" | jq -r .defaultBranch)"
  git -C "$checkout" fetch origin \
    "+refs/heads/${default_branch}:refs/remotes/origin/${default_branch}" >/dev/null \
    || die "fetch failed for $REPO:$default_branch" 3
  base_sha="$(git -C "$checkout" rev-parse "refs/remotes/origin/$default_branch")" \
    || die "fetched default ref is missing for $REPO" 3
  integration_worktree="" adopted=false
  mode="$(print -r -- "$checked" | jq -r .mode)"
  if [[ "$mode" == reuse ]]; then
    integration_worktree="$(print -r -- "$checked" | jq -r .adoptWorktree)"
    # Keep the original base and ownership across retries, even after default advances.
    # Git's per-worktree directory keeps this bookkeeping out of the working tree.
    record="$(git -C "$integration_worktree" rev-parse --absolute-git-dir)/loop-repo-prepare.json"
    if [[ -f "$record" ]]; then
      base_sha="$(jq -er .baseSha "$record")" || die "invalid prepare record: $record" 4
      adopted="$(jq -er '.adopted | tostring' "$record")" || die "invalid prepare record: $record" 4
    fi
  elif [[ "$mode" == adopt ]]; then
    adopted=true
    adopt_branch "$checkout" "$INTEGRATION_BRANCH" "$base_sha" "$default_branch" \
      "$(print -r -- "$checked" | jq -r '.adoptWorktree // empty')"
  else
    new_integration_worktree "$checkout" -b "$INTEGRATION_BRANCH" "$base_sha"
  fi
  ahead_count="$(git -C "$checkout" rev-list --count "$base_sha..refs/heads/$INTEGRATION_BRANCH")"
  expected="$(integration_path)"
  if [[ "${integration_worktree:A}" == "${expected:A}" ]]; then
    record="$(git -C "$integration_worktree" rev-parse --absolute-git-dir)/loop-repo-prepare.json"
    tmp="$(mktemp "$record.XXXXXX")" || die "cannot create prepare record" 4
    jq -cn --arg baseSha "$base_sha" --argjson adopted "$adopted" \
      '{baseSha:$baseSha, adopted:$adopted}' > "$tmp" && mv "$tmp" "$record" \
      || die "cannot save prepare record" 4
  fi
  jq -cn --arg repository "$REPO" --arg sourceRoot "$checkout" \
    --arg defaultBranch "$default_branch" --arg baseSha "$base_sha" \
    --arg integrationBranch "$INTEGRATION_BRANCH" --arg integrationWorktree "$integration_worktree" \
    --argjson adopted "$adopted" --argjson aheadCount "$ahead_count" \
    '{repository:$repository, sourceRoot:$sourceRoot, defaultBranch:$defaultBranch,
      baseSha:$baseSha, integrationBranch:$integrationBranch,
      integrationWorktree:$integrationWorktree, adopted:$adopted, aheadCount:$aheadCount,
      prUrl:null, review:null}'
  ;;
retire)
  retire_worktree
  ;;
pr-state)
  [[ -n "$PR" ]] || die "pr-state requires --pr"
  pr_json="$(gh pr view "$PR" --json state)" || die "could not read PR state: $PR" 3
  print -r -- "$pr_json" | jq -er '.state | ascii_downcase | select(. == "open" or . == "merged" or . == "closed")' \
    || die "GitHub returned an invalid PR state: $PR" 3
  ;;
*)
  die "unknown command '$CMD' (map|get|list|unmap|check|prepare|retire|pr-state)"
  ;;
esac
