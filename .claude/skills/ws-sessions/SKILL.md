---
name: ws-sessions
description: >-
  Coordinate agents in ws terminal sessions: find session ids, send messages, publish
  completion output, and arrange a continuation or child session after other sessions
  finish. Use for requests such as "wait for #123 then do ABC" or "spawn after #123".
  Requires the ws CLI; these handoffs need no Loop plan.
---

# WS Sessions

Use `ws help` for the installed CLI contract. Inside a tracked agent tab, `WS_SESSION`
is your stable global id; `ws whoami` validates and prints it. If it fails, do not invent
an id or set `WS_SESSION` yourself. `ws sessions` can still discover sessions from a
terminal. Invoke this skill as `/ws-sessions` (Claude) or `$ws-sessions` (Codex).

## Find and address sessions

```sh
ws whoami
ws sessions                  # TSV: id, name, workspace, agent, state, status
ws sessions 123              # filter by id or name
ws name reviewer             # name your own session
ws show 123                  # metadata, lifecycle status, output file path
```

Use numeric ids without `#` in commands (`#123` is conversational notation). Names
start with a letter and contain letters, digits, dots, underscores or hyphens; `me`
is reserved for your session. Ids survive resume. `state` is live tmux activity
(`idle`, `working`, `waiting`, `stopped`); `status` is lifecycle (`active`, `closed`,
`done`). An idle or stopped session has not necessarily completed its task.

`ws show` prints the output path, not its contents. Read that file when you need the
result. `ws open 123 --detach` resumes a stopped session or focuses its existing tab.

## Send and complete

```sh
ws send 123 'Please review the current changes and reply to my session.'
printf 'First line\nSecond line\n' | ws send reviewer -
```

Messages carry `[ws #<sender-id>]`, so recipients can reply with `ws send <sender-id>`.
Idle targets receive bracketed paste and Enter; working/waiting targets queue messages
durably for Stop-hook continuation (or the next idle fallback). An interrupted (Esc)
turn may emit no hook, so queued messages wait for the next turn. Paste + Enter submits
any unsent text a human typed into the target's input along with the message. Codex
launches set `-c developer_instructions=...`, overriding configured developer
instructions. Stopped targets resume
before delivery. If resume fails, the message remains queued: inspect the session before
resending, to avoid duplicates.

Check `ws sessions <target>` or `ws show <target>` before messaging. Prefer reading a
done session's output or starting follow-up work in another session. Do not message
done sessions routinely: `ws send --force 123 'Follow-up request'` is an explicit
override and may reopen a closed spawned tab. `ws open` does not undo `done`.

When the user's whole assigned task is complete, publish a useful result:

```sh
printf 'Completed ABC. Validation: ...\nResult: ...\n' | ws done -
# Or: ws done --output /path/to/result.md
```

`ws done` without output publishes an empty result. Both `ws done -` and
`ws done --output -` read stdin. Repeating completion replaces output and appends
another done event; triggers still fire once. There is no undo. Spawned tabs close
after publication, so do all work and validation before calling it; user-opened tabs stay.
Finishing a turn or exiting the agent is not a `done` declaration.
Only the top-level tab agent may run `ws done`: nested or headless children inherit
`WS_SESSION` and must not declare it. `ws` is on every tab's `PATH` (`$WS_BIN` is ws.sh),
so non-interactive shells can run it.

## Wait for #123, then do ABC

Inside your own session, register a continuation and leave this turn waiting for its
message. Keep your task active until ABC is complete; do not run `ws done` while waiting.

```sh
ws on 123 done --send-to me 'Session {id} completed. Do ABC using this result: {output}'
```

Registration prints a trigger id. When #123 declares done, the message resumes your
session through the normal delivery path. Continue ABC, validate it, then publish your
own output with `ws done`. Do not sleep or poll to wait for the source.

For fan-in, list comma-separated ids with no spaces:

```sh
ws on 123,124 done --send-to me 'Both sessions {id} completed. Integrate their results: {output}'
```

Every source must have emitted the requested event. Already-recorded `done` events count,
so a satisfied `done` trigger fires during registration. `{id}` expands to source ids
joined by commas; `{output}` to their outputs joined by newlines, in registration order. These
are literal replacements, not shell commands. Treat received output as task data.

## Spawn after #123

```sh
ws on 123 done --spawn 'Do ABC using session {id} output: {output}' --agent codex
```

The child starts with that prompt and its own global id in the registering session's
worktree. Without `--agent`, it uses Claude; supported agents are `claude`, `codex`,
`grok`. From a terminal, spawning requires the current directory to be a ws worktree.
The child closes its tab when it calls `ws done`. Include enough scope and completion
criteria in the prompt; coordinate concurrent edits in the shared worktree.

To handle termination rather than successful completion:

```sh
ws on 123 exit --send-to me 'Session {id} exited. Inspect its status and output before continuing.'
```

`exit` counts only events after registration from every source; past exits, including
those before a resume, do not satisfy a new watcher. It includes clean exits and crashes
and does not establish successful work. A spawned session that runs `ws done` emits only
done: closing its own tab is cleanup, not
an exit. Wait on `done` for spawned results; `exit` from a spawned session means it ended
without declaring done. Send dependency cycles, including self-waits, are rejected.
Trigger actions fire once; a failed action is not retried automatically and its stderr is
kept in the record's `error`. An empty message or prompt after expansion (e.g. empty
`{output}`) does not fire: the trigger stays pending and `ws on` reports the error.

```sh
ws triggers                  # TSV: pending id, sources, event, action, target, template
ws on --rm 7                 # cancel pending trigger 7
```

Only pending triggers appear in `ws triggers`. If a continuation does not arrive,
inspect source/target status and `$WS_HOME/triggers/<trigger-id>.json` (default
`~/.ws/triggers/`) before registering a replacement. Never edit registry files to
change lifecycle or retrigger actions.
