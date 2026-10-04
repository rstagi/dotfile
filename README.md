# Dotfile

## Automatic install

Run the following command to install everything:
```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/rstagi/dotfile/master/install.sh)"
```

## ws — terminal workspaces

Bare-minimum Conductor in tmux: `./install.sh ws`, then `ws`.

- Workspace = git worktree under `~/.ws/worktrees/<repo>/<branch>` + its own tmux session (`repo/branch`). Each agent session is a tab in it (`claude`, `claude#2`, `codex#3`). Agents run with permission prompts off (Claude `--dangerously-skip-permissions`, Codex `--dangerously-bypass-approvals-and-sandbox`).
- `ws new [--agent codex]` picks a repo (current repo, `~/.loop/repos.json`, then repos found under `$WS_REPO_ROOTS`, default `~/Dev ~/dotfile`; or type any path) + branch, copies `.env*`, runs setup in a `setup` tab (closes on success). Honors `.conductor/settings.toml` (`file_include_globs`, `[scripts] setup`), else an executable `.ws-setup`.
- Picker: `ws` (attaches from a plain terminal) or `prefix+w` inside a ws session — one row per worktree, pinned first, then grouped by repo. enter open · ^n new · ^o new codex · ^a add claude tab · ^t add codex tab · ^s show/hide sessions · ^p pin · ^x rm. `prefix+N` = new; `prefix+a` / `prefix+A` = claude / codex tab in the current worktree. CLI: `ws add [repo/branch] [--agent codex]` (defaults to the worktree you're in), `ws open`, `ws rm`, `ws list`.
- Persistence: sessions + their Claude/Codex session ids are recorded per worktree. After a reboot (or closing tabs), workspaces show as stopped; opening one resumes every session by id. Quitting an agent cleanly forgets its session.
- Status (○ idle · ◐ working · ● needs input) comes from per-launch agent hooks → tab icons + a `terminal-notifier` notification (sound; click brings the terminal + tab to front) when an unfocused agent stops or needs input. Notifications come from a branded copy of terminal-notifier at `~/.ws/ws.app` (own name + robot icon `assets/ws/app.png`, composed from `assets/ws/robot.png`, built by `install.sh ws` or `ws _build-notifier`; re-run after `brew upgrade terminal-notifier`). Allow **ws** in System Settings → Notifications (style *Alerts* keeps them on screen); until then it falls back to `osascript`. Customize via `WS_NOTIFY_SOUND_DONE` (default Glass) / `WS_NOTIFY_SOUND_WAITING` (default Ping) — any name in `/System/Library/Sounds` or `~/Library/Sounds` — and `WS_NOTIFY_ICON_DONE` / `WS_NOTIFY_ICON_WAITING` (default `assets/ws/{done,waiting}.png`).
- `ws merge` / `prefix+M` (inside a workspace): merges the branch's PR with `gh` (`$WS_MERGE_METHOD`, default squash), fast-forwards the default branch in the main checkout, then — after confirmation — removes the worktree, its tabs and the local + remote branch.
- Trust: run `claude` once in `~/.ws/worktrees` and accept (covers all worktrees); Codex asks once per repo.

### Notification adapters

`WS_NOTIFY=0` disables notifications. `WS_NOTIFIER` selects the adapter:

- `auto` (default): use the branded `~/.ws/ws.app`, else `terminal-notifier` on PATH; fall back to `osascript` if missing or rejected by macOS.
- `terminal-notifier`: explicitly select the same terminal-notifier adapter, including its osascript fallback.
- `osascript`: use macOS AppleScript notifications directly (text only, as with the fallback).
- An executable path, e.g. `export WS_NOTIFIER="$HOME/bin/ws-notify"`: receive one compact JSON object plus newline on stdin, without arguments. Paths containing spaces work. Adapters run asynchronously; failures do not block agent hooks.

Currently, events are sent only when an unfocused tab transitions to `idle` or `waiting`. Repeated states and `working` transitions do not emit events. The built-in adapters preserve the sounds, icons, per-tab replacement and click-to-focus behavior described above.

The v1 event schema is shared by all adapters:

| Field | Type | Meaning |
| --- | --- | --- |
| `v` | number | Protocol version, `1` |
| `id` | string | Stable tab identity, e.g. `ws:@12`; use for replacement |
| `source` | string | `ws` |
| `state` | string | `working`, `idle` or `waiting` (currently only the latter two emit) |
| `prev` | string | Previous tab state; empty if unknown |
| `title` | string | Notification title, currently `ws` |
| `message` | string | Workspace + status, e.g. `api/feat-login needs input` |
| `detail` | string | Hook detail; currently empty |
| `agent` | string | Agent name, `claude` or `codex`; empty if unknown |
| `repo` | string | Repository name from the workspace |
| `branch` | string | Git branch name, preserving `/`; empty if unknown |
| `focused` | boolean | Whether the tab is focused; currently always `false` |
| `sound` | string | Semantic key: `done` for idle, `waiting` for input |
| `actions` | array | Objects with string `id`, `label`, `command`; currently one `focus` action labelled `Focus tab` |
| `ts` | number | Unix timestamp in seconds (UTC) |

The `focus` action's shell command selects the tab and switches the tmux client to its session. The terminal-notifier adapter also activates the terminal app. Custom adapters decide how to display events and handle actions; execute commands only from a trusted local source. Adapters should ignore unknown fields and reject unsupported protocol versions.

## Agent checkpoints

Invoke `/checkpoint-30` in Claude or `$checkpoint-30` in Codex to request a status report and guidance after 30 minutes on one interactive task. Interactive timing is advisory. Loop phase attempts have a hard 30-minute cutoff and return a checkpoint to the orchestrator; other phases continue.

Loop defaults: latest Opus orchestrator; latest GPT Sol phase executor with latest Sonnet fallback. Escalation (Fable, Opus fallback) and review use separate model chains. Effort is per role: implementing `high`, Opus fixes `medium`, orchestrating `high`, reviewing by tier. PR reviews are tiered with `[review: shallow|medium|max]` on the review phase (default medium: Sol + Opus adversaries, Opus fixes, Opus verdict, 3 rounds; shallow: one Opus reviewer, Sonnet fixes, 1 round; max: Astra + Fable adversaries) and `[rounds: N]`. Chains in `loop-models.conf` track the newest model of each family — Claude aliases (`opus`, `sonnet`, `fable`) and Codex `@<family>` legs (`codex:@sol`, resolved at launch from `codex debug models`) — so new releases need no edit; pin a full model name to freeze one.

New Loop plans include an estimate and confidence for each work phase. The planner targets 20–25 minutes of active work per phase, leaving room for the 30-minute runner checkpoint. The terminal review phase runs as bounded stages per repository (medium 3 rounds = nine runs; `loop-review stages` prints the list). Retries and queue waits can take longer.

## Loop dashboard (`loop-top`)

`loop-top` shows the phases of a Loop run in the terminal, live from the central daemon (`:7717`). Run it anywhere: it infers the loop from cwd (coordinator worktree, phase worktree, or git branch + origin) and otherwise opens an fzf picker over active + finished loops. HIL questions are still answered through the orchestrating agent.

- Rows show the model that actually ran (`codex:gpt-6.1-sol`), 📝 on phases with a note, deps (`← 2a, 2b`), and `waiting on 2a` for a todo phase with unmerged deps.
- `↑↓`/`j k` select a phase, `⏎` opens its details (attempts, heartbeat, HIL text, note), `esc` goes back.
- `n` edits the phase's steering note in `$EDITOR` (empty = clear). A running phase picks it up within ~2s: the runner interrupts the engine and resumes the same session with the note.
- `x` pauses/resumes the selected phase; `X` pauses/resumes the whole loop (asks y/n). Pausing halts the runners (and the orchestrator for `X`) within ~2s; resuming continues the same engine session.
- Review phases show their tier and rounds (`medium review ×3`); change them with `[review: shallow|medium|max]` / `[rounds: N]` in the plan.
- A running review phase shows its sub-stages: the state names the current one + progress (`fix1 · 3/9`; `round2 failed` when a run fails), and a line per repository under the row draws the pipeline (`round1 ✓✓ › fix1 ◐ › round2 ·· › … › final ·`). Its `⏎` details list every run (`a<k>`, stage, chain, state, model — planned model in parens — and summary). The Observatory drawer shows the same "Review pipeline".
- Model column: `codex:gpt-6.1-sol` = what ran / is running now (live from the runner's `leg.json`), `→ claude:opus` = your override, `(codex:@sol)` = planned (first `CHAIN_TASK` leg; the light/default route is decided at launch), `medium review ×3` = review tier (untagged review phases use the default tier).
- `m` on a review phase that hasn't started picks its tier (shallow ×1 · medium ×3 · max ×3, with each tier's models) — the daemon rewrites its `[review: …]` tag in `.loop/plan.md`. The tier can also be named up front when calling `multiphase-plan` ("review: max"); otherwise it's proposed and confirmed there.
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
