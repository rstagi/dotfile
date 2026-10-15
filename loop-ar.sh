#!/bin/zsh
# AR shared library. Sourced; no runners, edits, or GitHub operations.
#
# Frozen ledger contract (Phases 4–9): one row per completed run, six TSV columns:
#   a<k>  stage  role  provider  verdict  run_dir
# PR ledger: runs/review-p<N>-<owner--repo>.ledger.tsv.
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
# A DISPUTED row records the round's <ledger-stem>.disputed.md sidecar. Replay uses
# that row, so an earlier round's sidecar cannot taint later rounds.

typeset -g _LOOP_AR_ROOT="${${(%):-%x}:A:h}"

# ar_next --rounds N --inner M --ledger F [--start-round r] [--single-round].
ar_next() {
  emulate -L zsh
  local rounds="" inner="" ledger="" start=1 single=0 round reviewer critic third decision
  local -a providers
  local -A ar_state
  while (( $# )); do
    case "$1" in
      --rounds|--inner|--ledger|--start-round)
        (( $# >= 2 )) || return 1
        case "$1" in
          --rounds) rounds="$2" ;; --inner) inner="$2" ;;
          --ledger) ledger="$2" ;; --start-round) start="$2" ;;
        esac
        shift 2 ;;
      --single-round) single=1; shift ;;
      *) return 1 ;;
    esac
  done
  _ar_count "$rounds" && _ar_count "$inner" && _ar_count "$start" \
    && [[ -n "$ledger" ]] && (( start <= rounds )) || return 1
  for (( round = start; round <= rounds; round++ )); do
    _ar_load_round "$ledger" "$round" || return 1
    [[ "${ar_state[final]}" == 1 ]] && { print -r -- done; return 0; }
    if [[ "${ar_state[fixed]}" == 1 ]]; then
      (( single )) && { print -r -- done; return 0; }
      continue
    fi
    providers=(${(ps:\t:)"$(ar_rotation "$round")"})
    reviewer="$providers[1]"; critic="$providers[2]"; third="$providers[3]"
    # A veto keeps T in the critic slot for every remaining inner exchange.
    if [[ "${ar_state[confirm]}" != - && "${ar_state[confirm]}" != AGREE ]]; then critic="$third"; fi
    case "${ar_state[role]}" in
      -) printf 'round%d\treview\t%s\n' "$round" "$reviewer"; return 0 ;;
      review|respond) printf 'round%d\tcritic\t%s\n' "$round" "$critic"; return 0 ;;
      critic)
        if [[ "${ar_state[first]}" == APPROVE && "${ar_state[critic]}" == AGREE \
          && "${ar_state[count]}" == 1 && "${ar_state[confirm]}" == - ]]; then
          printf 'round%d\tconfirm\t%s\n' "$round" "$third"; return 0
        fi
        decision="${ar_state[critic]}" ;;
      confirm)
        if [[ "${ar_state[confirm]}" == AGREE ]]; then
          (( single )) && print -r -- done || printf 'final\tfinal\tclaude\n'
          return 0
        fi
        decision="${ar_state[confirm]}" ;;
    esac
    if [[ "${ar_state[role]}" == critic || "${ar_state[role]}" == confirm ]] && [[ "$decision" != AGREE ]]; then
      if (( ${ar_state[count]} < inner )); then
        printf 'round%d\trespond\t%s\n' "$round" "$reviewer"
      else
        printf 'disputed\tdisputed\t-\n'
      fi
      return 0
    fi
    (( single )) && { print -r -- done; return 0; }
    if (( round < rounds )) && [[ "${ar_state[last]}" == NEEDS_CHANGES || "${ar_state[disputed]}" == 1 ]]; then
      printf 'fix%d\tfix\tclaude\n' "$round"; return 0
    fi
  done
  (( single )) && print -r -- done || printf 'final\tfinal\tclaude\n'
}

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

# ar_round_status --ledger F --round r → clean|findings|disputed.
ar_round_status() {
  emulate -L zsh
  local ledger="" round=""
  local -A ar_state
  while (( $# )); do
    (( $# >= 2 )) || return 1
    case "$1" in
      --ledger) ledger="$2" ;; --round) round="$2" ;; *) return 1 ;;
    esac
    shift 2
  done
  [[ -n "$ledger" ]] && _ar_count "$round" || return 1
  _ar_load_round "$ledger" "$round" || return 1
  if [[ "${ar_state[disputed]}" == 1 ]]; then
    print -r -- disputed
  elif [[ "${ar_state[last]}" == NEEDS_CHANGES \
    || ( "${ar_state[confirm]}" != - && "${ar_state[confirm]}" != AGREE ) ]]; then
    print -r -- findings
  else
    print -r -- clean
  fi
}

# ar_prompt --role review|critic|respond|confirm --framing F --ledger F --run-dir D.
# Print the vendored source unchanged, then bind its slots to opaque caller data.
ar_prompt() {
  emulate -L zsh
  local role="" framing="" ledger="" run_dir="" section line report i
  local -a columns reports labels
  local -A ar_state
  local prompts="$_LOOP_AR_ROOT/.claude/skills/loop-execute/references/ar-prompts.md"
  while (( $# )); do
    (( $# >= 2 )) || return 1
    case "$1" in
      --role) role="$2" ;; --framing) framing="$2" ;;
      --ledger) ledger="$2" ;; --run-dir) run_dir="$2" ;; *) return 1 ;;
    esac
    shift 2
  done
  [[ -r "$prompts" && -f "$framing" && -r "$framing" && -n "$ledger" && -n "$run_dir" ]] || return 1
  case "$role" in
    review|respond) section=A.2.3 ;;
    critic|confirm) section=A.2.4 ;;
    *) return 1 ;;
  esac
  _ar_load_round "$ledger" 1 || return 1
  if [[ -f "$ledger" ]]; then
    while IFS= read -r line || [[ -n "$line" ]]; do
      columns=(${(ps:\t:)line})
      case "$columns[3]" in
        review|respond|critic|confirm)
          report="${ledger:a:h:h}/${columns[6]}/report.md"
          [[ -f "$report" && -r "$report" ]] || return 1
          reports+=("$report")
          labels+=("${columns[1]} | ${columns[2]} | ${columns[3]} | ${columns[4]}") ;;
      esac
    done < "$ledger"
  fi
  cat -- "$prompts" || return 1
  printf '\n## Runtime bindings\nRole: %s (%s)\nRUN_DIR=%s\n' "$role" "$section" "${run_dir:a}"
  printf '\n## Caller framing (verbatim)\n'
  cat -- "$framing" || return 1
  printf '\n## Prior exchange (ledger order)\n'
  for (( i = 1; i <= ${#reports}; i++ )); do
    printf '\n### %s\n\n' "$labels[i]"
    cat -- "$reports[i]" || return 1
    printf '\n'
  done
}

_ar_count() { [[ "$1" == [1-5] ]]; }

# Populate the caller's local ar_state map; no state survives a public call.
_ar_load_round() {
  local ledger="$1" target="$2" line active=0 stage role verdict
  local -a columns
  ar_state=(first - last - role - critic - confirm - count 0 disputed 0 fixed 0 final 0)
  [[ -e "$ledger" ]] || return 0
  [[ -f "$ledger" && -r "$ledger" ]] || return 1
  while IFS= read -r line || [[ -n "$line" ]]; do
    columns=(${(ps:\t:)line})
    (( ${#columns} == 6 )) || return 1
    _ar_valid_row "${columns[@]}" || return 1
    stage="$columns[2]"; role="$columns[3]"; verdict="$columns[5]"
    [[ "$stage" == final && "$role" == final ]] && ar_state[final]=1
    if [[ "$stage" == round[1-5] ]]; then active="${stage#round}"; fi
    (( active == target )) || continue
    case "$role" in
      review|respond)
        [[ "${ar_state[first]}" == - ]] && ar_state[first]="$verdict"
        ar_state[last]="$verdict"
        ar_state[count]=$(( ${ar_state[count]} + 1 )) ;;
      critic) ar_state[critic]="$verdict" ;;
      confirm) ar_state[confirm]="$verdict" ;;
      disputed) ar_state[disputed]=1 ;;
      fix) ar_state[fixed]=1; continue ;;
      final) continue ;;
    esac
    ar_state[role]="$role"
  done < "$ledger"
  return 0
}

_ar_valid_row() {
  local attempt="$1" stage="$2" role="$3" provider="$4" verdict="$5" run_dir="$6"
  [[ "$attempt" =~ '^a[1-9][0-9]*$' && -n "$run_dir" && "$run_dir" != /* ]] || return 1
  case "$stage:$role:$verdict" in
    round[1-5]:(review|respond):(APPROVE|NEEDS_CHANGES)|round[1-5]:(critic|confirm):(AGREE|DISAGREE_EVIDENCE|DISAGREE_CONCERN))
      [[ "$provider" == gpt || "$provider" == claude || "$provider" == grok ]] ;;
    disputed:disputed:DISPUTED) [[ "$provider" == - ]] ;;
    fix[1-5]:fix:(PUSHED|NOOP)|final:final:(REQUEST_CHANGES|COMMENT|APPROVE)) [[ "$provider" == claude ]] ;;
    *) return 1 ;;
  esac
}
