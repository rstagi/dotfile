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
zsh -n loop-emit.sh loop-runner.sh loop-merge.sh loop-notify.sh loop-state.sh loop-orchestrator.sh loop-plan.sh loop-repo.sh

# Loop engineering shell tests (occupancy + orchestrator; also runs zsh -n over the loop-*.sh)
zsh tests/run.sh

# Loop Observatory (loop-web) tests + build
cd loop-web && npm test && npm run build && cd ..
```

## Architecture

```
install.sh              Main installer - 25 packages, dependency resolution
ralph-agent.sh          Autonomous Claude iterations (GitHub + Linear)
ralph-source-github.sh  GitHub source adapter for Ralph
ralph-source-linear.sh  Linear source adapter for Ralph
afk-ralph.sh            Legacy Docker sandbox mode
loop-runner.sh          Loop engineering: one headless phase attempt w/ model-fallback chain
loop-orchestrator.sh    Loop engineering: two-tier — sequential SUB respawner (self-recycling orchestrator)
loop-merge.sh           Loop engineering: lane→integration merges (conflict = exit 2)
loop-notify.sh          Loop engineering: notification fan-out (osascript, opt-in Telegram)
loop-state.sh           Loop engineering: state/lock/journal/daemon + note/notes steering ops
loop-plan.sh            Loop engineering: register/push a plan on the daemon + progress notes (local backend, Kestral-free)
loop-repo.sh            Loop engineering: global repo mapping + fresh-remote integration bootstrap
loop-emit.sh            Loop engineering: best-effort event push to the loop-web daemon (sourced)
loop-models.conf        Loop engineering: model chains, budgets, timeouts
loop-web.sh             Loop Observatory launcher (--daemon = central observer on :7717)
loop-web/               Loop Observatory: zero-dep Node daemon + Vite/React graph UI
.zshrc                  Main shell config, sources extensions
.zshrc_*_ext            Modular configs (git, python, node, terraform, docker, gcloud, k8s, vim, ralph)
~/.zshrc_ext            User's local overrides (created by install.sh, not in repo)
```

**Extension system:** install.sh appends `source ~/dotfile/.zshrc_<tool>_ext` lines to `~/.zshrc_ext`. Main `.zshrc` sources that file if it exists.

**Dependency resolution:** Some packages auto-install deps (ralph→node, kubectl→gcloud, docker→gcloud, python→pyenv+pipx).

**Loop engineering:** `.claude/skills/loop-execute` orchestrates a plan end-to-end across repositories: one repository per phase and one integration branch/worktree/PR/review per repository. GitHub slugs stay in plans; reusable checkout mappings live in `~/.loop/repos.json` through `loop-repo.sh`. The central **Loop daemon** is always the base backend; **Kestral is opt-in**. On-disk coordinator state stays in flattened `.loop/`; repository review notes use `notes/pr-review.<owner--repo>.md`. Contract: `.claude/skills/loop-execute/references/loop-protocol.md`. Two-tier mode keeps scheduling/merges in recyclable SUBs and final repository reviews in detached runners. Shell tests live in `tests/` (`zsh tests/run.sh`).

**Loop Observatory (`loop-web`):** a perpetual central daemon (launchd LaunchAgent, `127.0.0.1:7717`) that is the base backend for every plan and renders every loop as a live L→R graph. `multiphase-plan` registers a plan on it (status `planned`) before any run; loops then register + push lifecycle events (`loop-emit.sh`/`loop-plan.sh`, sourced by/talking to the daemon) so status is authoritative via a monotone promotion lattice — never stale. Each loop is kept forever in `~/.loop/loops/<runId>.json`; a header selector switches between loops. Daemon/event contract lives in `loop-protocol.md` → Daemon & events.

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
