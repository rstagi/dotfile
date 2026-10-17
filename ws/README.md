# ws — terminal workspaces

Bare-minimum Conductor in tmux: `./install.sh ws`, then `ws`.

- Workspace = git worktree under `~/.ws/worktrees/<repo>/<branch>` + its own tmux session (`repo/branch`). Each agent session is a tab in it (`claude`, `claude#2`, `codex#3`, `grok#4`). Agents run with permission prompts off (Claude `--dangerously-skip-permissions`, Codex `--dangerously-bypass-approvals-and-sandbox`, Grok `--always-approve`; resumed with `--resume <id>`).
- `ws new [--agent codex|grok]` picks a repo (current repo, `~/.loop/repos.json`, then repos found under `$WS_REPO_ROOTS`, default `~/Dev ~/dotfile`; or type any path) + branch, copies `.env*`, runs setup in a `setup` tab (closes on success). Honors `.conductor/settings.toml` (`file_include_globs`, `[scripts] setup`), else an executable `.ws-setup`.
- Picker: `ws` (attaches from a plain terminal) or `prefix+w` inside a ws session — one row per worktree, pinned first, then grouped by repo. enter open · ^n new · ^o new codex · ^r new grok · ^a add claude tab · ^t add codex tab · ^v add grok tab · ^s show/hide sessions · ^p pin · ^x rm. `prefix+N` = new; `prefix+a` / `prefix+A` / `prefix+G` = claude / codex / grok tab in the current worktree. CLI: `ws add [repo/branch] [--agent codex|grok]` (defaults to the worktree you're in), `ws open` (focuses the tab you last used), `ws rm`, `ws list`. In a tab you opened by hand (`prefix+c`), `ws claude` / `ws codex` / `ws grok` runs the agent as a tracked session (hooks, status, resume); the tab is a plain shell again after it exits.
- Persistence: sessions + their Claude/Codex session ids are recorded per worktree. After a reboot (or closing tabs), workspaces show as stopped; opening one resumes every session by id. Quitting an agent cleanly forgets its workspace resume slot and marks its global record closed.
- Each agent tab also gets a global ID in `WS_SESSION`, stored under `$WS_HOME/sessions/<id>/` (default `~/.ws`). Codex launches pass `-c developer_instructions=...` (the ws preamble), overriding any configured `developer_instructions`. `ws whoami` prints your ID; `ws sessions [id|name]` lists ID, name, workspace, agent, state and status as TSV. IDs survive workspace resume and never reuse. `ws name <label>` names your own session (unique, starts with a letter; letters/digits/dots/underscores/hyphens; `me` reserved). `ws open <id|name>` focuses or resumes that session.
- `ws send <id|name> <message|->` delivers a message to an idle agent with bracketed paste and Enter. Use `-` to read multiline text from stdin. Messages start with `[ws #<sender-id>]` (`[ws terminal]` outside a session). Working/waiting targets queue messages durably in their inbox; Claude, Codex and Grok drain them through a synchronous Stop hook, keeping the tab working during continuation. A continuation Stop delivers messages queued since the last reply; cancellation falls back to paste only when its turn id matches the current turn and the tab is idle (including background subagents). Stale or idless cancel/Interrupt hooks leave state and inbox unchanged. A Stop that cannot take the hook lock within 3s hands off to a detached retry that settles the tab idle and drains the inbox (`WS_LOCK_SCALE`, default 1, multiplies every lock wait; for tests). Hooks only act for the tab's agent pane (a Grok in a split pane is ignored). A Stop hook killed before its reply landed leaves the messages in flight for the next delivery. Grok session-end Stops (`channel_closed`/`shutdown`) leave the inbox for resume. Idle-pasted batches stay in flight until a newer `UserPromptSubmit` confirms submission; paste alone leaves the tab idle. If Enter is swallowed, `WS_PASTE_ACK_TIMEOUT` (seconds, default 10) requeues the batch for a later send or Stop; delivery may repeat after an unconfirmed submission. An interrupted (Esc) turn may emit no hook, so queued messages wait for the next turn's Stop/idle. Paste + Enter submits whatever is in the agent's input: unsent text typed into a human-opened tab goes out with the message. Done targets warn and require `--force` (before the target or after the message). Stopped targets resume by their retained ID before delivery. Sending during a fresh/resumed tab's startup requires tmux 3.8+ to verify bracketed-paste readiness and avoid pasting into the shell. If startup fails or times out, the message remains queued. Exit 75 (with a `message queued` warning) means durably queued but not yet delivered: a later Stop or idle delivers it, and for an idle target a detached retry (`WS_DELIVER_RETRIES` attempts, default 12, every `WS_DELIVER_RETRY_INTERVAL` seconds, default 5) delivers it once input is ready, so it is not a failure and must not be re-sent. Exit 1 is a genuine failure (bad target, missing workspace, enqueue failed).
- `ws done [--output <file>|-]` declares your own session done, writes `output.md` (empty when omitted), and appends a `done` lifecycle event. Both `-` and `--output -` read stdin; repeated declarations replace output and append another event; fan-in triggers evaluate in a detached worker, so `ws done` returns without waiting on their actions. Pending inbox or in-flight messages refuse completion without publishing output or closing the tab: finish the turn so Stop delivers them, handle them, then retry `ws done`. Tabs marked `spawned` close after publication; user-opened tabs stay open. Agent process termination appends an `exit` event with its exit code, including manually adopted tabs; received SIGHUP is recorded as code 129. Exited tabs stop accepting messages before exit triggers run; sends resume the session. Events live as JSONL in `$WS_HOME/sessions/<id>/events` (`id`, `event`, `at`; `done` adds `output`, `exit` adds `code`). `ws show <id|name>` prints session metadata, status and the output path.
- `ws on <id[,id…]> <done|exit> (--send-to <id|me> <message> | --spawn <prompt> [--agent claude|codex|grok])` registers a one-shot fan-in trigger. It fires after every source has emitted the event. Already-recorded `done` events count; `exit` requires a new event from every source after registration, so past exits before a resume do not fire a new watcher. Spawn defaults to Claude and opens a tracked child in the registering session’s worktree (or the current ws worktree when run from a terminal), with `WS_SESSION` and the expanded prompt as a single positional argument. Prompts follow `--`, so option-like text stays a prompt. Spawn origin is recorded before launch; `ws done` closes the child tab. `{id}` expands to comma-separated source IDs in registration order; `{output}` expands to their output contents joined by a newline (empty for sources without output). Replacement is literal. Output-only templates are rejected at registration: include text or `{id}` alongside `{output}` so a plain `ws done` cannot produce an empty continuation. `ws triggers` lists pending triggers as TSV (trigger ID, sources, event, action, target, template). `ws on --rm <trigger-id>` cancels a pending trigger. Direct and transitive cycles in pending send dependencies are rejected under the registration lock. Pending trigger records live under `$WS_HOME/triggers/`; firing is claimed before delivery. Fired, queued (send accepted but not yet delivered; warning in `warning`, not a failure, never re-send) and failed records move to `$WS_HOME/triggers/archive/` after the action, retaining diagnostics without scanning them on later events. Failed actions are never retried automatically.
- Agents launched, resumed or run in an existing tab receive their global session id; spawned sessions also get a reminder to run `ws done` when the task is complete (done is permanent, so user-opened tabs aren't told to). Tabs get `WS_BIN` and a `ws` on `PATH` (`$WS_HOME/bin/ws`); `install.sh ws` also links `~/.local/bin/ws`, so non-interactive agent shells can run `ws`. Spawn prompts reach the agent via a one-shot `0600` file, so size isn't bound by tmux's command limit. `WS_READY_TIMEOUT` (seconds, default 60) bounds the wait for a starting tab's input before a send stays queued. The [ws-sessions skill](../.claude/skills/ws-sessions/SKILL.md) teaches discovery, messages, completion output, “wait for #123 then do ABC” and “spawn after” handoffs. `./install-agent-skills.sh` shares it with Claude (`/ws-sessions`), Codex (`$ws-sessions`) and Ratel Local; `--no-ratel` skips Ratel registration. Handoffs use the session bus without a Loop plan.
- Status (○ idle · ◐ working · ● needs input) comes from per-launch agent hooks (a Claude tab stays ◐ while background subagents run past the main turn) → tab icons + a `terminal-notifier` notification (sound; click brings the terminal + tab to front) when an unfocused agent stops or needs input. Notifications come from a branded copy of terminal-notifier at `~/.ws/ws.app` (own name + robot icon `assets/ws/app.png`, composed from `assets/ws/robot.png`, built by `install.sh ws` or `ws _build-notifier`; re-run after `brew upgrade terminal-notifier`). Allow **ws** in System Settings → Notifications (style *Alerts* keeps them on screen); until then it falls back to `osascript`. Customize icons via `WS_NOTIFY_ICON_DONE` / `WS_NOTIFY_ICON_WAITING` (default `assets/ws/{done,waiting}.png`); sound settings are below.
- `ws merge` / `prefix+M` (inside a workspace): merges the branch's PR with `gh` (`$WS_MERGE_METHOD`, default squash), fast-forwards the default branch in the main checkout, then — after confirmation — removes the worktree, its tabs and the local + remote branch.
- Grok Build (xAI `grok`, optional: `install.sh ws` asks, or `./install.sh grok`): it has no per-launch hooks flag, so ws writes global hooks to `~/.grok/hooks/ws.json` (`$GROK_HOME` honored; no-ops outside ws). Grok also runs your `~/.claude/settings.json` hooks (Claude compat; disable with `[compat.claude] hooks = false` in `~/.grok/config.toml`). ⚠️ In July 2026 Grok Build CLI (v0.2.93) uploaded whole tracked repos + git history to xAI storage; reportedly disabled server-side 13 Jul 2026, no advisory — use only on repos you're OK sending to xAI.
- Trust: run `claude` once in `~/.ws/worktrees` and accept (covers all worktrees); Codex asks once per repo.

## Notification adapters

`WS_NOTIFY=0` disables notifications. `WS_NOTIFIER` selects the adapter:

- `auto` (default): use the branded `~/.ws/ws.app`, else `terminal-notifier` on PATH; fall back to `osascript` if missing or rejected by macOS.
- `terminal-notifier`: explicitly select the same terminal-notifier adapter, including its osascript fallback.
- `osascript`: use macOS AppleScript notifications directly, with the same sound playback as terminal-notifier.
- `boringnotch`: send every transition to the Boring Notch fork's Unix socket; the app plays the sound (pick it in its Settings). If the socket is unavailable, fall back to `auto`, which plays the ws cue.
- An executable path, e.g. `export WS_NOTIFIER="$HOME/bin/ws-notify"`: receive one compact JSON object plus newline on stdin, without arguments. Paths containing spaces work. Adapters run asynchronously; failures do not block agent hooks.

Every hook state transition emits an event, including `working` and focused tabs. Repeated states do not emit events, and ws drops hooks whose `seq` is not newer than the tab's last applied `seq` (overlapping hooks are serialized per tab). Filtering belongs to the adapter: the built-in macOS adapters apply the same per-tab `seq` rule before delivery (including `working` events), then display only unfocused `idle` and `waiting` events with their sound. terminal-notifier adds icons, per-tab replacement and click-to-focus; `osascript` (explicit or fallback) shows text only.

Once a tab has notified, ws dismisses it with a `closed` event when you select that tab (tmux `session-window-changed` / `client-session-changed` hooks, installed with ws's key bindings) or when its agent exits. The tab state is unchanged; receivers drop the tab's notification (terminal-notifier removes its banner; the notch removes its activity). Each notification is closed at most once.

For notch notifications, build and run the local [Boring Notch fork](https://github.com/rstagi/boring.notch), then enable **Settings → General → System features → External notifications** (on by default in the fork). Set this in your shell config before launching ws sessions (agent hooks and tmux hooks read the tmux server's environment, so if tmux is already running also run `tmux set-environment -g WS_NOTIFIER boringnotch`):

```sh
export WS_NOTIFIER=boringnotch
# Optional: point at a different local receiver.
export WS_BORINGNOTCH_SOCKET="$HOME/Library/Application Support/boringNotch/notify.sock"
```

The adapter sends one NDJSON line per event using `nc -U -w 1`. The fork creates a user-only directory (0700) and socket (0600), updates activities by tab ID, keeps input requests visible, and runs the event's `focus` action on click. Quitting the app or disabling External notifications removes the socket; ws then uses `auto` with its usual focus/state filtering. Delivery has a one-second timeout and runs asynchronously. The adapter plays sounds locally; the fork does not, and fallback plays the cue only once. Shell actions are trusted local input; an upstream API should consider URL actions.

Built-in adapters play a soft chime for `done` (`assets/ws/sounds/done.wav`) and a rising attention cue for `waiting` (`assets/ws/sounds/waiting.wav`). Both are from [Kenney Interface Sounds](https://kenney.nl/assets/interface-sounds), licensed CC0; source files and conversions are recorded in [the bundled license](../assets/ws/sounds/LICENSE.txt).

Override each cue with `WS_NOTIFY_SOUND_DONE` / `WS_NOTIFY_SOUND_WAITING`: a macOS sound name such as `Glass` or `Ping` (first `<name>.*` file in `~/Library/Sounds`, `/Library/Sounds`, then `/System/Library/Sounds`), or a file path, including paths with spaces (relative paths use the hook working directory). For example:

```sh
export WS_NOTIFY_SOUND_DONE=Glass
export WS_NOTIFY_SOUND_WAITING="$HOME/Music/needs input.wav"
```

Playback uses `afplay` once per notification, including fallbacks, in the background so long cues never delay the notification; terminal-notifier's own sound is suppressed. With `boringnotch`, ws plays nothing on successful delivery: the notch app plays its configured sound. Unavailable sounds are skipped without blocking the notification. Custom adapters receive semantic keys and own their sound playback.

The v1 event schema is shared by all adapters:

| Field | Type | Meaning |
| --- | --- | --- |
| `v` | number | Protocol version, `1` |
| `id` | string | Tab identity within a tmux server, e.g. `ws:@12`; use for replacement |
| `source` | string | `ws` |
| `state` | string | `working`, `idle`, `waiting`, or `closed` (tab seen or agent exited: drop its notification) |
| `prev` | string | Previous tab state; empty if unknown |
| `title` | string | Notification title, currently `ws` |
| `message` | string | Workspace + status, e.g. `api/feat-login needs input` |
| `detail` | string | Hook message or prompt, truncated to 1,024 Unicode characters; empty if unavailable |
| `agent` | string | Agent name, `claude`, `codex` or `grok`; empty if unknown |
| `repo` | string | Repository name from the workspace |
| `branch` | string | Git branch name, preserving `/`; empty if unknown |
| `focused` | boolean | Whether the tab is active in an attached tmux session |
| `sound` | string | Semantic key: `done` for idle, `waiting` for input; empty for working and closed |
| `actions` | array | Objects with string `id`, `label`, `command`; currently one `focus` action labelled `Focus tab` |
| `seq` | number | Integer microseconds since epoch, taken at hook entry; per `id`, receivers drop events whose `seq` <= the last applied `seq`; events without `seq` are treated as newest |
| `ts` | number | Unix timestamp in seconds (UTC) |

`detail` uses the first nonempty string from `message` (Claude Notification), `last_assistant_message` (Claude/Codex Stop), or `prompt` (UserPromptSubmit). Quotes, newlines and Unicode are preserved within the length bound. Missing, non-text or malformed payloads still emit the transition with empty detail.

The `focus` action's shell command selects the tab, switches the tmux client to its session, and activates the terminal app (`open -b`, from `WS_TERMINAL_BUNDLE` or tmux's `__CFBundleIdentifier`). Custom adapters decide how to display events and handle actions; execute commands only from a trusted local source. Adapters should ignore unknown fields and reject unsupported protocol versions.

## Layout

`ws/ws.sh` is the entry point (env/config, command dispatch); it sources `ws/lib/`: `core.sh` (tmux wrappers, locks, window tagging), `workspace.sh` (worktree + session create/rm/list/merge), `tabs.sh` (agent tabs, launch/resume, per-agent commands), `hooks.sh` (`ws hook` ingest, hook config writers), `notify.sh` (notification adapters, notifier build), `picker.sh` (picker, key bindings), `bus.sh` (registry, send/inbox/delivery) and `triggers.sh` (done/on triggers). `ws/tests/` holds the suite (`zsh ws/tests/run.sh`): the `<area>.test.sh` files run in 4 groups (core, send, triggers, misc) in parallel, each in its own process where `helpers.sh` builds a sandbox (private HOME, tmux server, fake agents, real-HOME guard) and the group's areas are sourced in order, because workspaces and session ids carry across them. `zsh ws/tests/run.sh <group>` runs one group; `WS_TESTS_JOBS=1` runs every area sequentially in one sandbox.

## Rust commands

The Rust CLI in `wsd/` supports `whoami`, `name <label>`, `sessions [id|name]` and `show <id|name>`.
Each command accepts `--json`. Human output uses `#N` aliases. Targets accept aliases, names, ULIDs and unique ULID prefixes.
The `me` target selects the current session.
`whoami` checks `WS_SESSION_ID`, then `WS_SESSION`, then the tmux window option `@ws_session`.
`WS_TMUX_SOCKET` selects the tmux server.

JSON returns the registry session fields, including the ULID `id`, numeric `alias` and lifecycle `state`.
`sessions --json` returns an array. The other commands return one session object. `show --json` adds the `output` path.
Human output maps `finished` to `done`, `failed` to `closed`, and other lifecycle states to `active`.
The TSV state column reports the live tmux state or `stopped` when no tagged window exists.

The Rust CLI also supports `new [--repo <path>] [--branch <branch>] [--agent <agent>] [--detach]`,
`add [repo/branch|path] [--agent <agent>] [--detach]`, and `list|ls [--json]`.
`new` creates `$WS_ROOT/<repo>/<branch>` and replaces branch slashes with hyphens.
Without `--repo`, `new` offers the current repository, the read-only Loop mappings, and repositories under `$WS_REPO_ROOTS` through fzf.
Without `--branch`, it reads the branch from the terminal.
It copies ignored files that match `.conductor/settings.toml` `file_include_globs`, with `.env*` as the default.
It runs `[scripts] setup`, or an executable `.ws-setup`, in a separate tmux window.
`add` registers an existing worktree or adds a tab to a registered workspace.
When the target is absent, it uses the current directory or tmux workspace.
`list` selects workspaces from the registry. A worktree directory without a registry entry does not appear.
Workspace tabs start the selected agent through one preparation contract in `ws-daemon`.
`ws claude`, `ws codex` and `ws grok` run tracked agents in the current workspace tab and forward their arguments.
Each agent receives the same ULID in `WS_SESSION` and `WS_SESSION_ID`.
Claude receives a private settings file for each launch. Codex receives per-launch `-c hooks.*` flags and `--dangerously-bypass-hook-trust`.
Grok receives `${GROK_HOME:-$HOME/.grok}/hooks/ws.json`. All generated hook commands call the Rust binary's `ws hook` command.
Slots remain in `<git-dir>/ws-sessions`. A clean exit removes the slot. Other exits retain it for `ws open` and resume.
The hook configuration covers every agent event. Hook ingestion beyond `stop` remains a later port phase.
Run `cd wsd && cargo test -p ws --test agents && cargo test -p ws-daemon --test agent_preparation` to check launches.
The tests use fake agents and private tmux servers. Preparation tests also compare the existing zsh builders.

Run `zsh wsd/crates/ws/tests/golden/workspace-capture.zsh` to regenerate the zsh workspace fixtures.
Run `cd wsd && cargo test -p ws --test workspaces` to check the Rust workspace commands.

Run `zsh wsd/crates/ws/tests/golden/capture.zsh` from the repository root to regenerate the zsh fixtures.
The capture uses `ws/tests/helpers.sh` with a private HOME and tmux server.
Only ids and sandbox output paths are normalized. Run `cd wsd && cargo test -p ws --test session_commands` to compare the Rust output.

## Upgrading from the root-level `ws.sh`

ws moved from `~/dotfile/ws.sh` to `~/dotfile/ws/ws.sh`. After pulling, run `./install.sh ws` (re-links `~/.local/bin/ws`, re-points the alias source and runs `ws _migrate-hooks`, which rewrites `~/.grok/hooks/ws.json`, `$WS_HOME/claude-settings.json`, the `$WS_HOME/bin/ws` link and the tmux key bindings to the new path). Starting `ws` (bare, `new`, `add`, `open`, `pick`) migrates too, but only what is broken: a file whose embedded ws path is missing or not executable (and a dangling `$WS_HOME/bin/ws`), so a second ws checkout never steals hooks that still point at a working install. `ws _migrate-hooks` forces the re-point at the ws you run it from. Missing files are not created.

Agent tabs that were already running keep the old path baked into their hooks (Claude passes its settings at launch, Codex per-launch `-c hooks.*`) and into their launch wrapper. Once the old `ws.sh` is gone, those tabs stop updating status, and when their agent exits the wrapper's `_dismiss`/`_end` fail: the slot is not freed, the notification stays open and `ws on ... exit` triggers never fire (detached delivery retries from the old `_agent` fail too). Close and `ws open` such tabs before relying on any of that (resume keeps the conversation). New and resumed tabs pick up the new path automatically.
