#!/bin/zsh
# Skills engine end to end with the scripted fake orchestrator: happy, verify-fail, crash and broken.
# The real-model run is manual: zsh tests/loop-e2e/run.sh --scenario happy --model <id> (see tests/loop-e2e/RESULTS.md).
set -u
HERE="${0:A:h}"
exec zsh "$HERE/loop-e2e/run.sh" --all
