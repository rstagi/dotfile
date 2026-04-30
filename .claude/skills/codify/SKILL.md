---
name: codify
description: |
  Codify the most recent successful workflow in this conversation into a
  reusable Claude Code skill. Walks back ≤10 turns, delegates SKILL.md
  synthesis to the official skill-creator skill (or falls back inline),
  stages the result, asks for approval, and commits atomically. Use when
  asked to "codify", "save this as a skill", "make this a skill", or
  "skillify".
allowed-tools:
  - Bash
  - Read
  - Write
  - Edit
  - AskUserQuestion
  - SlashCommand
disable-model-invocation: true
---

# /codify — make the recent workflow a permanent skill

Take the most recent successful task in this conversation and turn it into a Claude Code skill the user can invoke directly next time.

## Iron contract

- **Stage in a temp dir; commit only on user approval.** On rejection, on synthesis failure, or on collision: remove the staged dir entirely. There is no "almost shipped" state.
- **Never overwrite an existing skill silently.** Refuse and ask for a different name.

## Step 1 — Provenance guard

Walk back through the conversation, **at most 10 agent turns**, looking for the most recent task that:

- has a clear user intent
- produced a successful outcome the user did not subsequently invalidate ("that's wrong", retried, abandoned)
- is concrete enough to codify (not "we discussed X")

If none found, refuse with: `"No recent successful task found in this conversation to codify. Run the task first, confirm it worked, then say /codify."`

If the candidate is more than 3 turns back, ask once: `"The last successful task was '<short intent>' a few turns back. Codify that one?"` Anything other than yes → refuse.

## Step 2 — Name + tier

Derive a short kebab-case name (≤32 chars, starts with letter, no consecutive dashes). Confirm with AskUserQuestion:

```
Codify "<intent>" as a skill?
A) Keep "<name>" at user tier — ~/.claude/skills/<name>/ (recommended; available in every project)
B) Keep "<name>" at project tier — ./.claude/skills/<name>/ (only this repo)
C) Rename it (free-form: tell me the new name)
```

## Step 3 — Collision check

```bash
[ -e "$HOME/.claude/skills/<name>/SKILL.md" ] && echo USER_EXISTS
[ -e "./.claude/skills/<name>/SKILL.md" ] && echo PROJECT_EXISTS
```

If a skill exists at the chosen tier: refuse, ask the user to pick a different name (back to step 2). If at the *other* tier: tell the user the new one will shadow it; proceed only if they confirm.

## Step 4 — Synthesize SKILL.md via `skill-creator`

Delegate the SKILL.md authoring to the official `skill-creator` skill (installed at `~/.claude/skills/skill-creator/`). Invoke `/skill-creator` via the SlashCommand tool, passing:

- the proposed name and tier
- the user's intent (the trigger that started the successful task)
- the trace of **successful steps only** — drop failed attempts, dead-ends, unrelated turns, conversation prose
- the tools the codified flow actually used
- the request: write the result to `/tmp/codify-staging-<name>-$$/SKILL.md`

If `/skill-creator` returns an error or fails to write the file: clean up the staged dir and surface the error to the user. Do not attempt inline synthesis — fix the underlying skill-creator invocation instead.

## Step 5 — Approval gate

Print the staged SKILL.md, then AskUserQuestion:

```
Commit "<name>" to <resolved-tier-path>?
A) Commit (recommended)
B) Show me the full content again
C) Discard — don't commit
```

If B: print the full SKILL.md once more and re-ask without B.

## Step 6 — Commit or discard

If approved (A):
```bash
DEST="<tier-base>/<name>"   # ~/.claude/skills/<name> OR ./.claude/skills/<name>
mkdir -p "$DEST"
mv "/tmp/codify-staging-<name>-$$/SKILL.md" "$DEST/SKILL.md"
rmdir "/tmp/codify-staging-<name>-$$" 2>/dev/null
ls -la "$DEST/SKILL.md"  # verify
```

If rejected (C):
```bash
rm -rf "/tmp/codify-staging-<name>-$$"
```
Report: `"Discarded. No skill was written to disk."`

## Step 7 — Lessons log (if in a project repo)

If `docs/lessons.md` exists in the current working directory, append under `## Rules`:

```
### <YYYY-MM-DD>: codified /<name>
- **Situation**: codified "<intent>" as a skill (<tier>)
- **Rule**: when this comes up again, run /<name> instead of re-driving the workflow by hand
```

If `docs/lessons.md` doesn't exist, skip silently.

## Step 8 — Confirm

One line: `"Skill '<name>' committed at <tier-path>. Invoke with /<name>."`

## Limits

- Codifies the trajectory you actually ran, not hypothetical workflows
- One task per invocation — for multiple successes, run /codify once per task
- Doesn't modify or remove existing skills (use $EDITOR or `rm -rf ~/.claude/skills/<name>/`)
