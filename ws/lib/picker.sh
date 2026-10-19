#!/bin/zsh
# Share the picker UI with Rust. Load the legacy data commands for the zsh backend.
source "${${(%):-%x}:A:h}/../../wsd/crates/ws/src/cmd/picker.zsh"
source "${${(%):-%x}:A:h}/picker-legacy.sh"
