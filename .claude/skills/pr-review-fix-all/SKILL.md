---
name: pr-review-fix-all
description: >-
  Reconcile one Loop PR-review round's independent Astra and Fable reports, plan every
  disposition, delegate independent accepted fixes to Opus 5 subagents, then integrate,
  verify, commit, and fast-forward push them. Use only for Loop's a3/a6 remediation runs
  when both local report paths, RUN_DIR, integration worktree, and verify command are given.
argument-hint: "astra-report:<path> fable-report:<path> run-dir:<path> worktree:<path> branch:<name> remote:<name> verify:<command>"
allowed-tools:
  - Agent
  - Bash
  - Edit
  - Glob
  - Grep
  - Read
  - Write
disable-model-invocation: true
---

# PR Review Fix All

You are the Opus 5 remediation coordinator. Reconcile two independent adversarial reviews,
delegate safe implementation units, and leave the integration branch verified and pushed.
The caller supplies the Astra report, Fable report, absolute `RUN_DIR`, integration worktree,
integration branch, remote, and repository verification command. Stay in that worktree.

## 1. Validate inputs and state

Require both report files, an absolute `RUN_DIR`, the expected integration branch, remote,
and a non-empty verification command. Confirm the current worktree and branch match. Stop with
`question` for ambiguous caller input, or `blocked` for unsafe/dirty/unrecoverable state. Do not
read reports from earlier rounds or GitHub review comments.

Record the starting HEAD and remote branch SHA. Never force-push, rewrite history, open a PR,
or post a GitHub review/comment. Require a clean worktree and local HEAD equal to the remote
integration branch before remediation begins.

## 2. Reproduce and reconcile

Read both reports independently, then inspect current code and tests. Reproduce every finding;
do not accept a report merely because it sounds plausible. Merge duplicates and resolve
conflicts from evidence in the current tree.

Write `RUN_DIR/remediation-plan.md` before editing code. Include every finding with:

- source and stable finding ID;
- severity and exact claim;
- disposition: accepted, rejected, or duplicate;
- reproduction/evidence and duplicate target where relevant;
- fix group, files, dependencies, and overlap risks;
- test-first step and verification command.

Rejected findings require concrete evidence. Every accepted finding must belong to exactly one
fix group; none may be silently skipped.

## 3. Build safe work units

Group accepted findings only when their implementation is cohesive. Mark groups independent
only when they touch disjoint files, have no behavioral dependency, and can run tests without
mutating shared generated state. Schedule groups with overlapping files or dependencies serially.

If there are no accepted findings, run the repository verification command, record the no-op in
the plan, do not create an empty commit or push, then finish with `done`.

## 4. Spawn Opus fixers

Use the Agent tool with the `pr-review-fixer` subagent for each ready independent fix group. Its
checked-in definition pins `model: claude-opus-5`; do not substitute another subagent or model.
Launch a maximum of 3 concurrent subagents in one batch. Wait for the batch, inspect the shared
worktree, then launch the next dependency-ready batch.

Each delegation prompt must include only its assigned findings, evidence, allowed files,
test-first step, relevant project instructions, and focused verification. Tell every fixer:

- work only in the current integration worktree and assigned files;
- use TDD where applicable: demonstrate the failing test, then make it pass;
- do not invoke `pr-review-fix-all` or spawn more subagents;
- do not inspect sibling assignments or either full review report;
- do not switch branches, stash, reset, revert, clean, or restore files;
- you may edit and test, but must not commit, push, or post to GitHub;
- report changed files, tests run, results, and any unresolved issue.

If the Agent tool or `pr-review-fixer` is unavailable, stop with `blocked`; do not silently
implement delegated groups in the coordinator or substitute another model.

## 5. Integrate and verify

After each batch, inspect all changes before continuing. Reject scope creep and reconcile any
unexpected collision. Reproduce the accepted finding against the changed code and run each
group's focused tests. A failed or incomplete group may be resumed once with precise feedback;
otherwise stop with `blocked`.

After all groups return:

1. Account for every accepted finding against the resulting diff.
2. Review the complete diff for correctness, regressions, and accidental files.
3. Run the repository verification command exactly as supplied.
4. Confirm only intended files changed and no accepted finding remains unresolved.
5. Commit logical fix groups and fast-forward push the integration branch to the supplied remote.
6. Confirm the remote branch SHA equals local HEAD.

The coordinator alone owns commits and the push. Subagents must not commit, push, or post to GitHub.
Never force-push. If verification fails or the remote cannot be safely fast-forwarded, write
`blocked` and stop without claiming success.

## 6. Finish last

Write `RUN_DIR/status.json` only as the final action, using:

```json
{
  "outcome": "done | question | blocked",
  "summary": "1-3 concise lines",
  "question": "only for question",
  "details": "only for blocked"
}
```

Use `done` only when the plan accounts for every finding, all accepted findings are fixed, the
full verification passes, commits are pushed when changes exist, and remote HEAD is confirmed.
