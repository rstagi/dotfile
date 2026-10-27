# ws — terminal workspaces

Install with `./install.sh ws`, then run `ws` for the picker.
The launcher runs the Rust CLI and local daemon.
The [Rust command reference](#rust-commands) describes state, migration and automation.

- A workspace combines a git worktree and a tmux session. Each interactive agent uses a window in that session.
- `ws new [--repo <path>] [--branch <branch>] [--agent claude|codex|grok] [--detach]` creates a workspace.
- `ws new --scratch [--agent claude|codex|grok] [--detach]` starts a session in `$WS_HOME/scratch/<ulid8>`.
- `ws new --cwd <dir> [--agent claude|codex|grok] [--detach]` starts a session in an existing directory.
- `ws new` also accepts `--name <name>` and `--prompt <text>`. The prompt is the first message to the agent. Only an agent process (`WS_SESSION_ID` set) becomes the parent of the new session. A command that you type in a session pane starts a top-level session. `--no-parent` also makes an agent start a top-level session. The picker keys use it.
- `ws session reparent <session>... [--parent <session>]` moves sessions under a parent, or to the top level without `--parent`. It rejects a cycle. The change is a log event, so it survives a registry rebuild.
- `ws add [<workspace>|<path>] [--agent claude|codex|grok] [--detach]` adds an agent window.
- `ws open <workspace|path|session> [--detach]` focuses or resumes a workspace, directory place or session.
- If every session of a directory ended, `ws open <dir>` resumes its most recent session.
- `ws list [--json]`, `ws pin <workspace|path>` and `ws rm [<workspace|path>]` manage registered workspaces.
- `ws rm <dir>` stops the sessions of a directory place and closes their windows. It preserves files outside `$WS_HOME/scratch/`.
- Removal of a scratch directory always requires confirmation, including with `--force`. `--interactive` also confirms removal of other places.
- `ws pin <dir>` rejects directory places. Pins apply only to workspaces.
- `ws merge` merges the workspace PR through `gh`, then offers to remove the worktree and branch.
- `ws claude|codex|grok [args]` starts a tracked agent in the current workspace window.
- `ws sessions [<id>|<name>] [--json]`, `ws whoami [--json]`, `ws name <label>` and `ws show <target> [--json]` manage session identity.
- `ws sessions` shows the name, or `#N` without a name, in the first column. Child sessions show under their parent with `└ `.
- Session IDs are ULIDs. Targets accept names, unique ULID prefixes and quoted aliases such as `'#123'`.
- New agents receive the ULID in `WS_SESSION_ID` only. IDs survive resume.
- `ws send <target> <message|->` queues a durable message. Exit 75 means queued. Do not send the message again.
- A message to a finished session reopens it. `--force` remains accepted for compatibility.
- `ws inbox [--json]` lists unread messages. `ws ack [<event_id>...]` acknowledges selected messages, or all visible unread messages.
- `ws done [--output <file>|-]` publishes completion output. Unread messages refuse completion.
- `ws wait <target> --for done|exit|state=<state>|event=<type> [--timeout <seconds>]` waits through the daemon.
- `ws event <type> [--body <json-object>] [--source tool|agent] [--key <key>] [--target <target>]` appends a `loop.*` event to a session log. The default target is the current session. With `--key`, a session that already has an event with that key gets no new entry. The daemon returns the existing event with HTTP 200, and a new key gives HTTP 201. The key is stored in the entry body as `event_key`, so the rule also holds after a restart. A body cannot set `event_key`. Other event types, unknown `loop.*` types and a body that does not match its type get HTTP 400. The checked types are `loop.plan`, `loop.phases`, `loop.phase`, `loop.attempt`, `loop.merge`, `loop.milestone`, `loop.gate`, `loop.review`, `loop.note`, `loop.decision`, `loop.control`, `loop.guard`, `loop.violation` and `loop.finish`. `loop.attempt` and `loop.review` fields depend on `state`. The entry body records the caller in `caller` and the producer in `source`: `tool` with `--source tool`, `agent` otherwise, and `daemon` for events that the daemon appends itself. `loop.phases` lists `{n, title, kind, output, milestone, gate, repo, branch, depends_on, status}` per phase. `loop.gate` needs `milestone`. The daemon appends `loop.phase` (`source: daemon`) in the same operation as the attempt, review or merge event that raises a phase status. A merge gate that stays open gives the loop status `awaiting-merge`. Then a keyed create or a phase spawn with a `loop` object for a later milestone gets HTTP 409 (`milestone_gate_open`).
- `ws loop create --dir <loop dir> [--plan-id <id>] [--json]` creates a loop session with no process. The daemon reads `planId` from the `<!-- loop-plan` header in `plan.md`. `--plan-id` overrides that value. A missing plan id gives HTTP 400. Repeated creates for the same plan id or canonical directory return the existing session, including after a restart.
- With a `loop` object, the daemon records execution events in the loop session. Phase attempts use `loop.attempt`, and review steps use `loop.review`. Each execution key gets one `started` and one `exited` event, both with `source: daemon`. An attempt uses `exit: -1` when its process status is unknown. Recovery repairs interrupted appends and suppresses duplicates.
- Agent preparation finds the loop through session parents and exports `WS_LOOP_ID`. It adds `ws hook guard` to `PreToolUse`. Sessions outside a loop receive neither.
- The guard records bypasses as `loop.guard {session, rule, command}` with `source: daemon`. Claude and Codex deny direct merges, integration pushes, PR merges and direct agent starts. Use `loop-merge.sh`, `phase_spawn` or `ws session create`. Grok records the same events without a denial. A review step with role `fix` can push to the integration branch.
- `ws log [<target>] [--type <prefix>] [--since <seq>]` prints the log entries as JSON, in order. `--since` keeps entries with a higher sequence number.
- `ws on <ids> done|exit --send-to <target> <text>` registers a continuation. `--spawn <prompt> [--agent <agent>]` starts a child instead.
- `ws triggers [--json]` lists pending actions. `ws on --rm <trigger-ULID>` removes one action.
- `ws mcp` serves local stdio MCP tools. Agent launches configure this server without changing Claude or Codex configuration files.
- `ws backend rust` imports any remaining legacy records. Rust is the only backend.

The picker uses fzf. Enter opens a workspace or directory place. Ctrl-N starts a Claude scratch session, Ctrl-O uses Codex, and Ctrl-R uses Grok.
Alt-N creates a Claude workspace from the repository picker, Alt-O uses Codex, and Alt-R uses Grok.
Ctrl-A adds Claude, Ctrl-T adds Codex, and Ctrl-V adds Grok. Ctrl-S shows sessions, Ctrl-P changes pins, and Ctrl-X removes workspaces.
The tmux bindings include `prefix+w` for the picker, `prefix+M` for merge and `prefix+B` for the agent priority of the current tab.
Picker rows show session names, or `#N` without a name. A place shows under the place of the session that created it.
Agent starts use permission bypass flags.
The [ws-sessions skill](../.claude/skills/ws-sessions/SKILL.md) describes agent coordination.
The [ws-orchestrate skill](../.claude/skills/ws-orchestrate/SKILL.md) runs work in one or more repos from a session without a repo.
`./install-agent-skills.sh` shares that skill with Claude, Codex and Ratel Local.

## Session places and repos

A session's place and its repos are separate. Both are optional.
The place supplies its working directory (`cwd`) and optional workspace (`workspace_id`).
A workspace combines a git worktree and a tmux session.
A directory place uses an existing directory without creating a git worktree.
A scratch place uses a new directory under `$WS_HOME/scratch/<ulid8>`.

```sh
ws new --repo /path/to/repo --branch feat/task --detach
ws new --cwd /path/to/notes --detach
ws new --scratch --agent codex --detach
ws sessions --json
```

`--cwd` accepts directories inside or outside git repositories.
`--repo` also accepts a non-git directory when `--branch` is absent.
Directory and scratch sessions have no workspace. The picker still shows them.
Use `ws add <dir>` to add an agent and `ws open <dir>` to resume stopped sessions.
`ws rm <dir>` preserves external files. Scratch removal requires confirmation, including with `--force`.
Pins apply only to workspaces.

The registry derives `repos` from events without changing the session's `cwd`.
Workspace launches record the main checkout path and workspace branch through `session.repo_attached`.
The `workspace.created` event also records `repo` and `branch`.
Phase spawns record their execution repo and base through `session.repo_attached`.
`ws phase worktree` records the detached checkout path and reference through `phase.worktree`.
Its checkout path becomes a `repos` entry, with the reference as `base`.
Directory launches do not detect repos from their cwd or hooks.
The `cwd` and `repos` fields appear in JSON. Human and TSV output keep their existing format.

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

`ws/ws.sh` executes `${WS_RUST_BIN:-$WS_HOME/bin/ws-rs}`. It ignores old backend selectors.
The Cargo workspace in `wsd/` contains four crates:

- `ws-core` stores durable data.
- `ws-daemon` provides the runtime.
- `ws-hub` holds the device identity code (and later the hub server and sync types).
- `ws` provides the CLI and embeds `wsd/crates/ws/src/cmd/picker.zsh` for the fzf UI.

Device identity: `ws ca init --out <dir>` writes the Ed25519 root key (0600) and a 10-year root certificate.
`ws device init` writes `$WS_STATE/device/key.pem` (0600) and `csr.pem` for the id in `$WS_STATE/device.json`.
`ws device sign --csr <file> --root-cert <file> [--root-key-cmd '<cmd>'] [--days 90]` prints the device certificate.
Hub: `ws hub serve --listen <addr> --tls-cert <file> --tls-key <file> --ca-cert <root cert> --state <dir>` serves the hub over mTLS and prints `hub ready <addr>`.
Each `/v1/` route needs a client certificate from the root CA whose fingerprint is an unrevoked row in `<state>/hub.db`. A failed registry check gives HTTP 403.
`ws hub add-device --state <dir> --cert <file> --name <name>` adds the first device on the hub host. `ws device list [--json]` and `ws device remove <id>` call the hub with the `[hub]` section (`url`, `cert`, `key`, `ca`) of `config.toml`. Without it they print one error line and exit 2.
Pairing: `ws device join <hub url> --name <name> [--ca <file>]` runs `ws device init`, sends the request to `POST /v1/pair`, prints an 8-character code and waits for the certificate (the default trust is the system roots). It then writes `$WS_STATE/device/cert.pem` and `root.pem` and appends the `[hub]` section to `config.toml`.
On a device that the hub has, `ws device approve <code> --root-key-cmd '<cmd>'` signs the request and sends the certificate. The hub adds the device to the registry. A code works one time and expires after 10 minutes. A used, unknown or expired code gives HTTP 404. An approve from a device that the registry does not have gives HTTP 403.
It reads the root key from the stdout of the command (default `op read op://ws/root-ca/key`).

Rust owns sessions, messages, logs, dependency edges and the local MCP server.
Run `cd wsd && cargo test` to test the CLI and daemon.
Run `zsh ws/tests/run.sh` to test the picker shell glue with a private tmux server.
`tests/run.sh` also runs this smoke test.
`ws/tests/picker-rust.test.sh` supplies the shell checks for the Rust picker integration test.
Run `cd wsd && cargo test -p ws --test backend` to test the launcher, startup import, messages, waits and MCP.

## Rust commands

`./install.sh ws` uses the `rustup` on `PATH`. When `PATH` has no `rustup`, the installer installs Homebrew `rustup`.
Then it selects the stable Rust toolchain.
A package install such as `./install.sh ws` does not run `brew upgrade`. Only the full interactive install (`./install.sh`) upgrades all brew formulae.
The installer skips a step that it did before, such as the tpm clone. It keeps a different `~/.tmux.conf` and prints a warning.
It builds `wsd/` with `cargo build --release` and copies the binary to `$WS_HOME/bin/ws-rs` (default `~/.ws/bin/ws-rs`).
Both `$WS_HOME/bin/ws` and `~/.local/bin/ws` point to the shell dispatcher.
The installer registers `~/Library/LaunchAgents/com.rstagi.wsd.plist` with `RunAtLoad` and `KeepAlive`.
The agent runs `$WS_HOME/bin/ws-rs daemon`. It retains the installer's `HOME`, `PATH`, `LANG`, `WS_HOME`, `WS_STATE`, `WS_SOCK`, `WS_ROOT` and `WS_TMUX_SOCKET`.
`LANG` defaults to `en_US.UTF-8`. The daemon also starts each tmux client with `-u`. Thus tmux replies use UTF-8 when the daemon has no locale.
`WS_ROOT` defaults to `~/.ws/worktrees`. An empty `WS_TMUX_SOCKET` selects the default tmux server. A value with `/` selects a socket path with `tmux -S`. Other values select a server name with `tmux -L`.
Its output goes to `$WS_HOME/daemon.log`. Run `./install.sh --dry-run ws` to preview each step without changes.

A Rust CLI request starts a background daemon when its socket is missing and launchd does not own `com.rstagi.wsd`.
Concurrent starts share a startup lock. A failed start reports the log path beside the socket.
The daemon exits when its socket file disappears or a different file replaces it.
State callbacks retain their bounded spool fallback when the daemon is absent. Notification dismissal remains best effort.
A stop or exit hook waits 5 s for the daemon reply. Other hooks wait 250 ms.
When the time expires, the hook spools the callback and prints `{}`. Test sandboxes set this bound with `WS_HOOK_TRANSPORT_DEADLINE_MS`.
The guard waits up to 4 s for the daemon. It never starts a daemon. A failed guard request leaves the tool call unchanged.
The daemon can record a block before a stop hook times out. The batch then arrives at the next stop hook. Until then, the session shows `working`.

The launcher always uses Rust. `WS_RUST_BIN` overrides the binary path for tests.
`ws backend rust` remains available to import remaining legacy sessions, messages and triggers.
It retains existing Rust session IDs. It does not enable another backend.
Alias allocation retains `sessions/counter` and the compatible `counter.lock`.

The Rust `ws pick` command and bare `ws` use the fzf UI in `wsd/crates/ws/src/cmd/picker.zsh`.
Rows, summaries, previews, pins, session expansion and unread changes use Rust commands.
The binary embeds the shell UI. It does not need the source checkout at runtime.
The tmux keys keep the picker, new workspace, agent tab and merge actions.
Bindings retain the configured state paths and tmux socket. They preserve user hooks at index 0.

On its first start with empty Rust state and an existing `$WS_HOME/sessions`, the daemon imports legacy state before serving requests.
It imports sessions, pending messages, pending triggers, listed branch workspaces and pins.
The import journal makes startup import resumable. A completed startup import does not read the source again.
Existing Rust logs prevent an automatic import when no journal exists.
The source files remain available as legacy import inputs. Rust is the only backend.
Rust state lives in `$WS_STATE` (default `~/.ws/state`). The socket uses `$WS_SOCK` (default `~/.ws/wsd.sock`).

`ws migrate --from-zsh [--dry-run]` manually imports `$WS_HOME/sessions` into `$WS_STATE` (default `~/.ws/state`).
Stop the Rust daemon before an import. The importer takes the zsh counter, names, output, inbox, metadata and event locks.
A busy lock stops the import with an error. Retry after the writer releases that lock.
Each legacy session receives a ULID and keeps its `#N` alias. New aliases start above `sessions/counter`.
The import copies completion output and prompts. It preserves pending inbox, Stop and unconfirmed paste messages as undelivered events from the human session.
`$WS_STATE/import-zsh.json` stores the snapshot, session ids and the map from each original event line to its new sequence number.
An interrupted import resumes from that snapshot. A completed import makes subsequent runs read-only.
`--dry-run` prints the JSON import plan and creates no files. The source artifacts remain in place.

The Rust CLI in `wsd/` supports `whoami`, `name <label>`, `sessions [id|name]` and `show <id|name>`.
Each command accepts `--json`. Human output uses `#N` aliases. Targets accept aliases, names, ULIDs and unique ULID prefixes.
The `me` target selects the current session.
New launches set `WS_SESSION_ID` to the session ULID. They remove inherited `WS_SESSION` values.
The CLI still reads `WS_SESSION` for agents that started before the Rust cutover.
`whoami` checks `WS_SESSION_ID`, then `WS_SESSION`, then the tmux window option `@ws_session`.
Legacy windows and `WS_SESSION` can contain a bare numeric alias.
Session lists, picker rows and child creation resolve that alias.
`WS_TMUX_SOCKET` selects the tmux server by name or socket path.

JSON returns the registry session fields, including the ULID `id`, numeric `alias`, lifecycle `state`, `cwd` and `repos`.
`cwd` is null when the session has no recorded directory. Legacy imports use their recorded `path` as `cwd`.
`repos` contains entries with `path` and optional `remote`, `branch` and `base` fields.
The registry merges repo entries by path. A `phase.worktree` event records its checkout path and uses its `reference` as `base`.
`sessions --json` returns an array. The other commands return one session object. `show --json` adds the `output` path.
Human output maps `finished` to `done`, `failed` to `closed`, and other lifecycle states to `active`.
The TSV state column reports the live tmux state or `stopped` when no tagged window exists.

`ws done [--output <file>|-]` writes `$WS_STATE/sessions/<ULID>/output.md` and appends a `done` event.
Omitted output produces an empty file. `-` and `--output -` read stdin. Repeated calls replace the output.
Unread messages refuse completion. Completion marks the session `finished`. Process exit remains a separate event.
The supervisor and interactive launch wrapper record `exit`. `ws hook exit` also records exit callbacks.
When the interactive launch wrapper reports the agent exit, a session that is not finished or failed becomes `finished` for code 0 or `failed` for another code.
A failed exit keeps its slot row, so a later message resumes the conversation.
Claude receives a generated `SessionEnd` hook. A `SessionEnd` payload selects the exit event. An `exit_code` field supplies its code.
An exit callback still records the event when tmux cannot read the pane. A spooled callback succeeds at the next replay.

`ws wait <id|name> --for done|exit|state=<state>|event=<type> [--timeout <seconds>]` waits through one daemon request.
It prints no output on success. A timeout exits 1. Without `--timeout`, the wait has no deadline.
Recorded events satisfy a wait immediately, including after a daemon restart.
After a session reopens, only events after its latest `session.reopened` event satisfy an event condition.
State conditions check the current lifecycle state.
The API is `POST /v1/sessions/{id}/wait` with `{"for":"done","timeout":30}`. Omit `timeout` for an indefinite wait.
It returns the session and the matching event, or a null event for a state condition. A timeout returns HTTP 408.
`ws wait <target>... --mode any|all --for <condition>` waits for several sessions. The default mode is `all`.
Repeat `--after-seq <seq>` once per target, in target order. Without this option, every target uses zero.
Only events with `seq > after_seq` count. A state condition requires a matching state event after that sequence.
A zero sequence keeps the existing current-state check.
The multi-target API is `POST /v1/wait` with `{"targets":[{"target":"<id>","after_seq":10}],"mode":"all","for":"done","timeout":30}`.
MCP `wait` accepts the same `targets` and `mode`, with `condition` in place of `for`.
The response is `{"matches":[{"session":...,"event":...}]}`, in target order.
Mode `any` returns the targets that match. Mode `all` requires every target to match at the same check.
The single-target API also accepts `after_seq` and keeps its existing response format.

Daemon shutdown releases pending waits with HTTP 503.
`POST /v1/sessions/{id}/done` accepts `{"output":"completion text"}`. Both routes accept `parent_session_id` for caller audit.

`ws on <id,id,...> done|exit --send-to <id|me> [--template <text>]` registers a one-shot action.
`ws on <ids> done|exit --spawn <claude|codex|grok> [prompt]` starts a tracked child in the owner's workspace or directory.
The legacy forms `--send-to <id> <text>` and `--spawn <prompt> [--agent <agent>]` also work.
Omitted text uses a completion message with `{id}` and `{output}`. Expansion preserves literal text and source order.
Templates must contain text or `{id}` in addition to `{output}`.
Done triggers accept earlier completion events. Exit triggers require a new exit after registration.
A self-send waits for the launch wrapper to retire before it reopens the session.
All sources must satisfy the condition. Repeated events and daemon restarts do not repeat a completed action.
The owner log stores `edge.action_registered` with the sources, offsets, dependency edges, and action.
A terminal caller uses the local human session as the owner. A terminal spawn uses the current workspace or directory.
A send records `message.sent` in that owner's log.
A spawn records its claim before launch. Failed actions record `edge.action_failed` and do not retry.
Trigger changes use the daemon's serialized store, without trigger lock files.
`ws triggers [--json]` lists pending actions. Human output keeps the six-column TSV format.
`ws on --rm <trigger-ULID>` removes a pending action and its projected edges.
`ws done` closes a spawned child's tab after it publishes the output.
The API uses `POST /v1/triggers`, `GET /v1/triggers`, and `POST /v1/triggers/{id}/remove`.
Registration accepts `sources`, `event`, either `target` or `spawn`, optional `template`, `workspace`, `cwd`, and `parent_session_id`.
A spawn without a workspace retains the owner's `cwd` in its trigger record. Older trigger records can omit `cwd`.
A terminal spawn supplies `workspace` or `cwd`. Removal accepts the caller context.

Dependency edges use `POST /v1/sessions/{id}/depends_on` or `/block_on` with `{"target":"<id|name>"}`.
`GET /v1/edges` returns the projected graph. A cycle returns HTTP 409 and the cycle's session IDs.
The source log records `session.depends_on` or `session.block_on`. The registry rebuild restores these edges.

`ws session create --headless [--parent <id>] [--cwd <dir>] [--inherit-env] [--env K=V]... [--stdin <file>] [--stdout <file>] [--stderr <file>] -- <cmd>`
starts a process under the daemon and prints its session ULID. The process leads its own process group.
The working directory defaults to the CLI's current directory. Relative I/O paths use that same CLI directory, independent of `--cwd`.
Environment overrides extend the daemon environment. Repeated keys use the last value.
`--inherit-env` reads the CLI process environment and sends it over the Unix socket.
Explicit `--env K=V` assignments override inherited values, regardless of flag order.
The daemon does not persist these environment values in its registry, events, spans or diagnostic logs.
A command can still write its own environment values to its output files.
The process receives its session ULID in `WS_SESSION_ID` only.
Claude, Codex and Grok use the shared agent preparation contract in headless mode.
Stdin defaults to EOF. Stdout and stderr default to `$WS_STATE/sessions/<id>/stdout.log` and `stderr.log`.

Explicit output files replace their previous contents. I/O files must refer to separate files, including through links.
`ws session tail <id> [--follow] [--stream stdout|stderr]` reads stdout by default.
With `--follow`, it streams new bytes until the process exits, then reads the final bytes.
The daemon records an `exit` event and `session.finished` for code 0, or `session.failed` for another code.
Signal exits use code `128 + signal`. `GET /v1/sessions/{id}` returns `exit_code`, which is null before exit.
`POST /v1/sessions/{id}/stop` sends SIGTERM to the headless process group.
After a 10-second grace period, it sends SIGKILL if the group still exists.
The response waits for the entire group to disappear and for the daemon to record the exit.
For an interactive agent, `ws session stop <id>` closes its owned tmux window and frees its slot.
The daemon records an `exit` event and marks the session `finished`.
`ws session run [--headless] …` accepts the same command and I/O options as `session create --headless`.
It waits in the foreground and returns the child exit code. Output stays in the session log files.
Use `ws session run --inherit-env -- <cmd>` to keep inherited environment values out of process arguments.
The Loop runner and orchestrator use this flag when the installed CLI lists it in `session run --help`.
They retain `--env K=V` forwarding for older binaries. Install milestone 4 to remove that argument exposure.
SIGTERM, SIGINT and SIGHUP stop the session group. The client waits for the recorded exit before it exits.
The daemon persists the process id, process group and process start identity in `session.running`.
After a restart, it adopts each matching live headless process and polls its liveness.
A missing process or a changed identity produces `session.failed` with a reason.
An adopted process has no waitable exit status. Its eventual exit produces a failure with a null `exit_code` and a reason.
Run `cd wsd && cargo test -p ws --test headless` to check the I/O contract with sandbox processes.

Interactive window bindings use `session.window` events. The daemon also reads older workspace window records.
For directories outside a repository, slot files use `$WS_STATE/places/<hash>/ws-sessions`.
Git worktrees keep their slot file in the git directory.

`ws agent prepare --agent claude|codex|grok --mode headless --session <ULID> [--resume <id>] --format json|sh`
prepares an agent command for an external runner. It uses the same builder as interactive sessions.
It writes private launch files without contacting the daemon or starting an agent.
Claude and Codex receive the `ws` MCP server in fresh and resumed launches, in both modes.
Claude receives an extra `--mcp-config` file. Codex receives `-c mcp_servers.ws.*` overrides.
These additions preserve other MCP servers and do not edit the agent configuration files.
The MCP child runs the same Rust binary with `mcp`. Its caller is the prepared session.
Codex forwards `WS_SOCK` to the MCP child when that variable is set.
JSON contains `program`, `argv`, `env`, and `files` (each file has `path` and `contents`).
The arguments include `-p` for Claude and `exec` for Codex, plus any resume arguments.
The caller supplies its prompt and other flags. Put a Claude positional prompt before variadic flags such as `--allowedTools`.
Shell output sets `WS_AGENT_PROGRAM`, exports the environment, and sets positional parameters to the prepared arguments.
`WS_AGENT_FILES` contains shell-quoted generated paths. The caller owns these files after the agent exits.
For example, a POSIX shell runner can use a prompt on stdin:

```sh
eval "$(ws agent prepare --agent claude --mode headless --session "$WS_SESSION_ID" --format sh)"
printf '%s' "$prompt" | "$WS_AGENT_PROGRAM" "$@" --allowedTools Read Bash
```

Hooks without a pane use `WS_SESSION_ID` and retain each session's sequence and turn guards across daemon restarts.
They record the same turn and tool events without accessing tmux.

`ws send <ULID|name|#N> <message|->` records `message.sent` in the sender's session log.
`-` reads stdin and preserves trailing newlines. Calls outside a session use one persistent `human` session per device.
The envelope `event_id` identifies the message. Its body contains `from_session`, `to_session`, `pair_seq`, and `body`. The inbox resolves the sender alias from the log owner's identity.
Pair numbers start at 1 for each sender and recipient. A gap holds later messages from that sender.
`ws inbox [--json]` shows unread messages for the current session, or the device's human session.
Reading the inbox does not acknowledge messages. The text header uses event metadata, and body lines cannot forge a header.
`ws ack [<event_id>...]` writes `message.read` events in the recipient log. With no ids, it acknowledges all visible unread messages.
Repeated acknowledgements do not add read events. Unknown ids, foreign messages and messages behind a gap are rejected.
These commands use `POST /v1/messages`, `GET /v1/inbox?session_id=<ULID>`, and `POST /v1/ack`.
Omit `session_id` to use the human session. The send caller uses `parent_session_id`.

The Rust bus returns exit 75 with a queue warning. A live Stop hook returns `{decision:"block", reason:<batch>}` for undelivered messages.
The batch follows `pair_seq` order for each sender. Its headers use sender metadata.

After the hook writes and flushes its reply, it confirms delivery through `POST /v1/hooks/delivered`.
The daemon writes `message.delivered` in the recipient log. Failed confirmations remain in `$WS_STATE/delivery-spool/` for background replay or before the next hook.
A hook batch stays provisional until the next Stop. If that Stop reports `stop_hook_active: false`, the daemon returns the batch again.
If the client does not confirm a block and no receipt exists in the spool, recovery returns the same window to `idle` after the CLI budget.
The idle paste worker then submits the pending inbox. Recovery checks ownership and the sequence before it changes the window, so it preserves newer turns.

For an idle agent window, the daemon pastes a durable batch into the recorded agent pane with bracketed paste and a separate Enter.
A new Claude window gets no paste until its first hook. Claude receives a generated `SessionStart` hook (`ws hook ready`), and Claude runs hooks only after its "trust this folder" dialog.
A paste into that dialog selects its default answer, "No, exit". The message stays queued until the human accepts the dialog. Claude also trusts a folder when it trusts a parent folder.
Other agents wait for their first hook or for the pane to enable bracketed paste.
A newer `UserPromptSubmit` confirms the batch and writes `message.delivered`. Tool hooks and older prompts do not confirm it.
One daemon worker retries queued messages and requeues unconfirmed batches after `WS_PASTE_ACK_TIMEOUT` seconds (default 10).
Pending batches survive daemon restart. The worker replaces the detached zsh retry and paste timeout processes for the Rust backend.
`ws send` still returns exit 75 after it durably queues a message.

A message to a finished or failed local agent session appends `session.reopened`.
If its agent still runs in its owned pane, delivery uses that process.
Otherwise, the daemon starts a new tmux window before delivery.
Claude uses `--resume <id>`. Codex uses `resume <id>`.
Durable hook records retain the conversation ID after a clean exit removes its slot.
A previous launch window loses its session tags after its agent exits, so its hooks cannot consume the new messages.
The Rust backend accepts `--force` before the target or after the message. The flag has no effect.
If the agent cannot resume, the message stays queued. Sessions on other devices do not launch locally.

The next accepted Stop or matching cancellation writes `message.read` with `implicit: true` for delivered messages that lack an explicit acknowledgement.
A Stop with `stop_hook_active: false` retains provisional hook deliveries instead. Older hook payloads that omit this field keep the implicit-read behavior.
Hook spool replay does not inject a new batch. Only the session's local device can inject messages.

Run `cd wsd && cargo test -p ws --test messages` to check persistence, ordering, acknowledgements and stdin.
Run `cd wsd && cargo test -p ws-daemon bus::delivery::tests` to check Stop replies, paste, timeout recovery and restart.

`ws mcp` serves a local stdio MCP connection through `rmcp`. It calls the daemon at `$WS_SOCK`.
Standard output contains MCP messages. Diagnostics use standard error.
Each call supplies `$WS_SESSION_ID` as `parent_session_id`. Tool arguments cannot replace the caller.
The tools return the daemon JSON in both text and structured content.

| MCP tool | JSON CLI command |
|---|---|
| `session_create` | `ws session create [--name <name>] --json`<br>`ws session create --headless --json … -- <cmd>` |
| `session_list` | `ws session list --json` |
| `session_rename` | `ws name <name> --json` |
| `session_get` | `ws session get <target> --json` |
| `session_send` | `ws send <target> <message> --json` |
| `session_output` | `ws session output <target> --json` |
| `session_tail` | `ws session tail <target> --json [--stream stdout|stderr] [--max-bytes <n>]` |
| `session_event` | `ws event <type> [--body <json>] [--source tool\|agent] [--key <key>] [--target <target>]` |
| `session_log` | `ws log [<target>] [--type <prefix>] [--since <seq>]` |
| `session_stop` | `ws session stop <target> --json` |
| `session_depends_on` | `ws session depends-on <target> <dependency> --json` |
| `session_block_on` | `ws session block-on <target> <dependency> --json` |
| `wait` | `ws wait <target>... --for <condition> [--mode any\|all] [--after-seq <seq>] [--timeout <seconds>] --json` |
| `loop_get` | `ws loop get <id> --json` |
| `loop_plan` | `ws loop plan <id> [--json] [--out <file>]` |
| `loop_check` | `ws loop check <id> --json` |
| `loop_ack` | `ws loop ack <violation id> --reason <text> --json` |
| `loop_list` | `ws loop list --json` |
| `loop_create` | `ws loop create --dir <loop_dir> --json` |
| `loop_start` | `ws loop start <loop_dir> --json` |
| `loop_stop` | `ws loop stop <loop_dir> --json` |
| `loop_pause` | `ws loop pause <id> [<phase>] --json` |
| `loop_resume` | `ws loop resume <id> [<phase>] --json` |
| `loop_accept` | `ws loop accept <id> --json` |
| `loop_reject` | `ws loop reject <id> --json` |
| `loop_note` | `ws loop note <id> <phase> <text> --json` |
| `loop_model` | `ws loop model <id> <phase> <chain> --json` |
| `loop_tier` | `ws loop tier <id> <phase> <standard\|max> [--rounds N] --json` |
| `phase_spawn` | `ws phase spawn --repo <path> --base <ref> --max-runtime <seconds> --json -- <cmd>` |

The record form accepts a name and a workspace ID. The headless form rejects both fields.
`ws session create --cwd <dir> [--name <name>] --json` starts an interactive Claude session in that directory.
MCP `session_create` accepts the same `cwd` and `name` fields. Relative directories use the caller's current directory.
This form calls `POST /v1/sessions/start`. It returns the session record and creates no workspace.
MCP `session_create` also accepts `scratch: true`, or `repo` with optional `branch`, like `ws new --scratch` and `ws new --repo <repo> --branch <branch> --detach`.
`repo` with `branch` calls `POST /v1/workspaces`. A git `repo` requires `branch`. A non-git `repo` starts a directory session.
`agent` (`claude`, `codex` or `grok`, default `claude`) and `prompt` (first message) apply to interactive starts.
Use only one of `cwd`, `scratch`, `repo` or `workspace_id`. The caller becomes the parent. The result is the session record.
A headless create and `phase_spawn` accept an optional `loop` object: `{loop, phase, attempt}` for a phase attempt, or `{loop, phase, repo, role, provider, round, inner, attempt}` for a review step. The CLI flags are `--loop <id> --phase <n> --attempt <n>`, plus `--repo --role --provider --round --inner` for a review step. With the object, the loop session is the parent and the key namespace. The daemon records the caller as `actor` in `session.created`. The new process gets `WS_LOOP_ID`, `WS_LOOP_PHASE` and `WS_LOOP_ATTEMPT`. Without the object, the caller is the parent.
MCP `session_rename` takes `{"name":"<name>"}` and renames the caller session only.

`ws session output <target>` reads the published `output.md` text. It prints the text without adding a newline.
MCP `session_output` and `--json` return `{session_id, output, truncated}`.
`GET /v1/sessions/{id}/output` accepts `parent_session_id` for caller audit.
The response contains the first 65536 bytes at most. It preserves complete UTF-8 characters at the limit.

Send keeps exit code 75 after it queues the message. JSON tail returns `session_id`, `stream`, `output`, and `truncated`.
It reads the final 8192 bytes by default. The byte limit must be from 1 to 65536.
It replaces invalid UTF-8 bytes and enforces the limit after decoding. JSON tail does not support `--follow`.
`GET /v1/sessions/{id}/tail` accepts `stream`, `max_bytes`, and `parent_session_id` as query parameters.
`POST /v1/sessions/{id}/events` takes `{type, body, parent_session_id}` and appends the event when `type` starts with `loop.`. `GET /v1/sessions/{id}/log` accepts `type` (prefix), `since` and `parent_session_id` as query parameters and returns `{session_id, entries}`.
Phase spawn requires `$WS_SESSION_ID`. Repeat `--caps-required <capability>` to record required capabilities.
Run `cd wsd && cargo test -p ws --test mcp` to test every tool against a sandbox daemon.

`ws top` opens `loop-top.mjs` for the current session. It checks the live loop state's
`wsSession` against the session and its parents. It uses the session's worktree when no link matches.
`--once`, `--worktrees`, and `--graph` pass to the dashboard.

`GET /v1/loops` returns the loop states. `GET /v1/loops/{id}` returns one loop state.
Both routes rebuild the state from the loop session's `loop.*` events and accept `parent_session_id` for caller audit.
Loop routes, MCP `loop_get`, and CLI loop controls accept a session ULID, a `#N` alias or a plan id.
The state includes `plan_id`. Older sessions use the header in their latest `loop.plan` text, then the header in `plan.md`.
Creation preserves these older sessions and their children and events.
The state contains the plan, phases, milestone, and records for attempts, merges, gates, reviews, notes, decisions, controls, guards and violations.
Each record has `at_ms`, the time of its event.
Phase status only increases: `todo < claimed < running < done < merged`. A stale phase event cannot lower the status.
The daemon checks Loop rules after each Loop event. It records each new result as a
`loop.violation` event with `source: daemon`. A hard violation blocks phase spawns and
keyed creates for that loop until the human acknowledges it. A soft violation never blocks.
An orchestrator continuation can still start. Each new hard result sends one desktop notification.
`ws loop get <id> --json` lists violation IDs. Use `ws loop ack <violation id> --reason <text>`
to record an acknowledgment as a `loop.control` event. An agent cannot acknowledge a hard
violation. MCP `loop_ack` takes `{"id":"<violation id>","reason":"<text>"}`.
`POST /v1/loops/ack` accepts the same fields and optional `parent_session_id` for the caller.

`GET /v1/loops/{id}/plan`, MCP `loop_plan` and `ws loop plan <id>` return the latest `loop.plan` event as
`{plan_id, engine, sha, text, seq, milestone}`. `engine` is the `engine:` value in the plan header, or null.
The loop states show the same `engine`. `ws loop plan` prints the text, or the object with `--json`, or
writes it to the `--out` file. An unknown loop or a loop with no plan gives HTTP 404 and exit code 3.
`ws loop create` appends the first `loop.plan` from `<loop dir>/plan.md` with `source: daemon`.
The `sha` of a `loop.plan` body is the lowercase SHA-256 hex digest of the exact UTF-8 bytes of `text`.
A wrong `sha` gives HTTP 400. A plan with the `sha` of the latest plan appends nothing and returns
that latest event (HTTP 200 on the event route).

MCP `loop_get` takes `{"id":"<loop session>"}`. MCP `loop_list` takes `{}`.
The CLI read commands print formatted JSON by default. `--json` prints compact JSON.

Skills-loop controls use `POST /v1/loops/{id}/{action}` and return the appended event.
The actions are `pause`, `resume`, `accept`, `reject`, `note`, `model` and `tier`.
They require a session of kind `loop`. The body accepts `parent_session_id` for caller audit.
MCP tools take `id` and the same control fields. They derive the caller from `WS_SESSION_ID`.

Pause and resume accept an optional `phase`. They append `loop.control` and write or remove
`control/pause` or `control/pause-<phase>`. A loop pause and a phase pause are independent.
The daemon refuses phase spawns and keyed creates for a paused loop or phase with HTTP 409.
Resume clears a pause. It does not advance a milestone gate.

Accept and reject close the open accept gate with a daemon-produced `loop.gate` event.
The event copies the gate's milestone, phase and artifacts. A missing open gate gets HTTP 409.
Note requires `phase` and `text` and appends `loop.note`.
Model requires `phase` and `model`, such as `codex:@sol+high`. It appends `loop.control` and writes
`control/model-<phase>`, which the runner reads for the next attempt.
Tier requires `phase` and `tier` (`standard` or `max`). Optional `rounds` accepts 1 through 5.
It calls `loop-plan-tier.mjs` to edit `plan.md`, then appends `loop.plan` with the text and SHA-256.
It refuses a review that already started, including a status in `state.json` or the session events.
`LOOP_PLAN_TIER_MJS` overrides the tool path in the daemon environment. The default is `~/dotfile/loop-plan-tier.mjs`.

`POST /v1/loops/start` and `POST /v1/loops/stop` accept `loop_dir` and optional `parent_session_id`.
The API requires an absolute directory with a `state.json` file and a nonempty `runId`.
The CLI and MCP resolve relative directories from their current directory.
Start runs `loop-orchestrator.sh` as a detached headless session, with the existing `sub/prompt-seed.md` when present.
It records `wsSession` and `sub/orch.pid`. It rejects an active orchestrator or a pending milestone merge.
Stop writes `control/pause`, which pauses the orchestrator and its phase runners.
Use the dashboard's `X` control to resume the loop.
Loop scripts default to `~/dotfile/`. `LOOP_ORCHESTRATOR_SH` and `LOOP_TOP_MJS` override the script paths.
Set the orchestrator override in the daemon environment. Set the dashboard override in the CLI environment.

The daemon reads spawn limits from `$WS_STATE/config.toml` when it starts:

```toml
[limits]
max_children = 8
max_depth = 3

[loop]
max_spawns = 500
```

Missing fields use these defaults. Zero prevents child creation. Restart the daemon after a configuration change.
Outside a Loop, the child count includes finished and failed children.
Within a Loop, only live children count toward `max_children`.
`loop.max_spawns` caps all creations and moves into a Loop, including terminal sessions.
The old engine uses the same rules, grouped by the `--run-id` of `loop-orchestrator.sh`.
The counts survive daemon restarts. A move out of a Loop does not restore its total spawn capacity.

Each root has depth zero. A child adds one to its parent's depth.
The daemon checks limits before it creates an interactive session, a headless session, or a phase.
A resume keeps the existing session and does not consume another child slot.
A self-exit spawn trigger owned by a direct child of a `loop` session creates another child of that Loop.
This keeps the depth constant across orchestrator continuations.
A reparent into a Loop checks its capacity and consumes one spawn.

A violation returns HTTP 429 with `error.code`, `error.message`, `error.parent_session_id`, and `error.limit`.
The codes are `max_children_exceeded`, `max_depth_exceeded` and `max_spawns_exceeded`.
The CLI exits with code 1 and writes this JSON error to standard error.
MCP returns the same error as structured content with `isError: true`.

The same file sets the CPU priority of agent processes on macOS:

```toml
[agent]
priority = "utility"   # background | utility | normal
```

The daemon starts each interactive and headless agent with `taskpolicy -c <priority>`.
All child processes of the agent, such as test runners and compilers, inherit this clamp.
`utility` is the default. It uses all cores when the Mac is idle, and your own apps get the CPU first.
`background` uses only the efficiency cores and throttles disk I/O, also when the Mac is idle. `normal` applies no clamp.
Processes outside an agent's process tree keep their priority. Examples are a shell in your own terminal, a plain tmux pane and Docker containers.
New sessions use the new value after a daemon restart.

`ws session priority <session> background|utility|normal|next` sets the priority of one interactive agent session.
`next` cycles background, utility, normal. `prefix+B` runs `next` for the session of the current tab and shows the result.
A running agent gets SIGHUP, as if its terminal closed. The agent starts again in the same pane and resumes its conversation.
Work in progress in that agent, such as a running test, stops. The priority is a log event, so it survives a registry rebuild.
Headless sessions keep the configured priority.

The daemon names an unnamed session from its first prompt:

```toml
[naming]
model = "haiku"     # "off" keeps the first name
program = "claude"  # a bare name resolves through PATH
```

The first prompt of the main agent sets a kebab-case name at once, for example `fix-picker-scratch-path`.
The name has at most 5 words and 40 characters. A `-2` to `-9` suffix keeps names unique.
Headless sessions, subagent prompts and slash commands do not set a name.
Then `claude -p --model <model>` proposes a better name in the background, with a 20 s deadline.
The daemon applies that name only if the session still has the first name.
The model process gets no user hooks, plugins or MCP servers. It receives at most 2000 characters of the prompt.

The daemon also accepts OTLP/HTTP exports at `/v1/logs`, `/v1/metrics` and `/v1/traces`.
It accepts `application/json` and `application/x-protobuf` on IPv4 localhost only.
Configure it in the same file:

```toml
[telemetry]
port = 0
retention_days = 30
sinks = []
native_tracing = false
content = false
```

These are the defaults. Port zero selects an available port.
`$WS_STATE/otlp.endpoint` contains the bound HTTP endpoint when the daemon is ready.
A nonzero port selects a fixed port. Restart the daemon after a configuration change.

The shared launch contract reads this endpoint for interactive agents and `ws agent prepare`.
Claude receives OTLP environment settings. Codex receives independent log and metric exporters through `-c` flags.
Both agents receive the session, parent, device and project resource attributes.
Preparation does not change the user's agent configuration files.
It works without a daemon when the endpoint file is absent, with no telemetry additions.

Native tracing is off by default. Set `native_tracing = true` to enable Claude beta tracing and the Codex trace exporter.
This also supplies `TRACEPARENT` with the trace ID from the session ULID.
Parent sessions retain separate trace IDs.
The contract removes inherited native-tracing and content switches before it applies these settings.
It does not enable prompt or tool-content logging in either agent.
`content = true` only permits the receiver to retain Codex tool arguments and output.
Email redaction stays enabled.

Preparation JSON includes `telemetry.agent_version`, `telemetry.settings_version` and `telemetry.version_covered`.
The version probe uses `--version`. If the probe fails, the agent version is null.
The settings versions match the captured S2 manifests: Claude Code 2.1.302 and Codex 0.159.2.
A different detected version sets `version_covered` to false. It does not prevent launch.
Contract tests compare setting names and values with those manifests.

The daemon builds one canonical turn span from each accepted prompt and Stop pair.
It builds child tool spans from matching tool-use IDs and hook results.
An empty Codex hook result leaves the tool status unset, regardless of native `success`.
An interruption, process exit or abandoned session closes open spans with an error status.
A missing tool result closes the tool with an error when its turn closes.
Restart replay retains span IDs and does not append a completed span twice.
The receiver removes the known S2 native turn and tool boundaries to prevent duplicate spans.
It retains native model, transport and permission-wait timing.

Durable session events also produce OTLP logs using [WS semantic conventions v1](../docs/ws-v6/semconv-v1.md).
Each log record retains the source `event_id`, timestamp, epoch and sequence, with `event.domain=ws.session` and `ws.semconv.version=1`.
Events inside open turn or tool spans also appear as span events with the same source identity.
Replay does not append these records twice.
These OTLP logs omit message text, prompts, trigger templates and tool content unless `content = true`.
They always omit `user.email`.

The receiver replaces incoming `ws.device.id` resource attributes with the local device ID.
It appends one durable record per log record, metric or span to `$WS_STATE/telemetry/log.jsonl`.
Each record has an event ID, its receipt time, the signal type and an OTLP export payload.
The receiver removes `user.email` from all signals before storage.
It also removes `arguments` and `output` from `codex.tool_result` records unless `content = true`.
Other fields preserve the resource, scope, schema URLs and signal data.
The receiver syncs each accepted batch before it returns HTTP 200.
It repairs incomplete crash tails before the next append.
Retention uses receipt time. The daemon prunes expired records at startup and every minute.
Pruning replaces the file atomically under the append lock. It preserves retained event IDs and order.

Set `sinks = ["http://127.0.0.1:4318"]` to forward retained telemetry to OTLP/HTTP sinks.
Each entry is a base URL. The daemon appends `/v1/logs`, `/v1/metrics` or `/v1/traces`.
HTTP and HTTPS are supported. Restart the daemon after a sink change.
Each sink receives data independently. An unavailable sink does not block other sinks.
The daemon saves each sink's cursor per source device in `$WS_STATE/telemetry-cursors.json`.
It commits a cursor by atomic replacement only after the sink acknowledges the batch.
A sink without a cursor receives the full retention window. Expired records cannot be recovered.
Delivery can repeat after a lost response or a daemon crash.
Log records keep their `event_id` attributes. A log without that attribute uses its durable telemetry event ID.
Spans keep their span IDs. The sink must deduplicate repeated records.

The daemon collects Codex, Claude and Grok quota every minute and at each phase boundary.
The shared launch preparation saves the account binding in a private `quota-binding.json` beside the session log.
A resumed WS session retains that binding after an account change. The file contains no credential.
The collector initializes `codex app-server`, then calls the read-only `account/rateLimits/read` method.
It also reads `token_count` quota snapshots from the session's associated rollout.
Rollout snapshots retain their original event time and a separate collection receipt time.
Unrelated rollouts cannot supply an account binding.

Claude launch settings install a status-line command that forwards JSON through `ws hook quota`.
The hook transport spools callbacks when the daemon is unavailable. Quota callbacks do not change the session's work state.
The reader also checks the headless session's stdout for optional `rate_limit_event` records.
Status-line used percentages divide by 100. Stream utilization fractions already use the stored 0–1 ratio unit.
The reader binds both sources to the launch account. Repeated callbacks do not refresh an unchanged reading.
A stream without quota records leaves both windows without a reported value.

The Grok reader initializes `grok agent stdio`, then calls the read-only `_x.ai/billing` extension.
It records the reported period type and dates. It does not invent a five-hour window.
A missing `creditUsagePercent` stays unknown. Zero on-demand balances do not establish the included allowance.
The launch binding keeps the OIDC user and principal context. Unverified authentication modes stay unknown.
The reader checks the account before and after each billing request.

For a window without a reported value, the collector sums completed canonical model spans from the local telemetry log.
It deduplicates records by trace and span identity and adds normalized input and output tokens once.
It groups tokens by the frozen launch account and runner. Cache and reasoning subsets do not count twice.
Coverage records the interval, observed record range and count, and gaps outside local WS telemetry.
Without provider boundaries, a known duration supplies an explicitly labeled rolling interval.
Without a known duration or boundaries, the token total stays unknown.
An estimate has source `estimated`. Without a calibrated denominator, both quota ratios stay unknown.

The collector writes durable `runner.usage.updated` events through the existing session writer and OTLP emitter.
Each reading retains its account, bucket, window duration, reset time and `reported` or `estimated` source.
Window duration selects the window: 300 minutes means five hours, and 10080 minutes means one week.
Other durations remain separate. Invalid or expired values remain unknown.
A Codex bucket that reports no window keeps both standard windows as unknown. A bucket that reports
a window lists only its own windows: a plan with a weekly limit only has no five-hour window.
Unknown values produce no numeric quota attribute.

`ws usage` shows each runner's quota windows with the account, bucket, remaining percentage, source and reading age.
`ws usage --json` returns a `windows` array from the daemon's read-only `GET /v1/usage` endpoint.
Each window has `runner`, `account_id`, `bucket`, `window`, `remaining_ratio`, `source`, `observed_at_ms`, `age_secs` and `status`.
`account_id` is the opaque key. `account_label` is a readable name: the email of the current Claude
or Codex login, or the email and team of the current Grok login. Other accounts have no label.
The daemon reads the label from the login files at most once a minute. It never stores the label and never reads a token value.
The response also retains `reset_ms`, `tokens` and telemetry `coverage` when present.
The daemon selects the latest observation per runner, account, bucket and window across session logs.
Receipt time does not replace observation time. Reported values take priority when observation times match.
A missing ratio has status `unknown` and a null `remaining_ratio`. An expired window has status `unavailable` and a null ratio.
A runner without readings has an explicit unavailable row with unknown age and source.
A row without a bucket is a fallback. It disappears when the provider names a bucket for the same account.
An unknown row disappears when a later report of the same account and bucket omits its window.

`loop-top` and `ws top` show the same quota in a panel. The CLI request has a 1.5-second timeout.
The panel shows `account_label`, else a short account id such as `…e7fe2cf30`, else the team or user of a Grok identity.
The panel disappears when `ws` is absent, fails, times out or does not support `usage`.

`POST /v1/phases/spawn` starts a local phase as a headless child:

```json
{
  "parent_session_id": "<session ULID>",
  "phase": {"command": ["/path/to/runner", "argument"]},
  "repo": "/path/to/local/checkout",
  "base": "main",
  "caps_required": [],
  "max_runtime": 1800
}
```

The command uses `repo` as its working directory. `base` and `caps_required` remain in the phase metadata.
The daemon records `phase.spawned` in the parent log and returns the child session.
`max_runtime` is a positive number of seconds. The daemon persists its absolute deadline with the process details.
At that deadline, the daemon sends SIGTERM to the process group, then SIGKILL after two seconds.
A runtime limit produces a failure even if the command handles SIGTERM and exits with code 0.
An adopted child keeps its original deadline after a daemon restart.

`ws phase worktree [--session <id>] <repo> <ref>` creates a detached review worktree and prints its absolute path.
`repo` is a local repository path. The default session is the current ws session.

The worktree lives at `~/.ws/phases/<session>/`, or under the parent of a custom `$WS_STATE`.
The session log records ownership. The worktree never appears in `ws list`.
The daemon force-removes the worktree and prunes its Git registration when the session finishes or fails.
A daemon restart retries interrupted cleanup. Cleanup errors do not prevent daemon startup.

Run `cd wsd && cargo test -p ws --test phases` to check phase launches, review worktrees, and restart recovery.

The Rust CLI also supports `new [--repo <path>] [--branch <branch>] [--agent <agent>] [--detach]`,
`add [repo/branch|path] [--agent <agent>] [--detach]`, and `list|ls [--json]`.
With a repository and branch, `new` creates `$WS_ROOT/<repo>/<branch>` and replaces branch slashes with hyphens.
Without `--repo`, `new` offers `scratch`, the current repository, the read-only Loop mappings, and repositories under `$WS_REPO_ROOTS` through fzf.
Select `scratch` or use `new --scratch` to create a session in `$WS_HOME/scratch/<ulid8>`.
Use `new --cwd <dir>` to start in an existing directory, including a repository.
`--cwd` and `--scratch` each conflict with `--repo` and `--branch`. They also conflict with each other.
For a repository selection, `new` reads the branch from the terminal unless `--branch` supplies it.
A directory outside git skips the branch prompt and starts a session without a workspace.
Workspace creation copies ignored files that match `.conductor/settings.toml` `file_include_globs`, with `.env*` as the default.
Workspace creation runs `[scripts] setup`, or an executable `.ws-setup`, in a separate tmux window.
`add` registers an existing worktree or adds a window to a workspace or an existing directory.
Directory sessions share one tmux session and allocate successive agent slots.
When the target is absent, `add` uses the current tmux window's `@ws_path`, then the current directory.
`list` selects workspaces from the registry. A worktree directory without a registry entry does not appear.
Workspace tabs start the selected agent through one preparation contract in `ws-daemon`.
`ws claude`, `ws codex` and `ws grok` run tracked agents in the current workspace tab and forward their arguments.
Agent launches use `WS_AGENT_SHELL` (default `zsh -ic`) to load shell configuration.
Each new agent receives its ULID in `WS_SESSION_ID` only. Preparation removes inherited `WS_SESSION` values.
Claude receives a private settings file for each launch. Codex receives per-launch `-c hooks.*` flags and `--dangerously-bypass-hook-trust`.
Grok receives `${GROK_HOME:-$HOME/.grok}/hooks/ws.json`. All generated hook commands call the Rust binary's `ws hook` command.
Slots remain in `<git-dir>/ws-sessions`. A clean exit removes the slot. Other exits retain it for `ws open` and resume.
The Rust `hook` command records every callback and its original payload in the session log.
It retains prompt, turn and tool identifiers. Receipt times remain separate from agent durations.
The daemon records callbacks before it reduces the window state. Repeated `working` callbacks stay separate log events.
The window options retain the zsh sequence, turn and background subagent rules. A blocked Stop keeps its turn active.
A failed transport writes the callback to `$WS_STATE/spool/`. The daemon serves its socket before background replay starts.
It scans the spool each second and gives new files priority over the startup backlog.
Older callbacks retain their events but cannot replace newer window state.
Every hook exits 0. Stop returns its decision as JSON. Stop and exit share one 20 s CLI budget across resolve, ingest and confirmation.
Generated Claude Stop and SessionEnd hooks and Grok Stop hooks have a 30 s parent timeout.
Each tmux command that the daemon runs for a hook has a 2 s deadline.
The Rust daemon sends the same desktop notification events through terminal-notifier, osascript, boringnotch, or a custom adapter.
Hooks retain their notification settings, including `WS_NOTIFY=0`, when the daemon replays their spool records.
Built-in banners and sounds require an unfocused tab in `idle` or `waiting`. Custom adapters and boringnotch receive every state transition.
`ws _dismiss <window|pane>` emits `closed` once for an open notification. It keeps the tab state.
`ws _build-notifier` creates the branded `$WS_HOME/ws.app` from `WS_NOTIFIER_SOURCE`.
Run `cd wsd && cargo test -p ws --test notifications` to check adapters with fake commands in a private HOME.
Run `cd wsd && cargo test -p ws --test hook_ingest --test tmux -- --nocapture` to check hooks and measure latency.
This run prints the p95 values of the full ingest path and the daemon-down spool path. It does not assert them.
Run `cd wsd && WS_ASSERT_LATENCY=1 cargo test -p ws --test hook_ingest --test tmux -- --nocapture` to assert both p95 values below 50 ms.
`WS_ASSERT_LATENCY=1` also enables the 400 ms bound for the slow tmux lookup test.
Only the exact value `1` enables latency assertions. An unset variable or any other value disables them.
The slow lookup test always checks the callback and the spool count.
Enable latency assertions only on an idle machine. Parallel loads increase scheduler latency above the limit.
Run `cd wsd && cargo test -p ws --test agents && cargo test -p ws-daemon --test agent_preparation` to check launches.
The tests use fake agents and private tmux servers. Preparation tests also compare frozen output from the retired zsh builders.

Run `cd wsd && cargo test -p ws --test workspaces` to check the Rust workspace commands.

Rust import and parity tests copy frozen zsh files from `wsd/crates/ws-daemon/src/import/fixtures/` into their sandboxes.
These tests do not run the legacy backend. Fixture markers substitute sandbox paths and the preview commit hash.

The golden fixtures are frozen. Their capture scripts used the retired zsh backend.
Only IDs and sandbox output paths are normalized.
Run `cd wsd && cargo test -p ws --test session_commands` to compare the Rust output.

## Upgrading from zsh

Run `./install.sh ws` to install the Rust binary and update the launcher links.
New and resumed agents receive launch settings from Rust.
The installer does not rewrite old global hook files.
Resume existing tabs with `ws open` to receive the current launch settings.
The daemon retains the one-time legacy import described above.
