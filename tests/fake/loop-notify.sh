#!/bin/zsh
# Test stub for loop-notify.sh — appends its args to $NOTIFY_LOG (no osascript/daemon).
echo "$@" >> "${NOTIFY_LOG:-/dev/null}"
exit 0
