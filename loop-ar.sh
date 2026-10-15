#!/bin/zsh
# AR shared library. Sourced; no runners, edits, or GitHub operations.
#
# Frozen ledger contract (Phases 4–9): one row per completed run, six TSV columns:
#   a<k>  stage  role  provider  verdict  run_dir
# run_dir is relative to the loop directory; a<k> is the idempotency key.
# Roles / token-only verdicts:
#   review|respond: APPROVE|NEEDS_CHANGES
#   critic|confirm: AGREE|DISAGREE_EVIDENCE|DISAGREE_CONCERN
#   disputed: DISPUTED; fix: PUSHED|NOOP
#   final: REQUEST_CHANGES|COMMENT|APPROVE
# Stages: round<r> for review/critic/respond/confirm; disputed for the dispute
# marker (belongs to the preceding round); fix<r> for fix; final for final.
# Providers: gpt|claude|grok; fix/final use claude, disputed uses "-" (no runner).
# ar_next emits stage<TAB>role<TAB>provider, or the terminal sentinel "done".
# Completed rows are replayed in file order. No rows are written for skipped rounds.

# ar_parse_verdict <report> reviewer|critic → token; invalid → "invalid", exit 2.
ar_parse_verdict() {
  emulate -L zsh
  local report="${1:-}" kind="${2:-}" line last="" token summary
  if [[ -r "$report" ]]; then
    while IFS= read -r line || [[ -n "$line" ]]; do
      [[ "$line" == *[^[:space:]]* ]] && last="$line"
    done < "$report"
  fi
  token="${last%%:*}"
  case "$kind:$token" in
    reviewer:APPROVE|critic:AGREE)
      [[ "$last" == "$token" ]] && { print -r -- "$token"; return 0; } ;;
    reviewer:NEEDS_CHANGES|critic:DISAGREE_EVIDENCE|critic:DISAGREE_CONCERN)
      summary="${last#*: }"
      [[ "$last" == "$token: "* && "$summary" == *[^[:space:]]* ]] \
        && { print -r -- "$token"; return 0; } ;;
  esac
  print -r -- invalid
  return 2
}

# ar_rotation <round> → R<TAB>C<TAB>T provider names.
ar_rotation() {
  emulate -L zsh
  local round="${1:-}"; local -a providers=(gpt claude grok)
  [[ "$round" =~ '^[1-9][0-9]*$' ]] || return 1
  printf '%s\t%s\t%s\n' "${providers[$(( (round - 1) % 3 + 1 ))]}" \
    "${providers[$(( round % 3 + 1 ))]}" "${providers[$(( (round + 1) % 3 + 1 ))]}"
}
