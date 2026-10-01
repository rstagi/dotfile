#!/bin/zsh
set -u

# Loop Engineering — PR review pipeline shape. Turns a review phase's tier + round count into
# the ordered list of runs the supervisor executes (see loop-protocol.md § PR review phase
# pipeline). Mechanism only: no spawning, no GitHub.
#
# Usage:
#   loop-review.sh stages [--tier shallow|medium|max] [--rounds N] [--models-conf <f>]
#     → one TSV line per run: a<k> \t <stage> \t <chain>
#
# medium/max: each round runs two independent adversaries (review-adv-a, review-adv-b); every
# round but the last is followed by review-fix; the last round is followed by review-final.
# shallow: one reviewer (review-adv-a) + review-fix per round but the last; the last round is
# review-final alone (the final run reviews the code itself).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MODELS_CONF="$SCRIPT_DIR/loop-models.conf"
TIER="" ROUNDS=""

die() { echo "loop-review: $*" >&2; exit 1; }

CMD="${1:-}"; shift 2>/dev/null || true
[[ "$CMD" == stages ]] || die "usage: loop-review.sh stages [--tier T] [--rounds N] [--models-conf <f>]"
while [[ $# -gt 0 ]]; do
  case "$1" in
  --tier) TIER="$2"; shift 2 ;;
  --rounds) ROUNDS="$2"; shift 2 ;;
  --models-conf) MODELS_CONF="$2"; shift 2 ;;
  *) die "unknown arg $1" ;;
  esac
done

[[ -r "$MODELS_CONF" ]] && source "$MODELS_CONF"
TIER="${TIER:-${LOOP_REVIEW_TIER_DEFAULT:-medium}}"
case "$TIER" in
shallow) default_rounds="${LOOP_REVIEW_ROUNDS_SHALLOW:-1}" ;;
medium) default_rounds="${LOOP_REVIEW_ROUNDS_MEDIUM:-3}" ;;
max) default_rounds="${LOOP_REVIEW_ROUNDS_MAX:-3}" ;;
*) die "invalid tier '$TIER' (shallow|medium|max)" ;;
esac
ROUNDS="${ROUNDS:-$default_rounds}"
cap="${LOOP_REVIEW_ROUNDS_CAP:-5}"
[[ "$ROUNDS" =~ '^[0-9]+$' ]] && (( ROUNDS >= 1 && ROUNDS <= cap )) \
  || die "invalid rounds '$ROUNDS' (1..$cap)"

k=0
row() { k=$(( k + 1 )); printf 'a%d\t%s\t%s\n' "$k" "$1" "$2"; }
for (( r = 1; r <= ROUNDS; r++ )); do
  if [[ "$TIER" == shallow ]]; then
    (( r == ROUNDS )) && break
    row "round$r" review-adv-a
  else
    row "round$r" review-adv-a
    row "round$r" review-adv-b
  fi
  (( r < ROUNDS )) && row "fix$r" review-fix
done
row final review-final
