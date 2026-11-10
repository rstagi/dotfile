# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Follow `CODING_STANDARDS.md` for plans, code and reviews. A plan or a PR blocks only on security, unprotected breakage of existing behavior, or a failing deterministic check. These hard blocks are always fixed. A PR review labels each finding `[major]` or `[minor]`. The repository defer policy (`flag` or `strict`) and the owner decide in the chat which other findings the PR fixes and which it defers.

## Repository Overview

macOS dotfile repo w/ automated dev environment setup. Installs shell config, CLI tools, cloud tooling, and dev environment via homebrew.

## Key Commands

```bash
# Full interactive install
./install.sh

# Install specific packages. Only the full install runs brew upgrade.
./install.sh <package1> <package2>

# List available packages
./install.sh --list

# Preview install (no changes)
./install.sh --dry-run <packages>

# Lint shell scripts
shellcheck install.sh

# Syntax-check zsh scripts
zsh -n ws/ws.sh wsd/crates/ws/src/cmd/picker.zsh ws/tests/*.sh

# Shell tests and picker smoke test, including zsh syntax checks
zsh tests/run.sh

# visual-explainer graph + page + walkthrough logic (schemas, relation ids, layout, escaping, narration v3, screenshots)
zsh tests/visual-explainer.test.sh

# ws picker shell smoke test with a private tmux server
zsh ws/tests/run.sh

# Rust daemon and CLI tests + lint
(cd wsd && cargo test && cargo clippy --all-targets -- -D warnings)

# S1 transcript relocation tests
python3 -I docs/ws-v6/spikes/s1/relocate-claude.test.py
python3 -I docs/ws-v6/spikes/s1/relocate-codex.test.py
```

## Architecture

```
install.sh              Main installer - 25 packages, dependency resolution
install-agent-skills.sh Shares loop-do + assess, ws (sessions + orchestrate), pr-review, asd-ste100 + visual-explainer + engineering-standards skills across Claude Code, Codex, and Ratel Local
ws/ws.sh                Rust CLI launcher (see ws/README.md)
wsd/crates/ws/src/cmd/picker.zsh  Embedded fzf picker UI
ws/tests/               Picker smoke test and Rust picker integration shell checks
wsd/                    Default ws implementation: ws-core durable data, ws-daemon runtime, ws CLI + MCP
docs/ws-v6/             WS v6 architecture, semantic conventions and spike reports (S1 scripts stay)
wsd/crates/ws-daemon/tests/fixtures/spikes/  S2 telemetry and S3 quota samples
.claude/skills/ws-sessions/ Agent guide to ws session discovery, messaging, completion and handoffs
.claude/skills/ws-orchestrate/ Repo-less orchestrator: one child session per repo, ws-message coordination, PR URL summary, no repo edits
.claude/skills/visual-explainer/ Props JSON → concept graph (graph.mjs: nodes, edges, fixed layout) → one self-contained surfable HTML page (plan|ship), zero-dep render.mjs + schemas. Narration JSON (one commentary text per report section) → report walkthrough that spotlights each section (template/walkthrough.js)
.claude/skills/loop-do/ Pure skill, no scripts: read → ask → plan in chat → build (≤3 parallel worktree subagents in Claude Code) → Done-when check → one draft PR with proof → pr-review (≤2 rounds) → report
.claude/skills/assess/ Idea gate: evidence → go|clarify|kill decision with a Handoff goal
.claude/skills/engineering-standards/ One-file rules for plan, execution and review (blocking vs deferred, coding standards, TDD, tests, review rounds, human gates). Point a planner (for example Beluga advanced planning) at it. CODING_STANDARDS.md wins on conflict
.zshrc                  Main shell config, sources extensions
.zshrc_*_ext            Modular configs (git, python, node, terraform, docker, gcloud, k8s, vim, claude, ws)
~/.zshrc_ext            User's local overrides (created by install.sh, not in repo)
```

**Terminal workspaces (`ws`):** `ws/ws.sh` always runs Rust. It ignores old backend selector files.
It executes `${WS_RUST_BIN:-$WS_HOME/bin/ws-rs}`. Rust state uses `$WS_STATE` (default `~/.ws/state`).
The daemon socket uses `$WS_SOCK` (default `~/.ws/wsd.sock`).
The first daemon start imports existing `$WS_HOME/sessions` when Rust has no session logs.
The journal resumes interrupted imports. A completed startup import does not repeat.
The import includes pending messages, pending triggers, listed branch workspaces and pins.

Sessions use ULIDs and local `#N` aliases. Quote aliases in shell commands, for example `ws show '#123'`.
New agents receive their ULID in `WS_SESSION_ID` only. Launch preparation removes inherited `WS_SESSION` values.
The CLI still reads `WS_SESSION` for agents that started before the cutover.
`ws whoami` checks these variables in that order, then the tmux window identity.
`ws sessions`, `ws show` and `ws list` support `--json`.
`ws send` queues durable messages and returns 75. A message to a finished session reopens it.
`ws inbox` lists unread messages. `ws ack [<event_id>...]` marks selected messages, or all visible messages, as read.
`ws done [--output <file>|-]` publishes completion. `ws wait <target> --for done|exit` waits through the daemon.
`ws mcp` serves local stdio MCP tools. `ws session create --headless … -- <cmd>` starts a supervised process.
`ws backend rust` imports remaining legacy records. Rust is the only backend.
Alias allocation retains the legacy counter lock.
Cargo tests cover the CLI and daemon. `zsh ws/tests/run.sh` tests the embedded picker shell glue.
`cargo test -p ws --test backend` also tests Rust through the launcher.

**Extension system:** install.sh adds each `source ~/dotfile/.zshrc_<tool>_ext` line to `~/.zshrc_ext` with `append_line`. `append_line` skips a line that is already in the file. Main `.zshrc` sources that file if it exists.

**Idempotent install:** a second run of an install step changes no file. Links use `link_file` (keeps a different file and prints a warning) or `replace_with_link` (moves a different file to a new backup first). Each backup gets a unique `<path>.bak.<timestamp>` name. `tests/install-packages.test.sh` runs each package two times and compares the sandbox HOME.

**Dependency resolution:** Some packages auto-install deps (kubectl→gcloud, docker→gcloud, python→pyenv+pipx, ws→tmux+fzf+neovim).

**Visual explainers:** `.claude/skills/visual-explainer` renders one HTML page from a props JSON file: a concept graph to explore (pan/zoom, detail panel, `#node=<id>` deep links, search, kind filters, outline, tour). The agent writes only the props (optional relation fields add edges). `render.mjs` checks them against `schema/<plan|ship>.schema.json` and the relation ids, builds the graph and its fixed layout (`graph.mjs`), escapes all text, and fills the fixed template. Mode `ship` runs in `pr-review` Step 4. Each `ship` hand-test command needs a `path:line` code reference. The agent writes `narration.json` (v3: one commentary text for each report section) beside the props. `render.mjs --narration` checks it and adds a walkthrough to the report: it dims the page, puts a spotlight on one section at a time and shows the commentary in a card (`#walkthrough` or `#walkthrough-<n>` opens it). There is no video.

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
