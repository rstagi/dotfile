# S1 — Transcript relocation

## Question

Can Claude Code and Codex resume a conversation after its repository and transcript move to a different absolute path?
Does it recall a synthetic code word when the target uses a separate agent home?

This report covers Claude Code and Codex on one machine.
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

### Claude Code relocation recipe

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

### Codex

Test date: 2026-10-16. Codex: **0.159.2**. Requested and recorded model: `gpt-6.1-sol`.
The reasoning effort was `low`.
The [observation](s1/codex-observation.json) records the result and the path inventory.

Agent-home mode: **sandbox**. Both `HOME` and `CODEX_HOME` differed between calls.
The script copied the live `auth.json` into each private sandbox with mode `0600`.
The authentication mode was `chatgpt`. No `OPENAI_API_KEY` existed in the environment.
The experiment used no live agent home and changed no existing live rollout.
It did not copy live configuration, SQLite databases, plugins, or caches.

Run the reproduction from this repository:

```sh
docs/ws-v6/spikes/s1/relocate-codex.sh
```

Requirements: `codex`, `git`, Python 3, and authentication available to the sandbox.
Keep `relocate-claude.sh` beside this script. Both scripts use its generic JSONL rewrite interface.
Each run makes two model calls and one version query.
Each model call has a 180-second timeout.
The script retains its private temporary directory and prints its location.
That directory contains copied credentials and raw rollouts. Do not commit it.
The generated `summary.json` contains only selected fields and replaces the temporary root with `<sandbox>`.

The [script](s1/relocate-codex.sh) performs these steps:

1. Create `home-a` and `home-b` with separate `.codex` directories.
2. Copy authentication into each directory. Initialize a git repository at `home-a/repo`.
3. Create `marker.txt` with synthetic text and no code word.
4. Start `codex exec` from path A with a random code word.
5. Ask it to read the absolute marker path and reply `STORED`.
6. Get the thread ID from the `thread.started` JSON event.
7. Find its rollout under `$SOURCE_CODEX_HOME/sessions/`.
8. Copy the repository, including `.git`, to `home-b/repo`.
9. Write a relocated rollout under `$TARGET_CODEX_HOME/sessions/` with the same relative filename.
10. Resume from path B with `codex exec resume <id>`.
11. Ask it to read `marker.txt` and recall the code word. Do not supply the word again.
12. Check exact recall, thread identity, source preservation, and absence of old-home paths in the relocated copy.

Both calls use `--ignore-user-config`, `--ignore-rules`, and `--json`.
Configuration overrides set `approval_policy="never"` and `sandbox_mode="read-only"`.
The script removes inherited agent, WS, Loop, and OTEL environment variables.
It sets separate `HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, and `XDG_CONFIG_HOME` values.
Both calls receive `/dev/null` as standard input.
Codex initialized its own databases, system skills, and plugin caches in each sandbox.
Those generated files were not transferred from A to B.

The official [command documentation](https://learn.chatgpt.com/docs/developer-commands#codex-exec) describes non-interactive resume.
The installed CLI help listed the options above.
The storage and rewrite details below are observations from this version.

### Codex relocation recipe

Let `A` and `B` denote two absolute home paths with the same relative worktree path, `repo`.

1. Stop at a completed agent turn.
2. Copy `A/repo` to `B/repo`.
3. Find the source rollout whose filename contains the session ID.
4. Preserve its relative path, `sessions/YYYY/MM/DD/rollout-<timestamp>-<id>.jsonl`.
5. Create the corresponding parent directories under the destination `CODEX_HOME`.
6. Parse every JSONL record. Replace the `A` prefix with `B` in JSON string values.
7. Require a path boundary after the prefix. Preserve unrelated prefixes, JSON keys, and record order.
8. Preserve session IDs, thread IDs, turn IDs, tool links, and opaque reasoning content.
9. Write the destination as a new file with mode `0600`. Refuse an existing destination.
10. Authenticate in the destination home. Resume the explicit ID from `B/repo`.

The rewrite also updates paths inside historical text and serialized tool input.
It does not change unrelated absolute paths or map external directories.
The script uses the same rewrite function that the Claude experiment tested:

```sh
docs/ws-v6/spikes/s1/relocate-codex.sh --rewrite \
  "$SOURCE_ROLLOUT" "$NEW_TARGET_ROLLOUT" "$SOURCE_HOME" "$TARGET_HOME"
```

Resume with the destination environment and working directory:

```sh
cd "$TARGET_HOME/repo"
HOME="$TARGET_HOME" CODEX_HOME="$TARGET_HOME/.codex" \
  codex exec --ignore-user-config --ignore-rules \
  -c 'approval_policy="never"' -c 'sandbox_mode="read-only"' \
  resume "$SESSION_ID" 'What code word did I ask you to remember?'
```

There is no Claude-style project key in this recipe.
The destination contained only `auth.json` and the relocated rollout before resume.
No source SQLite index or session-history database was necessary for explicit-ID resume.
This result does not establish how `--last`, session names, or interactive pickers behave after transfer.

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

### Codex

**The test passed.** The initial response was `STORED`.
The resumed response exactly matched `TOPAZ-42E40737BC3F`.
The thread ID remained `01a14694-c5af-7783-b25f-57a65582a345`.
The marker contained no code word. The resume prompt contained no code word.
The source rollout stayed byte-identical after resume.

The initial command read `A/repo/marker.txt`.
The resumed command ran `cat marker.txt` with a recorded working directory of `B/repo`.
Both commands returned the synthetic marker text with exit code 0.
The destination appended the resumed turn to the relocated rollout.
The source had 20 records. The destination had 36 records after resume.
The relocated copy contained zero old-home occurrences before resume.

The following fields contained source-home paths in this run:

| Record type | Field | Path use |
| --- | --- | --- |
| `session_meta` | `payload.cwd` | Initial repository directory |
| `session_meta` | `payload.runtime_workspace_roots[]` | Workspace roots |
| `turn_context` | `payload.cwd`, `payload.workspace_roots[]` | Turn directory and workspace roots |
| `response_item` | `payload.content[].text` | Environment text and the initial prompt |
| `response_item` | `payload.input` | Absolute marker path inside the `exec` custom tool input |
| `world_state` | `payload.state.environments.environments.local.cwd` | Local working directory |
| `world_state` | `payload.state.environments.filesystem` | Filesystem environment text |
| `world_state` | `payload.state.host_skills.body` | Paths to sandbox system skills |
| `event_msg` | `payload.item.content[].text` | Copy of the initial prompt |
| `event_msg` | `payload.item.cwd` | Command working directory as a `file://` URI |
| `event_msg` | `payload.item.command[]` | Absolute marker path inside the shell command |
| `event_msg` | `payload.item.parsed_cmd[].cmd`, `payload.item.parsed_cmd[].path` | Parsed read command and path |

New records after resume also included destination paths in `payload.thread_settings.cwd` and `payload.thread_settings.runtime_workspace_roots[]`.
The [observation](s1/codex-observation.json) records field occurrence counts and pre-resume hashes.
It contains no raw rollout, credential, account ID, or email address.
Raw `session_meta` records contain creator identifiers. Do not commit them without redaction.

This test used one short completed turn and one shell read per call.
It did not test compaction, subagents, images, attachments, interrupted turns, or external directory mappings.
It proves that the complete recipe worked. It does not identify the minimum required changes.
The rollout format can change. Repeat the experiment after agent upgrades.

This phase used **two model calls and four help/version queries**, for six real CLI invocations.
The synthetic CLI tests made no model calls.

Local checks passed:

```sh
python3 docs/ws-v6/spikes/s1/relocate-codex.test.py
python3 docs/ws-v6/spikes/s1/relocate-claude.test.py
sh -n docs/ws-v6/spikes/s1/relocate-codex.sh
shellcheck docs/ws-v6/spikes/s1/relocate-codex.sh
test -s docs/ws-v6/spikes/s1/relocate-codex.sh
grep -q -i 'codex' docs/ws-v6/spikes/s1-transcript-relocation.md
```

The Codex tests check path rewriting, tool links, source preservation, and private output permissions.
They also check separate homes, excluded parent-session variables, and empty standard input.
The synthetic resume uses no transferred index.

## Verdict

**partial** for S1. **go** for the tested local Claude Code and Codex relocation cases.
Both agents recalled the exact code word after the home and repository paths changed.
Codex also read the destination marker without a transferred source database.
Subscription authentication on another host and the physical machine transfer remain unverified for both agents.

If either agent fails the S1 gate, use the v6 S1 fallback.
Normalize paths on every device (`~/.ws/…`), or resume with a summary prompt instead of the transcript.

## Impact on the plan

Keep native transcript resume for both agents as a candidate for the later checkpoint and move implementation.
Do not treat this result as approval for cross-machine failover.
The milestone gate must include the following human check:

- [ ] Move one completed synthetic session per agent from the laptop to the Mac mini.
- [ ] Use a different absolute `$HOME` and the same relative worktree layout.
- [ ] Authenticate independently on the Mac mini. First use the same version of each agent.
- [ ] Copy each repository and transcript or rollout. Apply its recipe to a new destination file.
- [ ] Resume by session ID. Check exact code-word recall and a file read under the destination repository.
- [ ] Record the versions, authentication mode, result, and any additional required files.

Check the original checkpoint's content hash before path rewriting.
Keep that artifact immutable. Rewrite a local copy and record its separate digest.
Repeat this test when the transcript format or agent version changes.
Do not infer sidecar portability from this single-file result.

See architecture §9, §10, and §15 S1.
