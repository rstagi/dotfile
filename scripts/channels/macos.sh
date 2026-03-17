#!/usr/bin/env bash
# macOS notification center channel
# Args: $1=title $2=body

_sanitize_applescript() {
  printf '%s' "$1" | sed 's/["\\]/\\&/g'
}

title="$1" body="${2:0:200}"
safe_title=$(_sanitize_applescript "$title")
safe_body=$(_sanitize_applescript "$body")
osascript -e "display notification \"$safe_body\" with title \"$safe_title\"" 2>/dev/null
