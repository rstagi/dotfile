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
shellcheck install.sh afk-ralph.sh

# Syntax-check zsh scripts
zsh -n ralph-agent.sh ralph-source-github.sh ralph-source-linear.sh
zsh -n loop-runner.sh loop-merge.sh loop-notify.sh loop-state.sh
```

## Architecture

```
install.sh              Main installer - 25 packages, dependency resolution
ralph-agent.sh          Autonomous Claude iterations (GitHub + Linear)
ralph-source-github.sh  GitHub source adapter for Ralph
ralph-source-linear.sh  Linear source adapter for Ralph
afk-ralph.sh            Legacy Docker sandbox mode
loop-runner.sh          Loop engineering: one headless phase attempt w/ model-fallback chain
loop-merge.sh           Loop engineering: lane→integration merges (conflict = exit 2)
loop-notify.sh          Loop engineering: notification fan-out (osascript, opt-in Telegram)
loop-state.sh           Loop engineering: state.json ops, run lock, event journal
loop-models.conf        Loop engineering: model chains, budgets, timeouts
.zshrc                  Main shell config, sources extensions
.zshrc_*_ext            Modular configs (git, python, node, terraform, docker, gcloud, k8s, vim, ralph)
~/.zshrc_ext            User's local overrides (created by install.sh, not in repo)
```

**Extension system:** install.sh appends `source ~/dotfile/.zshrc_<tool>_ext` lines to `~/.zshrc_ext`. Main `.zshrc` sources that file if it exists.

**Dependency resolution:** Some packages auto-install deps (ralph→node, kubectl→gcloud, docker→gcloud, python→pyenv+pipx).

**Loop engineering:** `.claude/skills/kestral-loop` orchestrates a published multiphase-plan end-to-end (headless runners per phase, single integration branch + PR, escalation → HIL, auto pr-review). Contract: `.claude/skills/kestral-loop/references/loop-protocol.md`; mechanism: the `loop-*.sh` scripts above.

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
load_secret VAR_NAME "op://Remote Agents/Item Name/credential"
```

Skips if var already set. Set `DISABLE_SECRETS=1` to skip all 1Password calls.

