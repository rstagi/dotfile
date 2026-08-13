#!/bin/zsh
# Run every tests/*.test.sh; exit non-zero if any file fails. Also syntax-checks the
# loop-*.sh scripts (zsh -n) first. Companion to `cd loop-web && npm test` for JS.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
fail=0

echo "== zsh -n (syntax) =="
for s in loop-state.sh loop-orchestrator.sh loop-runner.sh loop-merge.sh loop-notify.sh loop-emit.sh loop-plan.sh loop-repo.sh; do
  [[ -f "$ROOT/$s" ]] || continue
  if zsh -n "$ROOT/$s"; then echo "  ok: $s"; else echo "  FAIL: $s"; fail=1; fi
done
echo

for t in "$HERE"/*.test.sh; do
  echo "== ${t:t} =="
  zsh "$t" || fail=1
  echo
done

if [[ $fail -eq 0 ]]; then
  echo "ALL SHELL TESTS PASSED"; exit 0
fi
echo "SHELL TESTS FAILED"; exit 1
