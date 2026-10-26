#!/bin/zsh
# Read the private loop-web fixture store. Never make a network request.
url="${@[-1]}"
print -r -- "$url" >> "$FAKE_PLAN_WEB/calls.log"
[[ "${FAKE_PLAN_WEB_FAIL:-0}" == 0 ]] || exit 22
case "$url" in
*/api/loops)
  files=("$FAKE_PLAN_WEB"/*.json(N))
  if (( ${#files} )); then jq -s '.' "${files[@]}"; else print '[]'; fi ;;
*/api/loops/*/plan)
  id="${${url%/plan}##*/}"
  [[ -f "$FAKE_PLAN_WEB/$id.json" ]] || exit 22
  cat "$FAKE_PLAN_WEB/$id.json" ;;
*) exit 22 ;;
esac
