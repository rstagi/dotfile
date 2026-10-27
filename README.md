# Dotfile

## Automatic install

Run the following command to install everything:
```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/rstagi/dotfile/master/install.sh)"
```

You can run the installer again. A second run changes no file. Before the installer links a dotfile config (`.gitconfig`, `.zshrc`, `~/.config/nvim`, Claude and Codex config), it moves a different file or folder to a new `<path>.bak.<timestamp>` backup. For other links (`ws`, `docker-buildx`, `.tmux.conf`), it keeps a different file and prints a warning.

## ws — terminal workspaces

Bare-minimum Conductor in tmux: a git worktree + tmux session per branch, one tab per Claude/Codex/Grok session, hook-driven status, notifications, and a session bus (`ws send`, `ws done`, `ws on`). `./install.sh ws`, then `ws`. Docs: [ws/README.md](ws/README.md).

## Agent checkpoints

Invoke `/checkpoint-30` in Claude or `$checkpoint-30` in Codex to request a status report and guidance after 30 minutes on one interactive task. Interactive timing is advisory. Loop phase attempts have a hard 30-minute cutoff and return a checkpoint to the orchestrator; other phases continue.

Loop defaults: latest Opus orchestrator; latest GPT Sol phase executor with latest Sonnet fallback. Escalation (Fable, Opus fallback) and review use separate model chains. Effort is per role: implementing `high`, Opus fixes `medium`, orchestrating `high`, reviewing by tier. PR-review phases accept `[review: standard|max]` (default standard): standard uses Sol / Opus / Grok; max uses Astra / Fable / Grok. `[rounds: N]` sets the maximum number of outer rounds (default 2) and `[inner: N]` caps reviewer/critic exchanges per round (default 2). Both accept 1..5. Chains in `loop-models.conf` track the newest model of each family — Claude aliases (`opus`, `sonnet`, `fable`), Codex `@<family>` legs (`codex:@sol`, resolved at launch from `codex debug models`) and `grok:default` (the Grok CLI's default model) — so new releases need no edit; pin a full model name to freeze one. Grok reviewers need `grok` installed + logged in (`./install.sh grok`, then `grok login`) and send the reviewed repo to xAI (see the Grok caveat in [ws/README.md](ws/README.md)); a failed Grok reviewer blocks that repository's review like any other reviewer.

Adversarial Review (AR) freezes the artifact while reviewer R reports numbered findings and critic C audits them with full code context. Disagreements require code evidence; R responds and C audits again within the inner cap. Only the fixer (or planner for plan review) edits the artifact, between outer rounds. Providers rotate: GPT reviews / Claude critiques in round 1, Claude / Grok in round 2, Grok / GPT in round 3, then repeat. When R and C agree that no blocking finding is open, the review skips later rounds and goes to the final review. Unresolved disagreements at the cap produce `.disputed.md`, never a clean round. Each finding is blocking (a security problem, unprotected breakage or a failing check) or deferred. Only blocking findings make R say `NEEDS_CHANGES`, start fixes or make the final review request changes. Before the last outer round, Opus fixes run when the last review says `NEEDS_CHANGES` or the round is disputed. The next round checks only the fix diff and the open blocking items. The final Opus reviewer checks the last converged review, disputed items, and current code, then posts the only GitHub review.

New Loop plans include an estimate and confidence for each work phase. The planner targets 20–25 minutes of active work per phase, leaving room for the 30-minute runner checkpoint. The terminal review phase uses a per-repository AR ledger. `loop-review.sh next --tier standard --rounds 2 --inner 2 --ledger <file>` returns the next reviewer, critic, response, fix, or final run; a clean round stops later rounds. `record` appends completed evidence atomically, `recover` replays finished runs after a crash, and `prompt` renders each role’s instructions. The CLI rejects legacy shallow/medium tiers. Recovery retains custom limits in the ledger’s `.config.json` sidecar. Retries and queue waits can take longer.

Each `runs/review-p<N>-<owner--repo>.ledger.tsv` row contains `a<k>`, stage, role, provider, verdict token, and run directory relative to `.loop/`. Roles are `review|critic|respond|confirm|disputed|fix|final`; a clean round implies skipped rounds without writing skipped rows. Old ledgers with third-provider `confirm` rows still replay. Resume recovers completed runs, reconciles the GitHub final marker, then calls `next`. Plan review uses the same shared `loop-ar.sh` core: `loop-plan-review.sh run --round r` runs one independent round; the planner verifies findings and disputes against code and edits between rounds (at most 2 via `LOOP_PLAN_REVIEW_ROUNDS`, round 2 only after a blocking fix). `multiphase-plan light <goal>` skips plan review and the plan explainer. See [the AR protocol](.claude/skills/loop-execute/references/loop-protocol.md) and [rollout steps](CLAUDE.md#architecture) before upgrading an active loop.

The skills engine records review results with `loop-runner.sh --review-key <step key> --publish-output <report>` and `WS_LOOP_ID`. Keys use `review:<phase>:<repo or none>:<role>:<round>:<inner>:attempt:<k>`. Each published result appends one `loop.review` event with the key suffix `:result`. Final verdicts come from `status.json`, including `COMMENT` and `REQUEST_CHANGES`. Fixes need remote-head proof from `loop-ar.sh prove-fix`. `loop-ar.sh disputed --ledger <file> --review-key <step key>` records its sidecar without a runner session. Checkpoints and unsuccessful runs append no result. Keyed reviews append no `loop.attempt` result. Without these options, the old engine retains its behavior.

The plan tools require Node >= 22.18. `node loop-plan-parse.mjs --plan .loop/plan.md` prints normalized plan JSON. Use `--plan -` to read standard input. The parser and `loop-plan.sh archive` use `loop-plan-model/` and need no `loop-web/` files. The daemon keeps its own parser until it retires.

`loop-plan.sh` uses `wsd` for plans with `engine: skills` in the header. Other plans use loop-web.
Skills writes use `ws loop create` and `ws event`. Reads and listing select the active engine's copy.
Change the engine only while the loop is parked at a merge gate.
For rollback, remove `engine: skills`, then run `loop-plan.sh push` to update both stores.

For the skills engine, `WS_LOOP_ID=<loop-session-id> ./loop-milestone.sh open --dir .loop`
starts from `plan.md` alone or adopts an advanced `state.json`. It prepares missing integration
worktrees and records the current milestone, phases and merged gates in the loop session.
A second call changes nothing. Without `WS_LOOP_ID`, it exits 1. A parked loop exits 3 and
requires `loop-milestone.sh resume`. The `check` and `advance` commands retain their existing behavior.

After the gate PRs merge, run `./loop-milestone.sh resume --dir .loop`, then `ws loop start .loop`.
The resume command checks the PRs, archives shipped milestones, prepares the next integration
worktrees, and applies one state patch. With `WS_LOOP_ID`, it records keyed gate, milestone and
phase events. `WS_LOOP_ID` may be a session id or a plan id. On a plan with `engine: skills`, the command finds the loop from `state.runId` and exits 1 when it cannot. On other plans, it updates the files and the plan only. A retry after the
apply continues from `resumePending`. Open or closed PRs stop the command without changes.
`ws loop resume` only clears a pause.

## Loop dashboard (`loop-top`)

`loop-top` shows the phases of a Loop run in the terminal, live from the central daemon (`:7717`). Run it anywhere: it infers the loop from cwd (coordinator worktree, phase worktree, or git branch + origin) and otherwise opens an fzf picker over active + finished loops. HIL questions are still answered through the orchestrating agent.

- Rows show the model that actually ran (`codex:gpt-6.1-sol`), 📝 on phases with a note, deps (`← 2a, 2b`), and `waiting on 2a` for a todo phase with unmerged deps.
- `↑↓`/`j k` select a phase, `⏎` opens its details (attempts, heartbeat, HIL text, note), `esc` goes back.
- `n` edits the phase's steering note in `$EDITOR` (empty = clear). A running phase picks it up within ~2s: the runner interrupts the engine and resumes the same session with the note.
- `x` pauses/resumes the selected phase; `X` pauses/resumes the whole loop (asks y/n). Pausing halts the runners (and the orchestrator for `X`) within ~2s; resuming continues the same engine session.
- Review phases show their tier and rounds (`standard review ×2`); change them with `[review: standard|max]` / `[rounds: N]` / `[inner: N]` in the plan.
- A running review phase shows its current stage and progress, plus one pipeline per repository. Completed runs come from the AR ledger; the remaining path is planned, and a clean third-provider confirmation marks later rounds skipped. Skipped stages do not count as pending or become the current stage. Its `⏎` details list every run (`a<k>`, stage, chain, state, model — planned model in parens — and summary). The Observatory drawer also shows role, provider, and verdict.
- A shipped (archived or gate-merged) milestone is one line, `M5 — <title> ✓ completed 2026-10-26 11:02` (local time; no time when the source has none). Its phases are hidden. A done phase shows how long it took from its first start and when it finished: `42m · 10-26 11:02`.
- Model column: `codex:gpt-6.1-sol` = what ran / is running now (live from the runner's `leg.json`), `→ claude:opus` = your override, `(codex:@sol)` = planned (first `CHAIN_TASK` leg; the light/default route is decided at launch), `standard review ×2` = review tier (untagged review phases use the default tier).
- `m` on a review phase that hasn't started picks `standard` or `max`, showing rounds, inner cap, R/C rotation, fixer, and final models from `loop-models.conf` — the daemon rewrites its `[review: …]` tag in `.loop/plan.md`. Historical shallow/medium tiers still render, but cannot be selected for execution. The tier can also be named up front when calling `multiphase-plan` ("review: max"); otherwise it's proposed and confirmed there.
- `m` sets the model for a phase that hasn't started (fzf over `loop-models.conf` legs, or type `engine:model`); the default chain stays as fallback.
- `p` opens the plan in `$PAGER`; `g` toggles a git-log-style dependency graph; `w` toggles `branch → worktree`; `l` switches loop; `q` quits.
- `loop-top <runId>` opens a specific loop; `loop-top --once [--worktrees] [--graph]` prints one frame.
- Loop picker (start without an inferable loop, or `l`): opens in navigation mode — `⏎` open · `a` archive (reloads) · `j`/`k` move · `q` quit; `/` shows the search box (`esc` hides it).
- `ltop` is a shorthand alias for `loop-top`.
- `loop-top archive [runId]` hides a loop from the picker (default: the one inferred from cwd; asks first, `--yes` skips) without touching its worktree; `loop-top unarchive <runId>` brings it back. Loops whose coordinator worktree is gone are archived automatically.
- Daemon down → renders the last stored snapshot from `~/.loop/loops` (`daemon offline · from store`); actions need the daemon.

## Jev decisions in Loop

Loop can ask Jev for bounded route, runner-question, and pre-merge-risk recommendations. Put a
single-line key in `~/dotfile/.loop-secrets/typesafe-api-key` (gitignored; owner-readable only),
or set `TYPESAFE_API_KEY` in the supervisor environment. The environment value takes precedence;
`LOOP_JEV_KEY_FILE` can select another local file. Then choose a mode:

- `LOOP_JEV_MODE=off` disables calls explicitly. With no mode and no key, Loop is also off and
  records `missing_credentials`; explicit `off` records `disabled`.
- `LOOP_JEV_MODE=shadow` records advice but preserves the established route and triage behavior.
- `LOOP_JEV_MODE=active` may apply only allowlisted, above-threshold decisions. Routing remains
  opt-in until shadow replay supports the configured threshold.

Routing sends bounded phase title, Done when, and Estimate text after redacting common credential
forms. Without Done when and Estimate evidence, active routing keeps the default task chain.
Pre-merge risk requests send changed paths, diff totals, Done when, and a bounded verification
summary; they omit patch contents. The mandatory full diff skim still runs locally.

Verification, full diff review, merge checks, escalation, and human gates remain authoritative in
every mode. Observatory shows the current mode and fallback count in the header, decision badges on
phases, and proposal-versus-applied details in the phase drawer. Archived loop snapshots retain the
same decision history.

Replay one or more stored loop records locally without modifying them:

```bash
node loop-jev-replay.mjs ~/.loop/loops/<run-id>.json | jq .
```

The report includes agreement and fallback rates. Latency, runner retries, and cost are shown only
when present in the records and otherwise read `unavailable`. Synthetic fixtures validate report
mechanics only; they do not support savings claims.

## Manual configurations

There are some packages to be configured manually.

### .zshrc extensions

The `.zshrc` file after the installation checks if a `~/.zshrc_ext` file exists and, if so, it sources it. This is useful to add some custom configurations that are not present in the main file.

### Raycast

Replace the Spotlight shortcut with Raycast:
- Remove Spotlight shortcut `CMD + space` from Keyboard Settings > Keyboard Shortcuts > Spotlight
- Set Raycast Hotkey to `CMD + space` in Raycast Settings > General

#### Extensions list
TODO: add the missing hot keys
- **Brew**
    - **Search** Hotkey: `Option + B`
- **Clipboard History**
- **Code Stash**
- **Coffee**
- **Color picker**
- **Define Word**
- **Floating Notes**
    - **Toggle Floating Notes Focus** Hotkey: `Option + .`
- **Format JSON**
- **GitHub**
- **Google Search**
- **Google Translate**
- **Google Workspace**
- **My Password** (1password)
- **Navigation**
- **Notion**
- **Search Emoji**
- **Set Audio Device**
- **Show Cheatsheets**
- **Snippets**
- **Speedtest**
- **Window Management**
- **iTerm**

### 1Password

Login with my account (or accounts) and sync everything. Then, configure the SSH Agent to use 1Password as a source for SSH keys. (TODO: explain how)

Finally, configure the browser extension to use the 1Password app instead of the web interface.

### RunCat

This package needs to be installed manually from the AppStore.

### Launch Apps at startup

Go to `Settings > General > Login items` and set the following apps to start when logging in:
- Raycast
- Rectangle
- RunCat
