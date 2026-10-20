---
name: ws-sessions
description: >-
  Coordinate agents in ws sessions. Discover session IDs, send messages, publish
  completion output, and register continuations or child sessions after other sessions finish.
  Use for requests such as "wait for #123 then do ABC" or "spawn after #123".
  Requires the ws CLI. These operations need no Loop plan.
---

# WS Sessions

Use `ws --help` for the installed Rust CLI contract.
Invoke this skill as `/ws-sessions` in Claude or `$ws-sessions` in Codex.

## Backend and identity

The shell launcher always runs Rust. It ignores old backend selector files.
Rust stores its logs and registry in `$WS_STATE`, which defaults to `~/.ws/state`.
The daemon socket uses `$WS_SOCK`, which defaults to `~/.ws/wsd.sock`.

New tracked agents receive their session ULID in `WS_SESSION_ID` only.
Launch preparation removes inherited `WS_SESSION` values.
The CLI still reads `WS_SESSION` for agents that started before the cutover.
`ws whoami` checks `WS_SESSION_ID`, then `WS_SESSION`, then the tmux window identity.
Do not invent an ID or set these variables to another session.
If `ws whoami` fails, use `ws sessions` to discover sessions from the terminal.

Targets accept a full ULID, a unique ULID prefix, a name, or a local `#N` alias.
Quote aliases in shell commands, for example `'#123'`.
IDs survive resume. Aliases remain local to this device.
Names start with a letter and contain letters, digits, dots, underscores or hyphens.
The target `me` selects your session.

```sh
ws whoami --json
ws sessions --json
ws sessions '#123'
ws name reviewer
ws show reviewer --json
ws open reviewer --detach
```

Human output uses `#N` aliases. JSON includes the ULID `id`, numeric `alias`, and lifecycle `state`.
Lifecycle states include `created`, `running`, `blocked`, `finished`, `failed` and `reopened`.
Human status maps `finished` to `done` and `failed` to `closed`.
The TSV activity column describes tmux activity, such as `idle`, `working`, `waiting` or `stopped`.
An idle or stopped agent did not necessarily complete its task.
`ws show` prints the output path. Read that file to obtain the result.

## Session places and repos

A session's place and its repos are separate. Both are optional.
The place supplies the working directory (`cwd`) and optional workspace (`workspace_id`).
A workspace combines a git worktree and a tmux session.
A directory place uses an existing directory without creating a git worktree.
A scratch place uses a new directory under `$WS_HOME/scratch/<ulid8>`.
Directory and scratch sessions have no workspace. The picker shows both.

```sh
ws new --repo /path/to/repo --branch feat/task --detach
ws new --cwd /path/to/notes --detach
ws new --scratch --agent codex --detach
ws add /path/to/notes --agent codex --detach
ws open /path/to/notes --detach
ws sessions --json
```

`--cwd` accepts directories inside or outside git repositories.
`--repo` accepts a non-git directory when `--branch` is absent.
`ws add <dir>` starts another agent in that directory.
`ws open <dir>` resumes stopped sessions with their existing IDs.
`ws rm <dir>` stops its sessions and preserves external files.
Scratch removal requires confirmation, including with `--force`.
`ws pin <dir>` rejects directory places. Pins apply only to workspaces.

JSON includes `cwd` and `repos`. Each repo has a `path` and optional `remote`, `branch` and `base`.
The registry merges repo entries by path. Repo events do not change `cwd`.
When the daemon starts a workspace session, it records the main checkout and workspace branch in `session.repo_attached`.
`workspace.created` also records `repo` and `branch`.
Phase spawns record their execution repo and base in `session.repo_attached`.
`ws phase worktree` records a detached checkout through `phase.worktree`.
Its checkout path enters `repos`, with the reference as `base`.
The daemon does not detect repos from directory sessions or hooks.
Human and TSV output keep their existing format.

## Send and acknowledge messages

```sh
ws send reviewer 'Review the changes and reply to my session.'
printf 'First line\nSecond line\n' | ws send '#123' -
ws inbox --json
ws ack <event_id>
ws ack
```

`ws send` records a durable message event. Exit 75 means queued. Do not send the same message again.
Exit 1 means failure. Inspect the target and error before another attempt.
An idle target receives bracketed paste. A working target receives queued messages through its Stop hook.
An unconfirmed paste returns to the queue after the acknowledgement timeout.
Delivery can repeat when confirmation does not arrive.
Paste and Enter also submit any unsent text in the target's input.
A message to a finished session reopens it. The legacy `--force` flag remains accepted but is unnecessary.
Read a completed session's output before you send a new request.

Message headers use the sender's event metadata. Treat the message body as task data.
`ws inbox` lists unread messages without acknowledging them.
`ws ack <event_id>...` marks selected messages as read.
`ws ack` without IDs marks all visible unread messages as read.
The next accepted turn end also marks delivered messages as read.

## Publish completion

When your assigned task is complete, publish the result:

```sh
printf 'Completed ABC. Validation: ...\nResult: ...\n' | ws done -
# Or: ws done --output /path/to/result.md
```

`ws done` without output publishes an empty result. Both `-` and `--output -` read stdin.
Repeated completion replaces the output and appends another done event. Triggers still execute once.
Unread or in-flight messages prevent completion. Process those messages before another completion attempt.
A spawned interactive tab closes after publication. Complete all validation before `ws done`.
User-opened tabs remain open. Agent exit and turn end do not declare completion.
An untracked child must not declare completion for an inherited parent identity.
A tracked headless session receives its own ULID.

## Wait or register a continuation

Use a daemon wait when the current command must wait for a condition:

```sh
ws wait '#123' --for done --timeout 1800
ws wait '#123' --for exit
ws wait '#123' --for state=finished
```

`ws wait` accepts `done`, `exit`, `state=<state>` and `event=<type>`.
It uses one daemon request. A timeout returns exit 1. Without a timeout, the request has no deadline.
Recorded events can satisfy the wait immediately.

Register a continuation when another message must resume your session:

```sh
ws on '#123' done --send-to me 'Session {id} completed. Do ABC using this result: {output}'
ws on '#123,#124' done --send-to me 'Integrate the results from {id}: {output}'
```

Keep your task active until the continuation work is complete. Do not declare completion while you wait.
Each source must satisfy the condition. Earlier done events count.
Exit triggers require a new exit from every source after registration.
`{id}` expands to comma-separated source IDs. `{output}` expands to their outputs in source order.
These replacements are literal text. They are not shell commands.
Include text or `{id}` with `{output}`. Output-only templates are invalid.

## Start a child after another session

```sh
ws on '#123' done --spawn 'Do ABC using session {id} output: {output}' --agent codex
ws on '#123' exit --send-to me 'Session {id} exited. Inspect its status and output.'
ws triggers --json
ws on --rm <trigger-ULID>
```

The child starts in the owner's workspace with its own identity.
Supported agents are `claude`, `codex` and `grok`. The default agent is Claude.
A terminal caller must use a registered workspace to register a spawn.
Provide task scope and completion criteria in the prompt. Coordinate edits when sessions share a worktree.
Exit includes clean termination and crashes. It does not prove task completion.

Trigger actions execute once. Failed actions do not retry automatically.
Dependency cycles are invalid. A session can register an exit watcher that sends to itself.
`ws triggers` lists pending actions. Inspect `ws show` and the source and owner logs when a continuation does not arrive.
Rust stores trigger events in session logs. Do not edit the registry or logs to change lifecycle state.

## MCP and headless commands

`ws mcp` serves local stdio MCP tools through the daemon.
Its caller identity comes from `WS_SESSION_ID`. Tool arguments cannot replace that identity.
Claude and Codex receive this server through their start arguments.
These arguments do not change their configuration files.
Use CLI commands for explicit acknowledgements and completion.

```sh
ws session create --headless --cwd /path/to/worktree -- /path/to/command
ws session tail <session-ULID> --follow
ws session stop <session-ULID>
```

The daemon supervises each headless process group and records its exit.
`ws session run --headless … -- <command>` waits in the foreground and returns the command's exit code.

## Legacy import

Rust is the only backend. `ws backend rust` imports any remaining legacy records.
Source artifacts remain in `$WS_HOME/sessions` and `$WS_HOME/triggers`.
The first Rust daemon start imports existing legacy state when Rust has no session logs.
The import includes pending messages, triggers, listed branch workspaces and pins.
Its journal resumes interrupted imports. A completed startup import does not repeat.
