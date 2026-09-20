#!/bin/bash
set -u

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SOURCE_ROOT="$SCRIPT_DIR/.claude/skills"
USE_RATEL=true
SHARED_SKILLS=(
  multiphase-plan
  loop-execute
  loop-pickup
  loop-handoff
  pr-review
  pr-review-fix-all
)

usage() {
  echo "usage: install-agent-skills.sh [--source <skills-dir>] [--no-ratel]" >&2
}

link_skill() {
  local source="$1" target="$2" host="$3"
  if [[ -e "$target" || -L "$target" ]]; then
    if [[ "$target" -ef "$source" ]]; then
      return 0
    fi
    echo "install-agent-skills: refusing conflicting $host skill: $target" >&2
    return 1
  fi
  ln -s "$source" "$target"
}

register_ratel_skills() {
  local configured discovered skill source candidate
  local missing=()
  if ! command -v ratel-local >/dev/null 2>&1; then
    echo "install-agent-skills: ratel-local unavailable; skipping Ratel registration" >&2
    return 0
  fi
  if ! configured="$(cd "$HOME" && ratel-local skill list --configured --format json 2>&1)" ||
    ! jq -e 'type == "array"' >/dev/null 2>&1 <<< "$configured"; then
    echo "install-agent-skills: cannot read Ratel skill registrations" >&2
    return 1
  fi
  for skill in "${SHARED_SKILLS[@]}"; do
    jq -e --arg id "$skill" 'any(.[]; .id == $id)' >/dev/null 2>&1 <<< "$configured" ||
      missing+=("$skill")
  done
  (( ${#missing[@]} > 0 )) || return 0

  if ! discovered="$(cd "$HOME" && ratel-local skill list --discovered --format json 2>&1)" ||
    ! jq -e '.candidates | type == "array"' >/dev/null 2>&1 <<< "$discovered"; then
    echo "install-agent-skills: cannot discover skills for Ratel" >&2
    return 1
  fi

  for skill in "${missing[@]}"; do
    source="$SOURCE_ROOT/$skill"
    candidate="$(jq -r --arg id "$skill" --arg path "$source" \
      '.candidates[] | select(.id == $id and .canonicalPath == $path) | .candidateId' \
      <<< "$discovered" | head -1)"
    if [[ -z "$candidate" ]]; then
      echo "install-agent-skills: Ratel cannot discover $source" >&2
      return 1
    fi
    (cd "$HOME" && ratel-local skill import "$candidate" \
      --scope user --mode reference --yes) || return 1
  done
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source)
      [[ $# -ge 2 ]] || { usage; exit 1; }
      SOURCE_ROOT="$2"
      shift 2
      ;;
    --no-ratel)
      USE_RATEL=false
      shift
      ;;
    *)
      usage
      exit 1
      ;;
  esac
done

SOURCE_ROOT="$(cd "$SOURCE_ROOT" 2>/dev/null && pwd -P)" || {
  echo "install-agent-skills: skills directory not found" >&2
  exit 1
}

mkdir -p "$HOME/.claude/skills" "$HOME/.agents/skills" "$HOME/.codex/skills"
failed=0
for skill in "${SHARED_SKILLS[@]}"; do
  source="$SOURCE_ROOT/$skill"
  if [[ ! -f "$source/SKILL.md" ]]; then
    echo "install-agent-skills: missing $source/SKILL.md" >&2
    failed=1
    continue
  fi
  link_skill "$source" "$HOME/.claude/skills/$skill" "Claude Code" || failed=1
  link_skill "$source" "$HOME/.agents/skills/$skill" "Codex" || failed=1
  link_skill "$source" "$HOME/.codex/skills/$skill" "legacy Codex" || failed=1
done
(( failed == 0 )) || exit 1

if [[ "$USE_RATEL" == true ]]; then
  register_ratel_skills || exit 1
fi

echo "Shared Loop skills installed for Claude Code, Codex, and Ratel Local"
