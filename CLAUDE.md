# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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
shellcheck install.sh afk-ralph.sh

# Syntax-check zsh scripts
zsh -n ralph-agent.sh ralph-source-github.sh ralph-source-linear.sh
zsh -n loop-emit.sh loop-runner.sh loop-merge.sh loop-notify.sh loop-state.sh loop-orchestrator.sh loop-plan.sh loop-plan-review.sh loop-repo.sh loop-milestone.sh loop-review.sh loop-ar.sh ws/ws.sh ws/lib/*.sh ws/tests/*.sh

# Loop engineering shell tests (occupancy + orchestrator + loop-top + ws suite, run concurrently; also runs zsh -n over the loop-*.sh and ws files)
zsh tests/run.sh

# AR review pipeline ledger only
zsh tests/review-stages.test.sh

# AR plan review rounds only
zsh tests/plan-review.test.sh

# loop-top only
node --test tests/loop-top.test.mjs

# visual-explainer renderer (schemas, escaping, both modes)
node --test tests/visual-explainer.test.mjs

# ws (terminal workspaces) tests: 4 groups in parallel; one group: zsh ws/tests/run.sh core|send|triggers|misc
zsh ws/tests/run.sh

# Loop Observatory (loop-web) tests + build
cd loop-web && npm test && npm run build && cd ..

# Rust daemon and CLI tests + lint
(cd wsd && cargo test && cargo clippy --all-targets -- -D warnings)

# S1 transcript relocation tests
python3 -I docs/ws-v6/spikes/s1/relocate-claude.test.py
python3 -I docs/ws-v6/spikes/s1/relocate-codex.test.py

# S2 telemetry capture tests
python3 -I -B -m unittest discover -s docs/ws-v6/spikes/s2 -p 'test_*.py'
```

## Architecture

```
install.sh              Main installer - 25 packages, dependency resolution
install-agent-skills.sh Shares Loop, ws, asd-ste100 + visual-explainer skills across Claude Code, Codex, and Ratel Local
ralph-agent.sh          Autonomous Claude iterations (GitHub + Linear)
ralph-source-github.sh  GitHub source adapter for Ralph
ralph-source-linear.sh  Linear source adapter for Ralph
afk-ralph.sh            Legacy Docker sandbox mode
loop-runner.sh          Loop engineering: one headless phase attempt w/ model-fallback chain
loop-orchestrator.sh    Loop engineering: two-tier — sequential SUB respawner (self-recycling orchestrator)
loop-merge.sh           Loop engineering: lane→integration merges (conflict = exit 2)
loop-notify.sh          Loop engineering: notification fan-out (osascript, opt-in Telegram)
loop-state.sh           Loop engineering: state/lock/journal/daemon + note/notes steering ops
loop-plan.sh            Loop engineering: register/push a plan on the daemon + progress notes (the only plan backend)
loop-repo.sh            Loop engineering: global repo mapping + fresh-remote integration bootstrap (or adopt existing branch)
loop-milestone.sh       Loop engineering: milestone merge gate — check gate PR states, advance (prepare next milestone, print state patch)
loop-ar.sh              Loop engineering: shared AR verdicts, rotation, state machine, prompts, disputed reports (sourced)
loop-review.sh          Loop engineering: ledger-driven PR review — next/record/recover/prompt (a<k>, stage, role, chain)
loop-plan-review.sh     Loop engineering: one rotating AR reviewer ↔ critic round on a frozen draft plan
loop-emit.sh            Loop engineering: best-effort event push to the loop-web daemon (sourced)
loop-models.conf        Loop engineering: model chains, budgets, timeouts
loop-top.mjs            Loop engineering: terminal dashboard (cwd → loop inference, live phase status, notes/pause/model controls)
loop-web.sh             Loop Observatory launcher (--daemon = central observer on :7717)
loop-web/               Loop Observatory: zero-dep Node daemon + Vite/React graph UI
ws/ws.sh                Terminal workspace dispatcher: Rust by default, explicit zsh fallback (see ws/README.md)
ws/lib/                 Embedded fzf picker UI and legacy zsh backend modules
ws/tests/               ws suite: helpers.sh (sandbox) + area *.test.sh, sourced in order per parallel group by run.sh
wsd/                    Default ws implementation: ws-core durable data, ws-daemon runtime, ws CLI + MCP
docs/ws-v6/             WS v6 architecture and spike reports, scripts, tests, and samples
.claude/skills/ws-sessions/ Agent guide to ws session discovery, messaging, completion and handoffs
.claude/skills/visual-explainer/ Props JSON → one fixed, animated, self-contained HTML page (plan|ship), zero-dep render.mjs + schemas
.zshrc                  Main shell config, sources extensions
.zshrc_*_ext            Modular configs (git, python, node, terraform, docker, gcloud, k8s, vim, ralph, ws)
~/.zshrc_ext            User's local overrides (created by install.sh, not in repo)
```

**Terminal workspaces (`ws`):** `ws/ws.sh` selects Rust when `$WS_HOME/backend` is absent and the Rust binary exists. Without the binary, it selects zsh.
It executes `${WS_RUST_BIN:-$WS_HOME/bin/ws-rs}`. Rust state uses `$WS_STATE` (default `~/.ws/state`).
The daemon socket uses `$WS_SOCK` (default `~/.ws/wsd.sock`).
The first daemon start imports existing `$WS_HOME/sessions` when Rust has no session logs.
The journal resumes interrupted imports. A completed startup import does not repeat.
The import includes pending messages, pending triggers, listed branch workspaces and pins.

Sessions use ULIDs and local `#N` aliases. Quote aliases in shell commands, for example `ws show '#123'`.
Agents receive the same ULID in `WS_SESSION_ID` and the compatibility variable `WS_SESSION`.
`ws whoami` checks these variables in that order, then the tmux window identity.
`ws sessions`, `ws show` and `ws list` support `--json`.
`ws send` queues durable messages and returns 75. A message to a finished session reopens it.
`ws inbox` lists unread messages. `ws ack [<event_id>...]` marks selected messages, or all visible messages, as read.
`ws done [--output <file>|-]` publishes completion. `ws wait <target> --for done|exit` waits through the daemon.
`ws mcp` serves local stdio MCP tools. `ws session create --headless … -- <cmd>` starts a supervised process.
`ws backend zsh` restores the legacy implementation and reports unreachable Rust-only sessions.
`ws backend rust` returns to Rust and imports new legacy records. Both backends share the alias counter.
The zsh suite selects zsh in each sandbox. `cargo test -p ws --test backend` also tests Rust through the dispatcher.

**Extension system:** install.sh appends `source ~/dotfile/.zshrc_<tool>_ext` lines to `~/.zshrc_ext`. Main `.zshrc` sources that file if it exists.

**Dependency resolution:** Some packages auto-install deps (ralph→node, kubectl→gcloud, docker→gcloud, python→pyenv+pipx, ws→tmux+fzf+neovim).

**Loop engineering:** `.claude/skills/loop-execute` orchestrates a plan end-to-end across repositories: one repository per phase and one integration branch/worktree/PR per repository. Explicit PR-review phases use Adversarial Review (AR): a read-only reviewer R reviews a frozen artifact, then a critic C from another provider audits that review. Disagreements require code evidence; each response and critic call is a fresh runner with the prior exchange inlined. Only the fixer edits code, between outer rounds. Tiers are `[review: standard|max]` (default standard): standard uses Sol / Opus / Grok; max uses Astra / Fable / Grok. `[rounds: N]` and `[inner: N]` both default to 2 and accept 1..5. Provider roles rotate GPT→Claude, Claude→Grok, Grok→GPT across outer rounds, with the remaining provider T confirming a first-review `APPROVE` + critic `AGREE`. T's `AGREE` skips all later rounds and goes straight to final; a veto makes T the critic for the remaining inner exchanges. Reaching the inner cap without agreement writes `.disputed.md`; disputed items must be checked against code and never count as clean. Before the last outer round, Opus fixes run iff the reviewer ends with `NEEDS_CHANGES` or the round is disputed. An Opus final reviewer alone posts the GitHub review (`REQUEST_CHANGES`, `COMMENT`, or `APPROVE`, based on unresolved severity).

`loop-ar.sh` is the shared protocol core; `loop-review.sh next/record/recover/prompt` drives a per-repository `runs/review-p<N>-<owner--repo>.ledger.tsv`. Each completed run has six TSV columns: `a<k>`, stage, role, provider, verdict token, and run directory relative to `.loop/`. Roles are `review|critic|respond|confirm|disputed|fix|final`; no skipped rows are written. Resume recovers finished-but-unrecorded runs, reconciles the GitHub final marker, then asks for `next`. Effort and provider chains are in `loop-models.conf`. `multiphase-plan` uses the same AR protocol before registering: `loop-plan-review.sh run --round r` runs one independent round; the planner verifies the converged review and disputed items, edits the plan between rounds, and stops early on clean. `LOOP_PLAN_REVIEW_ROUNDS` defaults to 2; `light <goal>` skips plan review.

GitHub slugs stay in plans; reusable checkout mappings live in `~/.loop/repos.json` through `loop-repo.sh`. The central **Loop daemon** + `.loop/plan.md` are the only plan backend. On-disk coordinator state stays in flattened `.loop/`; review notes use `notes/<reviewPhase>.md` (legacy `notes/pr-review.*` aliases remain accepted). Contract: `.claude/skills/loop-execute/references/loop-protocol.md`; vendored prompts: `references/ar-prompts.md` beside it. Two-tier mode keeps scheduling/merges in recyclable SUBs and review-phase pipelines in detached runners. Shell tests live in `tests/` (`zsh tests/run.sh`).

**Milestones:** one Loop plan may carry several `## Milestone N — <outcome>` sections (plain `## Phases` = one implicit milestone 1). Each ships as its own PR per participating repository and ends with a `[kind: pr-review]` `[gate: merge]` phase. After that review's verdict is posted the loop parks in `awaiting-merge` (plan Status `awaiting merge`, typed `milestone.awaiting-merge` event, notification with the PR URLs); it never finishes or respawns. The human assesses and merges, then runs `/loop-execute resume`: `loop-milestone.sh check` confirms the gate PRs merged, `advance` prepares the next milestone's worktrees from a freshly fetched default branch, and the orchestrator respawns. Milestone ≥2 integration branches are declared in the plan; worktrees live at `…/<owner--repo>/integration-<branch-slug>-<hash8>` (8-char sha1 of the exact branch). `loop-pickup`/`loop-handoff` never touch phases of a later milestone than `state.milestone`. Format: `.claude/skills/multiphase-plan/references/plan-format.md` → Milestones. After pulling these changes, restart the Loop daemon (`awaiting-merge` status + milestone events are new): `launchctl kickstart -k gui/$(id -u)/com.rstagi.loop-web`; run `./loop-web.sh --daemon` only when no LaunchAgent is bound to :7717.

**Visual explainers:** `.claude/skills/visual-explainer` renders one animated HTML page from a props JSON file. The agent writes only the props. `render.mjs` checks them against `schema/<plan|ship>.schema.json`, escapes all text, and fills the fixed template. Mode `plan` runs after `multiphase-plan` registers a plan, except for `light` plans. It also runs at each milestone resume. Mode `ship` runs at each milestone merge gate and at the Loop finish, before the notification. The finish page explains the last milestone, or the whole plan when the plan has no milestones. Mode `ship` also runs in `pr-review` Step 4. Each `ship` hand-test command needs a `path:line` code reference. In a Loop run, a detached runner writes the props, and supervise renders, publishes and links the page. A failed explainer never blocks a park, a finish or a resume. Loop output goes to `.loop/explainers/plan/` and `.loop/explainers/m<n>-<plan|ship>/`.

**AR rollout:** after pulling, restart the daemon with `launchctl kickstart -k gui/$(id -u)/com.rstagi.loop-web`, then check `curl -fsS http://localhost:7717/api/health`. Retag any active plan's review phase using a legacy tier to `[review: standard]` or `[review: max]` before execution. Legacy shallow and medium tiers are rejected by execution paths and the tier endpoint; the Observatory and `loop-top` still render them for historical loops.

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
