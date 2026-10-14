#!/bin/zsh
# Runs the ws suite: helpers.sh builds one sandbox (private HOME, tmux server, fake agents,
# real-HOME guard), then each area file is sourced in this order in the same process, since
# workspaces, tabs and global session ids carry over between areas. Exits non-zero on any failure.
set -u
HERE="${0:A:h}"
source "$HERE/helpers.sh"

for area in workspace hooks tabs picker setup notify merge send lifecycle triggers inflight \
  triggers-actions trigger-edges review-fixes rm migrate; do
  echo "== ws: $area =="
  source "$HERE/$area.test.sh"
done

test_summary
