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
        "multiphase-plan", "loop-execute", "loop-pickup", "loop-handoff",
        "pr-review", "pr-review-fix-all"
      ] | map({
        candidateId: ("cand-" + .), id: ., source: "claude",
        canonicalPath: ($root + "/" + .)
      })}' >&2
    ;;
  "skill import "*)
    printf 'imported 1 skill(s)\n' >&2
    ;;
  *)
    printf 'unexpected ratel-local call: %s\n' "$*" >&2
    exit 1
    ;;
esac
