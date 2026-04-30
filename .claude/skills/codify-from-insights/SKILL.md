---
name: codify-from-insights
description: |
  Read the latest /insights output, triage skill / CLAUDE.md / hook
  candidates from its suggestions, and codify approved ones atomically.
  Delegates SKILL.md synthesis to skill-creator and hook config to
  update-config when available. Runs /insights first if its output isn't
  in this session. Use when asked to "act on insights", "review insights
  candidates", or "codify from insights".
allowed-tools:
  - Bash
  - Read
  - Write
  - Edit
  - AskUserQuestion
  - SlashCommand
disable-model-invocation: true
---

# /codify-from-insights — act on /insights suggestions

Read the most recent `/insights` output, triage each candidate (skill / CLAUDE.md edit / hook config), and codify approved ones into the right destination atomically.

## Iron contract

Same as `/codify`: stage → ask → commit/discard. **Per-candidate approval — never batch-commit.** No silent disk writes. On any failure: remove the staged dir entirely.

## Step 1 — Get insights data

Look back through the conversation history for a recent `/insights` tool result.

- **If found**: use that JSON directly (it has the `suggestions.{claude_md_additions, features_to_try, usage_patterns}` shape).
- **If not found**: invoke `/insights` via the SlashCommand tool, wait for completion, use the result.

If neither works (no fresh data, no cache), refuse: `"No /insights data available. Run /insights first, then /codify-from-insights."`

## Step 2 — Triage candidates

Build a candidate list from the three buckets in `suggestions`:

| Source | Type | Default destination |
|---|---|---|
| `claude_md_additions[]` | CLAUDE.md edit | `./CLAUDE.md` (project, if present) else `~/.claude/CLAUDE.md` |
| `features_to_try[]` where `feature == "Custom Skills"` | New skill | `~/.claude/skills/<name>/SKILL.md` |
| `features_to_try[]` where `feature == "Hooks"` | Hook config | `./.claude/settings.json` (project) else `~/.claude/settings.json` |
| `usage_patterns[]` (`copyable_prompt`) | New skill (slash command) | `~/.claude/skills/<name>/SKILL.md` |

**Skip** `features_to_try[]` entries that aren't Skills/Hooks (MCP Servers, Sub-Agents, etc.) — those are user-info, not direct codify targets.

For each candidate, derive a short kebab-case name (for skills) and a one-line rationale from the source's `why` / `why_for_you` / `detail` field.

## Step 3 — Per-candidate approval loop

For each candidate, AskUserQuestion:

```
[<i>/<total>] <type> — <name-or-title>
Why: <one-line rationale>

A) Codify it
B) Show me the full draft first
C) Skip this one
D) Stop here (skip remaining)
```

If B: print the full proposed artifact (SKILL.md content / CLAUDE.md section / hook config snippet) and re-ask without B.

If A: stage and commit per the type rules below.
If C: move to next candidate.
If D: stop and jump to step 5.

## Step 4 — Per-type commit (delegate where possible)

All commits stage in `/tmp/codify-staging-<name>-$$/` first; on any failure, remove the staged dir.

### Skill — delegate to `skill-creator`

Invoke `/skill-creator` via SlashCommand, passing the candidate's title, intent, and `example_code` / `copyable_prompt`. Have it write the SKILL.md to `/tmp/codify-staging-<name>-$$/SKILL.md`.

Collision check: if `~/.claude/skills/<name>/SKILL.md` (or chosen tier) exists → refuse, skip. Otherwise:
```bash
mkdir -p "$DEST/<name>" && mv "/tmp/codify-staging-<name>-$$/SKILL.md" "$DEST/<name>/SKILL.md"
ls -la "$DEST/<name>/SKILL.md"
```

### Hook — delegate to `update-config`

Invoke `/update-config` via SlashCommand to add the proposed hook entry to settings.json. Pass it the event (`PostToolUse`, `Stop`, etc.), the matcher, and the command. `update-config` handles the merge into the appropriate scope (project vs user) without overwriting existing entries.

If `update-config` rejects (e.g., identical hook already exists), surface that and skip.

### CLAUDE.md edit — inline

1. Determine target: `./CLAUDE.md` if present in cwd, else `~/.claude/CLAUDE.md`.
2. Find a sensible insertion point — the candidate's `prompt_scaffold` may suggest a section header; otherwise append at the bottom.
3. Use Edit to insert. **Show the diff to the user before saving.**
4. If the section already exists with similar content → refuse, skip.

## Step 5 — Lessons log (if in a project repo)

If `docs/lessons.md` exists, append one line under `## Rules` per committed candidate:

```
### <YYYY-MM-DD>: codified <type> "<name>" from /insights
- **Situation**: /insights flagged "<rationale>"
- **Rule**: <how-to-apply, e.g., "use /<name> when ..." or "follow the new CLAUDE.md section ...">
```

If it doesn't exist, skip silently.

## Step 6 — Summary

```
Codified N of M candidates from /insights:
  - <type>: <name> at <path>
  - ...
Skipped: <count> (or stopped at i if D was chosen).
```

## Limits

- Reads `/insights`'s already-curated suggestions; does not re-analyze raw facets
- Per-candidate approval only — never auto-commits even high-confidence picks
- Doesn't modify existing artifacts (only adds new ones)
- Discord/cron handling is out of scope — separate piece
- Authoring is delegated to `skill-creator` (for skills) and `update-config` (for hooks); both are installed in this environment
