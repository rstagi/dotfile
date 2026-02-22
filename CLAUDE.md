# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Overview

macOS dotfile repo w/ automated dev environment setup. Installs shell config, CLI tools, cloud tooling, and dev environment via homebrew.

## Key Commands

```bash
# Full interactive install
./install.sh

# Install specific packages
./install.sh <package1> <package2>

# List available packages
./install.sh --list

# Preview install (no changes)
./install.sh --dry-run <packages>

# Lint shell scripts
shellcheck install.sh afk-ralph.sh afk-ralph-github.sh
```

## Architecture

```
install.sh              Main installer - 25 packages, dependency resolution
afk-ralph.sh            Autonomous Claude iterations in Docker sandbox
.zshrc                  Main shell config, sources extensions
.zshrc_*_ext            Modular configs (git, python, node, terraform, docker, gcloud, k8s, vim, ralph)
~/.zshrc_ext            User's local overrides (created by install.sh, not in repo)
```

**Extension system:** install.sh appends `source ~/dotfile/.zshrc_<tool>_ext` lines to `~/.zshrc_ext`. Main `.zshrc` sources that file if it exists.

**Dependency resolution:** Some packages auto-install deps (ralph→node, kubectl→gcloud, docker→gcloud, python→pyenv+pipx).

## Git Aliases (from .zshrc_git_ext)

| Alias | Cmd |
|-------|-----|
| `gst` | status |
| `gac` | add --patch + commit |
| `gaac` | add . + commit -a |
| `gsw` | switch |
| `gswc` | switch --create |
| `gpsu` | push --set-upstream origin |
| `gpf` | push --force-with-lease |
| `gpr()` | push + open PR in browser |
| `gsq` | squash to parent branch |
| `gremreb` | fetch + rebase on parent |
| `gbparent` | detect parent branch |

## Secrets Management

Store API keys in 1Password, retrieve lazily via `load_secret` (defined in `.zshrc_claude_ext`):

```bash
load_secret VAR_NAME "op://Private/Item Name/credential"
```

Skips if var already set. Set `DISABLE_SECRETS=1` to skip all 1Password calls.

