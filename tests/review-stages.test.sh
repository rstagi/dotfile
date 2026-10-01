#!/bin/zsh
# loop-review.sh stages — tier/rounds → ordered review runs (a<k> \t stage \t chain).
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
CONF="$TMP/models.conf"
cat > "$CONF" <<'CONF'
LOOP_REVIEW_TIER_DEFAULT=medium
LOOP_REVIEW_ROUNDS_SHALLOW=1
LOOP_REVIEW_ROUNDS_MEDIUM=3
LOOP_REVIEW_ROUNDS_MAX=3
LOOP_REVIEW_ROUNDS_CAP=5
CONF

stages() { zsh "$ROOT/loop-review.sh" stages --models-conf "$CONF" "$@" 2>"$TMP/err"; }
T=$'\t'

echo "review-stages: medium 3 is today's nine-run pipeline"
assert_eq "$(stages --tier medium --rounds 3)" "a1${T}round1${T}review-adv-a
a2${T}round1${T}review-adv-b
a3${T}fix1${T}review-fix
a4${T}round2${T}review-adv-a
a5${T}round2${T}review-adv-b
a6${T}fix2${T}review-fix
a7${T}round3${T}review-adv-a
a8${T}round3${T}review-adv-b
a9${T}final${T}review-final" "medium 3 → 9 runs"

echo "review-stages: medium 1 is one adversarial pair + verdict"
assert_eq "$(stages --tier medium --rounds 1)" "a1${T}round1${T}review-adv-a
a2${T}round1${T}review-adv-b
a3${T}final${T}review-final" "medium 1 → 3 runs"

echo "review-stages: max 2"
assert_eq "$(stages --tier max --rounds 2 | cut -f2,3 | tr '\t\n' ' |')" \
  "round1 review-adv-a|round1 review-adv-b|fix1 review-fix|round2 review-adv-a|round2 review-adv-b|final review-final|" \
  "max 2 → pair, fix, pair, final"

echo "review-stages: shallow has one reviewer and no adversary in the last round"
assert_eq "$(stages --tier shallow --rounds 1)" "a1${T}final${T}review-final" "shallow 1 → the final run alone"
assert_eq "$(stages --tier shallow --rounds 3)" "a1${T}round1${T}review-adv-a
a2${T}fix1${T}review-fix
a3${T}round2${T}review-adv-a
a4${T}fix2${T}review-fix
a5${T}final${T}review-final" "shallow 3 → review/fix ×2, final"

echo "review-stages: defaults come from the models conf"
assert_eq "$(stages | wc -l | tr -d ' ')" "9" "no flags → medium 3"
assert_eq "$(stages --tier shallow | wc -l | tr -d ' ')" "1" "shallow without rounds → its default (1)"

echo "review-stages: validation"
stages --tier huge >/dev/null; assert_exit "$?" "1" "unknown tier rejected"
assert_contains "$(cat "$TMP/err")" "tier" "says what is wrong with the tier"
stages --rounds 0 >/dev/null; assert_exit "$?" "1" "rounds 0 rejected"
stages --rounds 6 >/dev/null; assert_exit "$?" "1" "rounds above the cap rejected"
assert_contains "$(cat "$TMP/err")" "1..5" "names the allowed range"
stages --rounds 5 >/dev/null; assert_exit "$?" "0" "rounds at the cap accepted"

test_summary
