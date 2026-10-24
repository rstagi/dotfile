#!/bin/zsh
# Record the scheduling priority. macOS reports a background QoS clamp as priority 4.
[[ -n "${FAKE_PRI_LOG:-}" ]] && ps -o pri= -p $$ | tr -d ' ' >> "$FAKE_PRI_LOG"
# Record the external engine interface when an integration test requests it.
if [[ -n "${FAKE_CONTEXT_DIR:-}" ]]; then
  context_engine="$1"
  shift
  mkdir -p "$FAKE_CONTEXT_DIR"
  jq -n --arg cwd "$PWD" --arg session "${WS_SESSION_ID:-}" \
    --arg sentinel "${FAKE_SENTINEL:-}" --args \
    '{cwd:$cwd,wsSession:$session,sentinel:$sentinel,argv:$ARGS.positional}' \
    -- "$@" > "$FAKE_CONTEXT_DIR/$context_engine.json"
  cat > "$FAKE_CONTEXT_DIR/$context_engine.stdin"
  print -r -- 'fake engine stderr' >&2
fi
