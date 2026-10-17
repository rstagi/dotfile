#!/bin/zsh
# Capture message errors from the zsh sandbox.
set -e
DEST="${0:A:h}"
source "$DEST/../../../../../ws/tests/helpers.sh"
source "$WS"
mkdir -p "$WS_SESSIONS/1"
print '{"id":"1","status":"active"}' > "$WS_SESSIONS/1/meta"
"$WS" send absent hello 2> "$DEST/message-missing.txt" && exit 1
"$WS" send 1 '' 2> "$DEST/message-empty.txt" && exit 1
exit 0
