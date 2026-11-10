#!/bin/bash
set -u

printf '%s\n' "$*" >> "$FAKE_RATEL_LOG"

case "${1:-} ${2:-} ${3:-}" in
  "skill list --configured")
    printf '%s\n' "${FAKE_RATEL_CONFIGURED_JSON:-[]}" >&2
    ;;
  "skill list --discovered")
    jq -cn --arg root "$FAKE_SKILL_SOURCE" '
      {candidates: [
        "checkpoint-30", "assess", "loop-do", "pr-review", "ws-sessions", "ws-orchestrate", "asd-ste100",
        "visual-explainer", "engineering-standards"
      ] | map({
        candidateId: ("cand-" + .), id: ., source: "claude",
        canonicalPath: ($root + "/" + .)
      })}' >&2
    ;;
  "skill import "*)
    if [[ -L "$HOME/.agents/skills/pr-review" ||
      -L "$HOME/.codex/skills/pr-review" ]]; then
      printf 'ambiguous discovered skill %s; use its candidateId\n' "${3:-unknown}" >&2
      exit 1
    fi
    printf 'imported 1 skill(s)\n' >&2
    ;;
  *)
    printf 'unexpected ratel-local call: %s\n' "$*" >&2
    exit 1
    ;;
esac
