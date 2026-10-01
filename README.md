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

## Agent checkpoints

Invoke `/checkpoint-30` in Claude or `$checkpoint-30` in Codex to request a status report and guidance after 30 minutes on one interactive task. Interactive timing is advisory. Loop phase attempts have a hard 30-minute cutoff and return a checkpoint to the orchestrator; other phases continue.

Loop defaults: Opus 5.5 orchestrator; GPT-6-Sol phase executor with Sonnet 5 fallback. Escalation and review use separate model chains. GPT-6-Sol requires Codex CLI 0.155.0 or newer.

New Loop plans include an estimate and confidence for each work phase. The planner targets 20–25 minutes of active work per phase, leaving room for the 30-minute runner checkpoint. The terminal review phase runs as nine bounded stages per repository. Retries and queue waits can take longer.

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
