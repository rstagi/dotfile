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
  validate_skill_target "$source" "$target" "$host" || return 1
  if [[ -e "$target" || -L "$target" ]]; then
    return 0
  fi
  ln -s "$source" "$target"
}

validate_skill_target() {
  local source="$1" target="$2" host="$3"
  if [[ -e "$target" || -L "$target" ]]; then
    if [[ "$target" -ef "$source" ]]; then
      return 0
    fi
    echo "install-agent-skills: refusing conflicting $host skill: $target" >&2
    return 1
  fi
}

register_ratel_skills() {
  local configured discovered skill source candidate target backup i ratel_rc=0
  local missing=()
  local hidden_targets=()
  local hidden_backups=()
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

  # Ratel 0.9.0 reports the same canonical skill through Claude and both Codex roots,
  # then rejects its own shared candidateId as ambiguous. Hide installer-owned Codex
  # links while importing; restore them on every path below.
  for skill in "${missing[@]}"; do
    source="$SOURCE_ROOT/$skill"
    for target in "$HOME/.agents/skills/$skill" "$HOME/.codex/skills/$skill"; do
      if [[ -L "$target" && "$target" -ef "$source" ]]; then
        backup="$target.ratel-import.$$"
        if [[ -e "$backup" || -L "$backup" ]] || ! mv -- "$target" "$backup"; then
          echo "install-agent-skills: cannot isolate duplicate Ratel candidate: $target" >&2
          ratel_rc=1
          break 2
        fi
        hidden_targets+=("$target")
        hidden_backups+=("$backup")
      fi
    done
  done

  if (( ratel_rc == 0 )); then
    if ! discovered="$(cd "$HOME" && ratel-local skill list --discovered --format json 2>&1)" ||
      ! jq -e '.candidates | type == "array"' >/dev/null 2>&1 <<< "$discovered"; then
      echo "install-agent-skills: cannot discover skills for Ratel" >&2
      ratel_rc=1
    else
      for skill in "${missing[@]}"; do
        source="$SOURCE_ROOT/$skill"
        candidate="$(jq -r --arg id "$skill" --arg path "$source" \
          '.candidates[] | select(.id == $id and .canonicalPath == $path) | .candidateId' \
          <<< "$discovered" | head -1)"
        if [[ -z "$candidate" ]]; then
          echo "install-agent-skills: Ratel cannot discover $source" >&2
          ratel_rc=1
          break
        fi
        if ! (cd "$HOME" && ratel-local skill import "$candidate" \
          --scope user --mode reference --yes); then
          ratel_rc=1
          break
        fi
      done
    fi
  fi

  for ((i=0; i<${#hidden_targets[@]}; i++)); do
    target="${hidden_targets[$i]}"
    backup="${hidden_backups[$i]}"
    if [[ -e "$target" || -L "$target" ]] || ! mv -- "$backup" "$target"; then
      echo "install-agent-skills: cannot restore native skill link: $target" >&2
      ratel_rc=1
    fi
  done
  return "$ratel_rc"
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
  validate_skill_target "$source" "$HOME/.claude/skills/$skill" "Claude Code" || failed=1
  validate_skill_target "$source" "$HOME/.agents/skills/$skill" "Codex" || failed=1
  validate_skill_target "$source" "$HOME/.codex/skills/$skill" "legacy Codex" || failed=1
done
(( failed == 0 )) || exit 1

for skill in "${SHARED_SKILLS[@]}"; do
  link_skill "$SOURCE_ROOT/$skill" "$HOME/.claude/skills/$skill" "Claude Code" || exit 1
done

if [[ "$USE_RATEL" == true ]]; then
  register_ratel_skills || exit 1
fi

for skill in "${SHARED_SKILLS[@]}"; do
  link_skill "$SOURCE_ROOT/$skill" "$HOME/.agents/skills/$skill" "Codex" || exit 1
  link_skill "$SOURCE_ROOT/$skill" "$HOME/.codex/skills/$skill" "legacy Codex" || exit 1
done

echo "Shared Loop skills installed for Claude Code, Codex, and Ratel Local"
