#!/usr/bin/env bash
# Telegram notification channel (via openclaw)
# Args: $1=title $2=body $3=telegram_body (optional, defaults to $2)

TELEGRAM_TARGET="${CLAUDE_TELEGRAM_TARGET:-}"
[ -z "$TELEGRAM_TARGET" ] && exit 0

title="$1" telegram_body="${3:-$2}"
openclaw message send --channel telegram --target "$TELEGRAM_TARGET" --message "$title: $telegram_body" 2>/dev/null
