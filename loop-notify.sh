#!/bin/zsh
set -u -o pipefail

# Loop Engineering — notification fan-out. Best effort, always exits 0.
# Usage: loop-notify.sh --level info|question|error --title <t> --body <b> [--url <u>]

LEVEL="info" TITLE="kestral-loop" BODY="" URL=""

while [[ $# -gt 0 ]]; do
  case "$1" in
  --level) LEVEL="$2"; shift 2 ;;
  --title) TITLE="$2"; shift 2 ;;
  --body) BODY="$2"; shift 2 ;;
  --url) URL="$2"; shift 2 ;;
  *) shift ;;
  esac
done

[[ -n "$URL" ]] && BODY="$BODY
$URL"

echo "[$LEVEL] $TITLE — $BODY"

# macOS notification — data passed as argv, never interpolated into the AppleScript
# source (BODY carries LLM-generated text; interpolation = script injection)
if [[ "$LEVEL" == "info" ]]; then
  osascript -e 'on run argv' \
    -e 'display notification (item 1 of argv) with title (item 2 of argv)' \
    -e 'end run' "$BODY" "$TITLE" 2>/dev/null
else
  osascript -e 'on run argv' \
    -e 'display notification (item 1 of argv) with title (item 2 of argv) sound name "Glass"' \
    -e 'end run' "$BODY" "$TITLE" 2>/dev/null
fi

# Telegram (opt-in: both vars must be set; deliberately not wired to load_secret here)
if [[ -n "${TELEGRAM_BOT_TOKEN:-}" && -n "${CLAUDE_TELEGRAM_TARGET:-}" ]]; then
  curl -sf -m 10 "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/sendMessage" \
    -d chat_id="$CLAUDE_TELEGRAM_TARGET" \
    --data-urlencode text="[$LEVEL] $TITLE
$BODY" >/dev/null 2>&1
fi

exit 0
