#!/bin/zsh
# Store loop-web writes in the test sandbox. Match the skills-loop emission guard.
loop_ensure_daemon() {
  print -r -- "ensure:${WS_LOOP_ID:-unset}" >> "$EMIT_LOG"
}
loop_emit() {
  local id="$1" action="$2" body
  body="$(cat)"
  print -r -- "emit:${WS_LOOP_ID:-unset}:$id:$action" >> "$EMIT_LOG"
  [[ -z "${WS_LOOP_ID:-}" ]] || return 0
  if [[ "$action" == register ]]; then
    jq -n --arg id "$id" --argjson body "$body" \
      --rawfile text "$(jq -r .planFile <<< "$body")" \
      '{runId:$id,status:"planned",effort:$body.effort,planText:$text}' > "$FAKE_PLAN_WEB/$id.json"
  else
    print -r -- "$body" >> "$FAKE_PLAN_WEB/notes.jsonl"
  fi
}
