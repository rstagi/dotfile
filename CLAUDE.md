# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Follow `CODING_STANDARDS.md` for plans, code and reviews. A plan or a PR blocks only on security, unprotected breakage of existing behavior, or a failing deterministic check. All other items are deferred.

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
zsh -n loop-emit.sh loop-runner.sh loop-merge.sh loop-notify.sh loop-state.sh loop-orchestrator.sh loop-plan.sh loop-plan-review.sh loop-repo.sh loop-milestone.sh loop-review.sh loop-ar.sh ws/ws.sh wsd/crates/ws/src/cmd/picker.zsh ws/tests/*.sh

# Loop shell tests and picker smoke test, including zsh syntax checks
zsh tests/run.sh

# AR review pipeline ledger only
zsh tests/review-stages.test.sh

# AR plan review rounds only
zsh tests/plan-review.test.sh

# converge + assess skill contracts and their Loop wiring
zsh tests/converge-assess-skills.test.sh

# loop-top only
node --test tests/loop-top.test.mjs

# visual-explainer graph + page + walkthrough logic (schemas, relation ids, layout, escaping, narration v3, screenshots)
zsh tests/visual-explainer.test.sh

# loop-retro milestone facts collector
node --test tests/loop-retro.test.mjs

# ws picker shell smoke test with a private tmux server
zsh ws/tests/run.sh

# Loop daemon (loop-web) model and server tests
(cd loop-web && npm test)

# Rust daemon and CLI tests + lint
(cd wsd && cargo test && cargo clippy --all-targets -- -D warnings)

# S1 transcript relocation tests
python3 -I docs/ws-v6/spikes/s1/relocate-claude.test.py
python3 -I docs/ws-v6/spikes/s1/relocate-codex.test.py
```

## Architecture

```
install.sh              Main installer - 25 packages, dependency resolution
install-agent-skills.sh Shares Loop (incl. assess + converge), ws (sessions + orchestrate), asd-ste100 + visual-explainer skills across Claude Code, Codex, and Ratel Local
loop-runner.sh          Loop engineering: one headless phase attempt w/ model-fallback chain
loop-orchestrator.sh    Loop engineering: two-tier — sequential SUB respawner (self-recycling orchestrator)
loop-merge.sh           Loop engineering: lane→integration merges (conflict = exit 2)
loop-notify.sh          Loop engineering: notification fan-out (osascript, opt-in Telegram)
loop-state.sh           Loop engineering: state/lock/journal/daemon + note/notes steering ops
loop-plan.sh            Loop engineering: register/push a plan on the daemon + progress notes (the only plan backend) + archive a shipped milestone
loop-plan-archive.mjs   Loop engineering: pure milestone-archive transform for `loop-plan.sh archive` (imports loop-web's TS parser, Node ≥22.18); puts a pending `milestones/m<N>.retro.md` first
loop-retro.mjs          Loop engineering: milestone retrospective facts collector (attempts, setback candidates, totals; facts only, zero-dep, Node ≥20)
loop-repo.sh            Loop engineering: global repo mapping + fresh-remote integration bootstrap (or adopt existing branch)
loop-milestone.sh       Loop engineering: milestone merge gate — open (skills loop), park (skills loop at the gate), check gate PR states, advance (prepare next milestone, print state patch)
loop-plan-park.mjs      Loop engineering: pure plan transform for `loop-milestone.sh park` (done statuses, PR lines, Status `awaiting merge`; imports `loop-plan-model/park-milestone.ts`, Node ≥22.18)
loop-ar.sh              Loop engineering: shared AR verdicts, rotation, state machine, prompts, disputed reports, fix proof (sourced, or run as `verdict|prompt|disputed|prove-fix`)
loop-review.sh          Loop engineering: ledger-driven PR review — next/record/recover/prompt (a<k>, stage, role, chain)
loop-plan-review.sh     Loop engineering: one rotating AR reviewer ↔ critic round on a frozen draft plan
loop-emit.sh            Loop engineering: best-effort event push to the loop-web daemon (sourced)
loop-models.conf        Loop engineering: model chains, budgets, timeouts
loop-top.mjs            Loop engineering: terminal dashboard (cwd → loop inference, live phase status, notes/pause/model/tier controls). Reads a wsd loop with `ws loop get --json` and steers it with `ws loop …`. Reads every other loop from the loop-web daemon
loop-web.sh             Loop daemon launcher (--daemon = central observer on :7717)
loop-web/               Loop daemon: Node HTTP API, persistence and pure TypeScript model
ws/ws.sh                Rust CLI launcher (see ws/README.md)
wsd/crates/ws/src/cmd/picker.zsh  Embedded fzf picker UI
ws/tests/               Picker smoke test and Rust picker integration shell checks
wsd/                    Default ws implementation: ws-core durable data, ws-daemon runtime, ws CLI + MCP
docs/ws-v6/             WS v6 architecture, semantic conventions and spike reports (S1 scripts stay)
wsd/crates/ws-daemon/tests/fixtures/spikes/  S2 telemetry and S3 quota samples
.claude/skills/ws-sessions/ Agent guide to ws session discovery, messaging, completion and handoffs
.claude/skills/ws-orchestrate/ Repo-less orchestrator: one child session per repo, ws-message coordination, PR URL summary, no repo edits
.claude/skills/assess/ Idea gate before multiphase-plan: evidence → go|clarify|kill → .loop/assessments/<slug>.md with a Handoff goal
.claude/skills/converge/ Read-only Done-when check vs built code (met|partial|missing) → .loop/converge/p<N>.md. loop-execute §8 runs it once per review phase and inserts one gap phase per missing criterion
.claude/skills/visual-explainer/ Props JSON → concept graph (graph.mjs: nodes, edges, fixed layout) → one self-contained surfable HTML page (plan|ship), zero-dep render.mjs + schemas. Narration JSON (one commentary text per report section) → report walkthrough that spotlights each section (template/walkthrough.js)
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

**Loop engineering:** `.claude/skills/loop-execute` orchestrates a plan end-to-end across repositories: one repository per phase and one integration branch/worktree/PR per repository. Explicit PR-review phases use Adversarial Review (AR): a read-only reviewer R reviews a frozen artifact, then a critic C from another provider audits that review. Disagreements require code evidence; each response and critic call is a fresh runner with the prior exchange inlined. Only the fixer edits code, between outer rounds. Tiers are `[review: standard|max]` (default standard): standard uses Sol / Opus / Grok; max uses Astra / Fable / Grok. `[rounds: N]` is a maximum (default 2) and `[inner: N]` defaults to 2. Both accept 1..5. Provider roles rotate GPT→Claude, Claude→Grok, Grok→GPT across outer rounds. A round in which R and C agree that no blocking finding is open goes straight to final. New reviews have no third-provider confirm. Legacy ledgers with `confirm` rows still replay. Reaching the inner cap without agreement writes `.disputed.md`; disputed items must be checked against code and never count as clean. R gives each finding one class: blocking (`security`, `unprotected-breakage` or `failing-check`, with evidence) or deferred. R ends with `NEEDS_CHANGES` only when a blocking finding stays. Deferred findings never cause a fix, a dispute or `REQUEST_CHANGES`. Before the last outer round, Opus fixes run iff the reviewer ends with `NEEDS_CHANGES` or the round is disputed. The next round checks only the fix diff and the open blocking items. An Opus final reviewer alone posts the GitHub review: `REQUEST_CHANGES` for an open blocking finding, otherwise `APPROVE`. On a PR that the authenticated user opened, it posts `COMMENT` with a `Verdict:` line.

`loop-ar.sh` is the shared protocol core; `loop-review.sh next/record/recover/prompt` drives a per-repository `runs/review-p<N>-<owner--repo>.ledger.tsv`. Each completed run has six TSV columns: `a<k>`, stage, role, provider, verdict token, and run directory relative to `.loop/`. Roles are `review|critic|respond|confirm|disputed|fix|final`; no skipped rows are written. Resume recovers finished-but-unrecorded runs, reconciles the GitHub final marker, then asks for `next`. Effort and provider chains are in `loop-models.conf`. `multiphase-plan` uses the same AR protocol before registering: `loop-plan-review.sh run --round r` runs one independent round; the planner verifies the converged review and disputed items, edits the plan between rounds, and stops early on clean. `LOOP_PLAN_REVIEW_ROUNDS` is a maximum (default 2). Round 2 runs only when a blocking finding changed the plan. `light <goal>` skips plan review.

GitHub slugs stay in plans; reusable checkout mappings live in `~/.loop/repos.json` through `loop-repo.sh`. The central **Loop daemon** + `.loop/plan.md` are the only plan backend. On-disk coordinator state stays in flattened `.loop/`; review notes use `notes/<reviewPhase>.md` (legacy `notes/pr-review.*` aliases remain accepted). Contract: `.claude/skills/loop-execute/references/loop-protocol.md`; vendored prompts: `references/ar-prompts.md` beside it. Two-tier mode keeps scheduling/merges in recyclable SUBs and review-phase pipelines in detached runners. Runners run only targeted tests while they work. The runner Verify runs the full repository Verify one time, and `loop-merge.sh --verified-record` does not run it again on the same tree. A new lane worktree gets an APFS clone (`cp -c -R`) of the repository `Build caches:` from the integration worktree. A lane in the same repository keeps its worktree. Shell tests live in `tests/` (`zsh tests/run.sh`).

**Milestones:** one Loop plan may carry several `## Milestone N — <outcome>` sections (plain `## Phases` = one implicit milestone 1). Each ships as its own PR per participating repository and ends with a `[kind: pr-review]` `[gate: merge]` phase. After that review's verdict is posted the loop parks in `awaiting-merge` (plan Status `awaiting merge`, typed `milestone.awaiting-merge` event, notification with the PR URLs); it never finishes or respawns. The human assesses and merges, then runs `/loop-execute resume`: `loop-milestone.sh check` confirms the gate PRs merged, `advance` prepares the next milestone's worktrees from a freshly fetched default branch, and the orchestrator respawns. Between `check` and `advance`, `loop-plan.sh archive --milestone M --dir .loop` (M = 1 up to the parked milestone) moves each shipped milestone's phase blocks and progress-log entries to `.loop/milestones/m<M>.md` and leaves an `[archived]` stub in `plan.md`, so the plan holds full blocks only for unshipped milestones. Every plan reader treats a stub-listed phase as an existing `done` phase; the daemon parser must be restarted to read stubs. Milestone ≥2 integration branches are declared in the plan; worktrees live at `…/<owner--repo>/integration-<branch-slug>-<hash8>` (8-char sha1 of the exact branch). `loop-pickup`/`loop-handoff` never touch phases of a later milestone than `state.milestone`. Format: `.claude/skills/multiphase-plan/references/plan-format.md` → Milestones. After pulling these changes, restart the Loop daemon (`awaiting-merge` status + milestone events are new): `launchctl kickstart -k gui/$(id -u)/com.rstagi.loop-web`; run `./loop-web.sh --daemon` only when no LaunchAgent is bound to :7717.

**Visual explainers:** `.claude/skills/visual-explainer` renders one HTML page from a props JSON file: a concept graph to explore (pan/zoom, detail panel, `#node=<id>` deep links, search, kind filters, outline, tour). The agent writes only the props (optional relation fields add edges). `render.mjs` checks them against `schema/<plan|ship>.schema.json` and the relation ids, builds the graph and its fixed layout (`graph.mjs`), escapes all text, and fills the fixed template. Mode `plan` runs after `multiphase-plan` registers a plan, except for `light` plans. It also runs at each milestone resume. Mode `ship` runs at each milestone merge gate and at the Loop finish, before the notification. The finish page explains the last milestone, or the whole plan when the plan has no milestones. Mode `ship` also runs in `pr-review` Step 4. Each `ship` hand-test command needs a `path:line` code reference. In a Loop run, a detached runner writes the props and the narration, and supervise renders, publishes and links the page. The agent writes `narration.json` (v3: one commentary text for each report section) beside the props. `render.mjs --narration` checks it and adds a walkthrough to the report: it dims the page, puts a spotlight on one section at a time and shows the commentary in a card (`#walkthrough` or `#walkthrough-<n>` opens it). There is no video. A failed explainer never blocks a park, a finish or a resume. Loop output goes to `.loop/explainers/plan/` and `.loop/explainers/m<n>-<plan|ship>/`.

**Milestone retrospective:** at each merge gate and at the finish, supervise runs `loop-retro.mjs --dir .loop --milestone <n> --json` (facts only: attempts, Verify results, escalations, questions, HIL, merge conflicts, review rounds, plan edits, wall time, and `setbackCandidates` with evidence paths). The `ship` explainer runner turns the same facts into the page's "Setbacks and hiccups" scene (`process` + `setbacks` props) and into `.loop/milestones/m<n>.retro.md` for agents (format: protocol § Milestone retrospective). A deviation is a different result; a setback is a process cost. `loop-plan.sh archive` puts the retro first in `milestones/m<n>.md` (then `## Raw log`), uses its Outcome for the stub and removes the retro file. Milestone resume seeds the SUB with the latest archived Lessons (≤15 lines); `multiphase-plan` reads the Lessons of the last 3 archives before it revises phases. A missing retro never blocks a park or a finish.

**AR rollout:** after an update, restart the daemon with `launchctl kickstart -k gui/$(id -u)/com.rstagi.loop-web`. Check `curl -fsS http://localhost:7717/api/health`. Before execution, replace each active plan's legacy review tier with `[review: standard]` or `[review: max]`. Execution paths and the tier endpoint reject legacy shallow and medium tiers. The model and `loop-top` retain these tiers for historical loops.

**Loop daemon (`loop-web`):** a central Node daemon on `127.0.0.1:7717` stores plans and lifecycle events for the old engine. `multiphase-plan` registers plans before execution. Loop scripts send events through `loop-emit.sh` and `loop-plan.sh`. Events promote phase status through a monotone lattice. Each loop persists in `~/.loop/loops/<runId>.json`.

Use `ws top` to view loops and change steering controls. The server imports its TypeScript model through Node's native type stripping. It needs no build step. The daemon and event contract is in `loop-protocol.md` → Daemon & events.

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
