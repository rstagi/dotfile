# S1 — Transcript relocation

## Question

Can Claude Code resume a conversation after its repository and transcript move to a different absolute path?
Does it recall a synthetic code word when the target uses a separate agent home?

This report covers Claude Code. Phase 5 adds the Codex result.
The laptop → Mac mini check remains a human step at the milestone gate.

## Method

### Claude Code

Test date: 2026-10-16. Claude Code: **2.1.302**. Requested model: `haiku`.
The [observation](s1/claude-observation.json) records the resolved model and result.

Agent-home mode: **sandbox**. Both `HOME` and `CLAUDE_CONFIG_DIR` differed between the two calls.
Authentication used the inherited `ANTHROPIC_API_KEY`. The test did not print its value.
No `.credentials.json` file existed to copy on this machine.
The script copies that file into each sandbox when it exists.
It never copies live settings.

Run the reproduction from this repository:

```sh
docs/ws-v6/spikes/s1/relocate-claude.sh
```

Requirements: `claude`, `git`, Python 3, and authentication available to the sandbox.
Each run makes two model calls and one version query.
Each model call has a 180-second timeout.
The script retains its private temporary directory and prints its location.
Treat that directory as private because it can contain copied credentials.
Do not commit its raw files.

The [script](s1/relocate-claude.sh) performs these steps:

1. Create two temporary homes, `home-a` and `home-b`.
2. Initialize a git repository at `home-a/repo`.
3. Create `marker.txt` with synthetic text. Keep the code word out of this file.
4. Start `claude --session-id <uuid> -p` from that repository.
5. Give the agent a random code word. Ask it to read the marker and reply `STORED`.
6. Copy the repository, including `.git`, to `home-b/repo`.
7. Find the new session JSONL under the source config's `projects/` directory.
8. Create the destination project directory under `home-b/.claude/projects/`.
9. Write a relocated copy of the JSONL there. Preserve the source file.
10. Resume from `home-b/repo` with `claude --resume <uuid> -p`.
11. Ask for the code word without supplying it again.
12. Check an exact code-word match and the same session ID.

Both calls disable customizations, MCP servers, and setting sources.
They allow only the `Read` tool and use `dontAsk` permissions.
They receive `/dev/null` as standard input.
They use separate `HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, and `XDG_CONFIG_HOME` values.
The target config contains only the relocated transcript before resume.
The source config remains intact and lies outside the target's lookup scope.

### Exact relocation recipe

Let `A` and `B` denote the two absolute home paths.
The worktree has the same relative path, `repo`, under each home.

1. Stop at a completed agent turn.
2. Copy the repository from `A/repo` to `B/repo`.
3. Derive the destination project key from the canonical absolute path `B/repo`.
4. Replace each character outside `[A-Za-z0-9]` with `-` in that path.
5. Create `$TARGET_CLAUDE_CONFIG_DIR/projects/<project-key>/`.
6. Parse each source JSONL record.
7. Replace the `A` path prefix with `B` in every JSON string value.
8. Require a path boundary after that prefix. Preserve unrelated prefixes.
9. Preserve record order, session IDs, UUIDs, parent links, tool IDs, and reference hashes.
10. Write `<session-id>.jsonl` as a new file with mode `0600`.
11. Start `claude --resume <session-id> -p` from `B/repo` with the target config directory.

Use the script's rewrite interface for step 6 through step 10:

```sh
docs/ws-v6/spikes/s1/relocate-claude.sh --rewrite \
  "$SOURCE_JSONL" "$NEW_TARGET_JSONL" "$SOURCE_HOME" "$TARGET_HOME"
```

The destination must not exist. The script refuses to overwrite it.
The script parses all records before it creates the destination.
It changes string values, including embedded paths in historical text. It does not change JSON keys.
This recipe preserves the worktree's relative layout. It does not map unrelated external directories.

The tested project paths were ASCII and shorter than 200 characters.
The reproduction refuses longer destination keys.
Claude Code uses an additional hash for longer keys. This spike does not implement that variant.
See the official [session storage documentation](https://code.claude.com/docs/en/sessions#where-transcripts-are-stored).

## Findings

### Claude Code

**The final test passed.** The initial response was `STORED`.
The resumed response exactly matched `SAPPHIRE-E9E26CEFC59B`.
The session ID remained `81a1e16f-fdce-4546-b44e-d7794cd4ebc7`.
The marker file contained no code word. Both calls produced empty standard-error files.
The source transcript stayed byte-identical after the target resumed.

The source contained 27 JSONL records.
The relocated copy contained no remaining source-home prefix before resume.
The [observation](s1/claude-observation.json) includes record counts and hashes of both pre-resume files.
Its paths use `<sandbox>` in place of the temporary root.
It contains no raw API response, credential, account ID, or email address.

The following fields contained absolute paths in this run:

| Record type | Field | Path use |
| --- | --- | --- |
| `user`, `assistant`, `attachment` | `cwd` | Working directory |
| `attachment` | `attachment.snapshot.workingDirectory` | Environment snapshot |
| `attachment` | `rendered[].content` | Working directory inside rendered environment text |
| `assistant` | `message.content[].input.file_path` | Absolute `Read` input |
| `user` | `toolUseResult.file.filePath` | Absolute read-result path |
| `user` | `message.content` | Absolute marker path inside the initial prompt |
| `queue-operation` | `content` | Copy of the initial prompt |
| `last-prompt` | `lastPrompt` | Copy of the initial prompt |
| `api-request-blob` | `message.content[].text` | Prompt or environment text |
| `api-request-blob` | `message.content[].input.file_path` | Copy of the tool input |

This inventory covers one short session with one `Read` tool call.
Other tools, subagents, attachments, and compaction can add path fields or sidecar files.
No sidecar file was necessary for this session.

The test proves that the complete rewrite recipe works for this case.
It does not identify the minimum fields that require changes.
It does not prove that unchanged internal request hashes retain their original content meaning after a rewrite.
The internal transcript format can change between releases, as the official storage documentation states.

The first exploratory run accidentally included the reproduction's source through inherited standard input.
A regression test reproduced that issue before the script received the `/dev/null` fix.
The final experiment used only the intended synthetic prompts.
This phase used **four model calls and four help/version queries**, for eight real CLI invocations.

Local checks passed:

```sh
python3 docs/ws-v6/spikes/s1/relocate-claude.test.py
sh -n docs/ws-v6/spikes/s1/relocate-claude.sh
shellcheck docs/ws-v6/spikes/s1/relocate-claude.sh
test -s docs/ws-v6/spikes/s1-transcript-relocation.md
grep -q '^## Verdict' docs/ws-v6/spikes/s1-transcript-relocation.md
```

The tests cover path replacement, message links, source preservation, private output permissions, and isolated agent input.
The synthetic CLI test makes no model calls.

## Verdict

**partial** for S1. **go** for this local Claude Code relocation case.
A different absolute home and repository path did not prevent recall.
Codex, subscription authentication on another host, and the physical machine transfer remain unverified.

## Impact on the plan

Keep native Claude Code transcript resume as a candidate for the later checkpoint and move implementation.
Do not treat this result as approval for cross-machine failover.
The milestone gate must include the following human check:

- [ ] Move a completed synthetic Claude Code session from the laptop to the Mac mini.
- [ ] Use a different absolute `$HOME` and the same relative worktree layout.
- [ ] Authenticate independently on the Mac mini. Use the same Claude Code version first.
- [ ] Copy the repository and transcript. Apply the recipe to a new destination file.
- [ ] Resume by session ID. Check exact code-word recall and a file read under the destination repository.
- [ ] Record the versions, authentication mode, result, and any additional required files.

Check the original checkpoint's content hash before path rewriting.
Keep that artifact immutable. Rewrite a local copy and record its separate digest.
Repeat this test when the transcript format or agent version changes.
Do not infer sidecar portability from this single-file result.

If either agent fails the S1 gate, use the architecture's fallback:
normalize paths on each device, or resume with a summary prompt.
See architecture §9, §10, and §15 S1. Phase 5 will update the combined verdict.
