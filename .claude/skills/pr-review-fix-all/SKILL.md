---
name: pr-review-fix-all
description: >-
  Verify one Loop AR round's converged review and optional disputed items against code,
  plan every disposition, delegate accepted findings to Opus fixers, then integrate,
  verify, commit, and fast-forward push. Use only for Loop's `fix<r>` remediation runs
  with one report, any current disputed sidecar, tier standard|max, RUN_DIR, integration
  worktree, and verification command supplied.
argument-hint: "report:<path> [disputed:<path>] tier:<standard|max> run-dir:<path> worktree:<path> branch:<name> remote:<name> verify:<command>"
allowed-tools:
  - Agent
  - Bash
  - Edit
  - Glob
  - Grep
  - Read
  - Write
---

# PR Review Fix All

You are the Opus remediation coordinator for both standard and max. Verify the current
round's converged review and disputed items against code, delegate accepted fix groups to
Opus fixers, and leave the integration branch verified and pushed. The caller supplies one
`report:` (the round's last R `report.md`), optional `disputed:` (the current round's
`.disputed.md`), tier, absolute `RUN_DIR`, integration worktree, integration branch, remote,
and repository verification command. Stay in that worktree.

## 1. Validate inputs and state

Require exactly one `report:` file and, when supplied, a readable `disputed:` file. Require
`tier:standard|max`, an absolute `RUN_DIR`, the expected integration branch, remote, and a
non-empty verification command. Legacy tiers shallow and medium are rejected. Confirm the
current worktree and branch match. Stop with `question` for ambiguous caller input, or
`blocked` for unsafe/dirty/unrecoverable state. Do not read reports from earlier rounds or
GitHub review comments. A sidecar belongs to this round only when that round has a disputed
ledger row; an old sidecar left beside the ledger is not input for a later clean round.

Record the starting HEAD and remote branch SHA. Never force-push, rewrite history, open a PR,
or post a GitHub review/comment. Require a clean worktree and local HEAD equal to the remote
integration branch before remediation begins.

## 2. Reproduce and reconcile

Read the converged review and, when supplied, the disputed sidecar in full. The sidecar
preserves the last R and critic reports and unresolved IDs, including critic-added issues.
Verify every finding and every disputed item against the current code before accepting or rejecting it.
Inspect surrounding code and tests; reproduce the claimed behavior or cite concrete code
that confirms or refutes it. Consensus or disagreement alone is not evidence. A bare rebuttal
is insufficient; trace disputed claims to the cited code and verify that code's behavior.

Account for every item, including an `APPROVE` report with critic-only disputed items. Merge
duplicates by evidence while retaining every source ID and its disposition. Do not drop a
critic concern merely because R rejected or omitted it, or accept R's finding merely because
C agreed. Resolve conflicts from evidence in the current tree.

Write `RUN_DIR/remediation-plan.md` before editing code. Include every finding with:

- source and stable finding ID (identify reviewer versus disputed/critic-only origin);
- severity and exact claim;
- disposition: accepted, rejected, or duplicate;
- reproduction/evidence and duplicate target where relevant;
- fix group, files, dependencies, and overlap risks;
- test-first step and verification command.

Accepted and rejected findings, including disputed items, require concrete code/test evidence. Every accepted finding must belong to exactly one
fix group; none may be silently skipped.

## 3. Build safe work units

Group accepted findings only when their implementation is cohesive. Mark groups independent
only when they touch disjoint files, have no behavioral dependency, and can run tests without
mutating shared generated state. Schedule groups with overlapping files or dependencies serially.

If there are no accepted findings after verifying both inputs, run the repository verification
command, confirm remote HEAD still equals local HEAD, and record the no-op in the plan. Do not
create an empty commit or push; finish with `done` and `result:NOOP`.

## 4. Implement fixes

**standard / max — always spawn Opus fixers.** Use the Agent tool with the `pr-review-fixer` subagent for each ready independent fix group. Its
checked-in definition sets `model: opus` (the latest Opus); do not substitute another subagent or model.
Launch a maximum of 3 concurrent subagents in one batch. Wait for the batch, inspect the shared
worktree, then launch the next dependency-ready batch.

Each delegation prompt must include only its assigned findings, evidence, allowed files,
test-first step, relevant project instructions, and focused verification. Tell every fixer:

- work only in the current integration worktree and assigned files;
- use TDD where applicable: demonstrate the failing test, then make it pass;
- do not invoke `pr-review-fix-all` or spawn more subagents;
- do not inspect sibling assignments or any full review report;
- do not switch branches, stash, reset, revert, clean, or restore files;
- you may edit and test, but must not commit, push, or post to GitHub;
- report changed files, tests run, results, and any unresolved issue.

For either tier, if the Agent tool or `pr-review-fixer` is unavailable, stop with `blocked`; do not
silently implement delegated groups in the coordinator or substitute another model.

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

After verification and remote confirmation, write `RUN_DIR/status.json` only as the final
action. Successful remediation must provide the evidence consumed by `loop-review.sh record`:

```json
{
  "outcome": "done",
  "summary": "1-3 concise lines",
  "result": "PUSHED | NOOP",
  "headBefore": "full starting SHA",
  "headAfter": "full verified final SHA",
  "remote": "supplied remote",
  "branch": "supplied integration branch",
  "worktree": "absolute integration worktree"
}
```

Use literal `PUSHED` only for changed HEAD with a verified fast-forward push; use `NOOP` only
for unchanged HEAD. Both require `git ls-remote` confirmation of the supplied remote branch
against local HEAD. Retain starting/final head evidence in the remediation plan for crash recovery.
For `question` or `blocked`, write outcome, summary, and respectively `question` or `details`;
never claim a successful result without verified evidence.

Use `done` only when the plan accounts for every finding and disputed item, all accepted findings are fixed, the
full verification passes, commits are pushed when changes exist, and remote HEAD is confirmed.
