#!/bin/bash
# Loop Observatory launcher — a read-only live graph for the Kestral loop-engineering flow.
#
# Usage:
#   loop-web.sh [--dir <.kestral/loop path>] [--plan <plan.md>] [--port 7717]
#
# Discovery (when --dir is omitted): $PWD/.kestral, else walk up for the nearest .kestral/.
# The observer only reads files — it never writes to .kestral/loop or the plan.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER="$SCRIPT_DIR/loop-web/server/index.mjs"

usage() {
  cat <<'EOF'
loop-web — Loop Observatory: a live read-only graph for the Kestral loop-engineering flow.

Usage:
  loop-web [--dir <.kestral/loop path>] [--plan <plan.md>] [--port 7717]

When --dir/--plan are omitted, the nearest ancestor with a .kestral/ directory is used.
The observer only reads files — it never writes to .kestral/loop or the plan.
EOF
}
need_arg() { [ "$#" -ge 2 ] || { echo "loop-web: $1 requires a value" >&2; exit 1; }; }

DIR="" PLAN="" PORT="${LOOP_WEB_PORT:-7717}"
while [ $# -gt 0 ]; do
  case "$1" in
    --dir) need_arg "$@"; DIR="$2"; shift 2 ;;
    --plan) need_arg "$@"; PLAN="$2"; shift 2 ;;
    --port) need_arg "$@"; PORT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "loop-web: unknown arg '$1'" >&2; usage; exit 1 ;;
  esac
done

command -v node >/dev/null 2>&1 || {
  echo "loop-web: node is required (install with: ./install.sh loop-web)" >&2; exit 1
}
# The server imports the .ts model via Node's native type-stripping (needs Node >= 22.18).
node -e 'const [a,b]=process.versions.node.split(".").map(Number); process.exit((a>22||(a===22&&b>=18))?0:1)' 2>/dev/null || {
  echo "loop-web: Node >= 22.18 required (have $(node -v)) for the .ts server — upgrade node" >&2; exit 1
}
[ -f "$SERVER" ] || { echo "loop-web: server not found at $SERVER" >&2; exit 1; }

# Discover the loop dir if not given: nearest ancestor with a .kestral/ directory.
if [ -z "$DIR" ] && [ -z "$PLAN" ]; then
  d="$PWD"
  while :; do
    if [ -d "$d/.kestral" ]; then DIR="$d/.kestral/loop"; break; fi
    [ "$d" = "/" ] && break
    d="$(dirname "$d")"
  done
fi

if [ ! -d "$SCRIPT_DIR/loop-web/dist" ]; then
  echo "loop-web: UI not built — run 'npm run build' in $SCRIPT_DIR/loop-web (data endpoints still work)" >&2
fi

export LOOP_WEB_PORT="$PORT"
[ -n "$DIR" ] && export LOOP_WEB_DIR="$DIR"
[ -n "$PLAN" ] && export LOOP_WEB_PLAN="$PLAN"

echo "loop-web: starting Loop Observatory on http://localhost:$PORT"
exec node "$SERVER"
