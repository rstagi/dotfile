---
name: ws-orchestrate
description: >-
  Orchestrate work in one or more repos from a ws session that has no repo.
  The orchestrator starts one child session for each repo and sends tasks.
  It relays contracts and collects PR URLs.
  It never edits repo files. Use for requests such as "work on repo X and Y" or "orchestrate this".
  Also use for "change these repos from this scratch session" or "delegate to one session per repo".
  Requires the ws CLI. Not for a session in a single-repo workspace or for a registered Loop plan.
---

# WS Orchestrate

Use this skill in a scratch session or a directory session.
In this skill, your session is the orchestrator. Each session that you start is a child.
Read the ws-sessions skill for the full ws command contract.

## When to use

Use this skill when these two conditions are true:

1. Your session has no workspace. `ws whoami --json` shows no `workspace_id`.
2. The user asks for changes in one or more repos.

Do not use this skill in these conditions:

- Your session is in the workspace of a single repo. Do the work in that workspace.
- The Loop daemon has a registered plan for the work. Use the loop-execute skill.

## Rules

1. Do not edit files in a repo checkout. This rule applies to the main checkout and to all worktrees.
2. Read files, make the plan, send tasks to children and report to the user.
3. Start exactly one child for each repo that the work changes.
4. Use ws messages for all coordination with children.
5. Do not merge a pull request (PR). The user merges.

## Name your session

ws names your session automatically from the first user prompt.
Before you start children, make sure that the name tells the topic of the work, for example `auth-rollout`.
If it does not, use the MCP tool `session_rename` or `ws name <name>`.

## Start one child for each repo

First, look for a child that you started for the repo:

```sh
ws whoami --json
ws sessions --json
```

Select the sessions whose parent ID is your session ID and whose `repos` include the repo path.
If such a child exists, send the new task to that child. Do not start a second child for that repo.
If no child exists, start one:

```sh
ws new --repo ~/code/api --branch feat/auth-rollout --agent codex \
  --name auth-rollout-api --prompt "$(cat task-api.md)" --detach
```

Name each child `<topic>-<repo>`. Your session becomes its parent.
The MCP tool `session_create` accepts the same fields: `repo`, `branch`, `agent`, `name` and `prompt`.

## Write the task prompt

Each task prompt must state these items:

1. Scope: the changes to make and the changes not to make.
2. Done when: the checks that show completion, for example passing tests.
3. Branch: the branch name for the work.
4. Report: when the work is complete, run `ws done -` with a summary on stdin. Include the PR URL if you open a PR.
5. Questions: for a question about a different repo or about a shared contract, run `ws send <orchestrator> <question>`. Do not guess.

Write your session name in the prompt as `<orchestrator>`.

## Coordinate the children

Use only these commands for coordination:

```sh
ws on auth-rollout-api done --send-to me 'Child {id} completed: {output}'
ws wait auth-rollout-web --for done --timeout 3600
ws inbox --json
ws ack <event_id>
ws send auth-rollout-web 'The API returns 401 with body {"error":"expired"}.'
```

When a child asks a cross-repo question, find the answer or ask the user.
Send each agreed contract, for example an API shape, to each child that uses it.
Exit 75 from `ws send` means queued. Do not send the message again.

## Handle a failed child

A child can stop before `ws done`. Then its state is `failed`, or its activity is `stopped`.

1. Run `ws show <child> --json`. Read the output path and the state.
2. Read the last output of the child with `ws session tail <child> --json` or the MCP tool `session_tail`.
3. If the cause is temporary, run `ws open <child-cwd> --detach` and send the task again.
4. If the cause is not temporary, report the cause to the user.

Do not start a second child for the repo. Do not do the work of the child yourself.

## Report to the user

Wait until each child completes or fails.
Read the output of each child. `ws show <child> --json` gives the output path.
Give the user one summary with these items:

- The result for each repo.
- The PR URL for each repo.
- The open questions and the failures.
