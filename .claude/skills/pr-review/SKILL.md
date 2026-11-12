---
name: pr-review
description: Review a pull request or a branch in one round. First, check it out into a local git worktree, read its CI checks and run focused checks of the changed code. CI runs the full test suite. Then give each finding a severity, major or minor, for the PR comments, and a disposition, fix now or defer, for the chat. Security, unprotected breakage and a failing check are always major and fix now. The defer policy of the repository (flag or strict) and the owner decide the disposition of the other findings. Report the footprint of the PR and a verdict that follows the fix-now list. Use this skill when the user asks to review a PR, a pull request or a branch, or to examine changes before a merge. Examples are "review #123", "can you look over this branch" and "review this MR". Use it also when the user does not say the word "skill". Use it instead of a quick diff-only read when the user wants a real review.
---

# PR review

> Source of truth for these rules: [`CODING_STANDARDS.md`](../../../CODING_STANDARDS.md). When this skill and that file differ, the file wins. Update the file first.


A review judges a PR on two tests. The PR does not break existing behavior. The PR is easy to remove. A diff alone does not show if the code works or if the tests exercise it. Thus this skill always reads the CI result and runs the changed code first. CI runs the full suite. The reviewer runs only focused checks. Then it gives each finding a severity (`[major]` or `[minor]`) and a disposition (fix now or defer). The severity goes on the PR. The disposition goes only in the chat.

Do not skip the worktree step when the change looks trivial. A confident but wrong review often starts with a change that looks trivial.

**Writing style for the report and the posted comments:** Be concise. Start with the verdict. Keep each finding to a few sentences. Do not tell a story. Never use em dashes or en dashes. Use commas, colons, parentheses or separate sentences. [Finding labels](#finding-labels) and [Writing the findings](#writing-the-findings) tell how to label and write each finding.

## One round

Review a PR one time. Do not review the same PR again to find new small issues. When the user asks you to check a fix, check only the earlier fix-now items. Also check that the fix adds no new major finding. Do not add new findings to defer in that check.

## Step 1 — Check it out, read CI and run the focused checks (do this first)

Create a detached worktree before you build or test the branch.
Run this block and all review commands in one persistent shell.
Exit that shell only after the review ends.

```bash
# Fetch a PR. For a branch review, use: git fetch origin <branch>
git fetch origin pull/<number>/head
REVIEW_REPO=$(git rev-parse --show-toplevel)
REVIEW_REF=FETCH_HEAD

REVIEW_ROOT="${TMPDIR:-/tmp}/pr-review"
mkdir -p "$REVIEW_ROOT" || exit 1
REVIEW_ROOT=$(cd "$REVIEW_ROOT" && pwd -P) || exit 1
REVIEW_TMP=$(mktemp -d "$REVIEW_ROOT/review.XXXXXX") || exit 1
REVIEW_WORKTREE="$REVIEW_TMP/worktree"
trap 'cd "$REVIEW_REPO"; git worktree remove --force "$REVIEW_WORKTREE"; git worktree prune; rmdir "$REVIEW_TMP"' EXIT
git worktree add --detach "$REVIEW_WORKTREE" "$REVIEW_REF" || exit 1
cd "$REVIEW_WORKTREE" || exit 1
```

CI runs the full test suite in parallel with the review. NEVER run the full test suite in the worktree. Build the project when you must run it to exercise the change. Run only the targeted tests of the changed code.

1. Read CI first. For a PR, run `gh pr checks <number>`. For a branch, run `gh run list --branch <branch>`.
   - Record each failing check: its name and its log URL. This is the evidence for `failing-check`.
   - When a check is pending, continue the review. Read the checks again before you write the verdict.
   - When a required check is still pending at the verdict, write `CI pending: <check names>` at the top of the report.
2. Run a project check that is not the test suite (for example the architecture tests, the flag off/on matrix, the monitoring check or the delete test) only when CI does not run it. When the PR has no CI, write `No CI: full test suite not run` at the top of the report. Do not run the full test suite instead.
3. Use the project's own commands (README, CLAUDE.md, package manifests) for focused work only:
   - Install dependencies only when a focused command needs them.
   - Build the project when you must run the changed code.
   - Run only the tests of the files and folders that the PR changes.
   - Run the existing tests of the changed area with the flag off when the PR adds a feature flag.
   - Exercise the change: run the app, call the endpoint or call the function that this PR touches.
   - Run one focused test or command to confirm or refute one finding.

Record each check that fails and each existing behavior that changes. These results are the evidence for the hard blocks `failing-check` and `unprotected-breakage`. Report them before you continue.
The exit trap removes the review worktree.

## Step 2 — Classify each finding

Give each finding one severity (`major` or `minor`) and one disposition (fix now or defer). Point to the file and the line. Say what is wrong and why.

### Hard blocks

A hard block is always `major` and always fix now. Nobody defers it. A finding is a hard block only when it is in one of these 3 categories. Each hard block names its category and its evidence.

| Category | The finding is blocking when | Evidence |
|---|---|---|
| `security` | The PR leaks a secret, bypasses auth or a permission check, exposes data or handles user data unsafely. Or the change can cause real harm in production. | The code path, and a repro when you can make one. |
| `unprotected-breakage` | The PR changes current behavior, and no feature flag or equivalent off switch protects the change. An existing flag protects the change when the changed code runs only with that flag on and the flag is on for less than 100% of users. A flag on for 100% of users does not protect it: ask the owner at the checkpoint to remove the flag first or to add a new flag. Or existing tests fail with the flag off. With `Feature flag policy: recommended` (`CODING_STANDARDS.md` → Feature flags), a `No flag: <reason>` line in the PR description that the owner agreed to protects the change. With no such line, the finding is `major`, not a hard block. A flagged PR with no stacked flag removal PR is `minor`. | The Step 1 result: the old and the new behavior, or the failing existing test. |
| `failing-check` | A deterministic check fails: the build, the tests, the architecture tests, the flag off/on matrix, the monitoring check or the delete test. | The failing CI check name and its log URL, or the focused command and its failing output from Step 1. |

A `failing-check` item blocks because the check fails, not because you judge the code. When the project does not have a check, a probable failure of that check is `minor`.

### Severity of the other findings

Use these lenses to find the other findings:

- **Bugs in protected code**: wrong logic or missing edge cases in new code that a flag or an off switch protects. A new entry point that no caller uses yet is an off switch.
- **Tests**: tests that do not exercise the new behavior, and important cases that are missing.
- **AI slop**: comments that only repeat the code, dead scaffolding, abstractions with one use, invented APIs.
- **Structure and names**: names, style, extra abstractions, code in the wrong place, cleaner refactors.
- **Docs**: docs that do not agree with the code.
- **Performance**: ideas that make the code faster.

A finding is `major` when it is a real defect and you have evidence: wrong behavior or a data bug in new code (also behind a flag), a missing test for the main new behavior, or code that breaks a contract that the author wrote. All other findings are `minor`. A finding with only reasoning and no evidence is `minor`.

### Disposition and the defer policy

The disposition says if this PR fixes the finding (fix now) or if follow-up work fixes it (defer). It never goes on the PR.

1. Find the defer policy of the repository. Read the line `Review defer policy: flag|strict` in the repository CLAUDE.md or AGENTS.md. When it is not there, use the table in `CODING_STANDARDS.md` (Ratel Cloud is `flag`, Beluga and other open-source repositories are `strict`). When the repository is in neither, propose `strict` for a public repository and `flag` for a private repository, and ask the owner at the checkpoint.
2. Propose a disposition for each finding that is not a hard block:
   - `flag`: defer a major behind an off-by-default flag. Fix now a major with no flag. Defer a minor.
   - `strict`: fix now each major, also behind a flag. Fix now a minor when the fix is small and in the scope of the PR. Propose defer only with a strong reason, for example a fix that needs a separate design or another repository.
3. Give the reason for each proposal in one sentence. The owner and you decide in the chat. The owner can change any proposal, except a hard block.

Keep the defer list short and concrete. Remove each item that does not deserve a follow-up ticket.

### Footprint

Count the existing files that the PR edits outside its feature folder. The feature folder is the folder that the PR description names, or the folder that holds the new files of the PR. When the PR has no feature folder, count all existing files that the PR edits.

```bash
# In the review worktree. <base> is the base branch of the PR.
git fetch origin <base>
git diff --name-only --diff-filter=MDR "$(git merge-base "origin/<base>" HEAD)" HEAD
```

The footprint is a fact, not a finding. Report the count and the list. When the count is more than 3, add a discussion flag that asks for the reason of each edit. The flag never blocks.

When the PR mixes the feature with unrelated changes, propose a split. Examples of unrelated changes are renames, refactors, formatting changes and dependency updates. Give an ordered list of PRs. For each PR, give the title, the base and the commits or paths that it takes. When `.loop/plan.md` exists, use its phases as candidate seams. Propose the split only. Never do it.

## Prevention pass

For each fix-now item, give the smallest fix. Then find the condition that let the problem occur, for example a missing check, test, type, validation or boundary. When the evidence shows a structural cause, add the preventive change as a `minor` finding with its own proposed disposition. When the evidence shows an isolated problem, say so. Do not invent a broad refactor.

## Step 3 — Report

Write the report in this order:

This report goes in the chat. It is the only place for the dispositions.

1. **Verdict**: `Approve` when no finding is fix now. `Request changes` when one or more findings are fix now. Add the counts and the policy, for example `Request changes. 1 major, 3 minor. Fix now: 2, defer: 2. Defer policy: strict (CLAUDE.md).`
2. **Validation**: the Step 1 commands and the result of each command.
3. **Footprint**: the count, the list, the discussion flag when the count is more than 3, and the split proposal or `No split recommended.`
4. **Fix now**: each item with its label, its file:line, its evidence, its fix and, when it is not a hard block, the reason for the disposition. When the list is empty, write `None.`
5. **Defer**: each item with its label, its file:line, one or two sentences and the reason that it can wait. When the list is empty, write `None.`
6. **Decisions for the owner**: each proposal that the owner must confirm, for example a major that you propose to defer, or a policy that you guessed. When there is none, omit this section.

The report has exactly two finding sections: Fix now and Defer. Do not add other finding sections. When the PR is clean, say so. Do not invent findings to fill a section.

Before you give the report, compare the verdict with the sections. The verdict is `Approve` only when Fix now is empty. When the counts in the verdict do not agree with the lists, fix the counts. When the owner changes a disposition, move the item and update the verdict.

## Finding labels

Each finding has exactly one severity label at the start of its title. A short summary in lowercase follows the label. A hard block names its category after the label:

```
[major] failing-check: the migration test fails on an empty database
[major] the retry drops the last event when the queue is full
[minor] the retry helper has one caller
```

The labels are `[major]` and `[minor]`. The hard-block categories are `security`, `unprotected-breakage` and `failing-check`.

How to choose:

- **Never raise a severity.** When you are not sure, the item is `minor`.
- **A major item needs evidence.** Use the repro or the failing output from Step 1, or the code path. When you only reasoned about a problem, the item is `minor`.
- **The consequence sets the severity, not the size of the fix.** A one-line fix for wrong behavior is still major. A large refactor that only makes the code clearer is still minor.
- **The severity does not depend on the flag.** A bug behind a flag is still major. The flag changes only the disposition, through the defer policy.
- **Say what the problem does not affect.** For example: "Plain semantic search is not affected." This makes the severity credible. It also stops a reaction from the author that is too strong.
- **Keep the label and the text in agreement.** Do not write a `[minor]` item whose text says that the process becomes unusable.
- When the user changes the severity of an item, change the label and keep the technical content. When the new label does not agree with the text, tell the user. Do not make a verified finding weaker without a statement.

## Writing the findings

What makes a comment land:

- **Lead with the mechanism, not the symbol names.** "The permit is claimed in the synchronous NAPI body, so its timing depends on when JS calls it" beats "`resolveEmbeddingArtifact` yields before `experimentalWarmEmbeddingsFromArtifact`". If the author has to reconstruct the causal chain themselves, rewrite it.
- **Show the evidence you actually gathered.** Paste the repro steps, the command output, the failing assertion, the empty grep. A finding with observed output attached is not arguable; one without it is an opinion.
- **A timeline beats a paragraph** for anything involving ordering, concurrency, or lifecycle. Two short interleaved traces (works / broken) explain a race faster than prose.
- **Cut the connective tissue.** No "worth fixing because", "it's important to note", "flagging this since". State the defect, the proof, the fix.
- **Sound like a colleague.** Ask real questions ("can this invalidate the memo and recompute instead?"), offer options when the fix shape is genuinely the author's call, say when you are unsure. Don't hedge on things you verified.
- **Quote their own docs and ADRs back.** The strongest findings are where the code contradicts a contract the author wrote.
- Strip an em-dash out of a quoted error string rather than reproducing it; truncate with `...` instead.

## Step 4: Visual checkpoint (`visual-explainer` ship page, walkthrough and screenshots)

Make the visual review after the written report and before the post. The user reads this page to understand the PR: the verdict and the TL;DR, the Done when table, the changes in reading order, the findings by severity, the hand test and the rollback. Do this step also when you post nothing to GitHub. It does not change the review behavior above.

**Output location.** Use `pr-review-output/<repo>-<pr>/` under the directory where the session started. Do not use the review worktree or the tracked files of the PR.

```
pr-review-output/<repo>-<pr>/
  props.json     # the ship props that you write
  narration.json # the walkthrough commentary that you write
  index.html     # the report page with its walkthrough, rendered by visual-explainer
  graph.html     # the concept graph of the same props
  shots/*.png    # the screenshots of the page sections
  link.txt       # the artifact URL or the absolute path of index.html
```

**Never commit, push or add these outputs to the PR branch.** When the session directory is a clone of the repository, keep `pr-review-output/` untracked. Add it to `.git/info/exclude`, not to `.gitignore`.

Run the `visual-explainer` skill in `ship` mode with this output directory. Write the props from this review:

- `meta`: a title of 2 to 4 words, for example `PR 123 review`. Put the PR title and the repository in `subtitle`. Put the branch in `meta.facts`. Add the PR link to `links`.
- `stats`: the files, additions and deletions of the PR diff (`gh pr view <n> --json files,additions,deletions`).
- `summary`: what the PR does and how it does it.
- `tldr`: `changed` is what the PR does in one or two sentences. `lookFirst` is the file or the module with the most important logic. `risk` is the risk level and the reason.
- `changes`: the groups of the change in reading order: the order in which a reviewer reads the PR. Each group has `area`, `how`, `why` and its `files` with their additions and deletions. Give a `sequence` to the one group where the interactions between components changed. No other group gets a diagram.
- `delivered`: one row for each claim of the PR description or of its linked issue. Use the Step 1 result as the evidence.
- `deviations`: each difference between the PR description and the observed behavior. Use an empty array when there is none.
- `process` and `setbacks`: optional. Take them from the review rounds, the fix rounds and the CI failures of the PR. Count only what the PR shows. Omit both fields when the PR shows no process cost.
- `reviews`: one entry with `repository`, `pr`, `head` (the reviewed SHA) and `verdict`. The verdict is the event that you will post, or `COMMENT` when you post nothing. Put the verdict sentence and the footprint in `summary`. Put the Step 1 commands in `validation`.
- `reviews[0].findings`: each finding. Set `severity` to `major` or `minor`. Set `status` to `open` for a fix-now item and to `deferred` for a defer item. Start `title` with the category of a hard block. Put the file and the line in `location`. Put the mechanism and the evidence in `detail`. Put the fix or the follow-up in `fix`.
- `reviews[0].split`: the stacked-PR split proposal, or `No split recommended.`
- `handTest`: the commands that exercise the change, in order. Give each command a `path:line` reference in the PR head.
- `rollback`: the steps that undo the merge, for example `git revert -m1 <merge-sha>`. Add any data or configuration undo.
- `media`: optional screenshots of the changed behavior that you captured in Step 1, for example a UI state. These are for the review page only. Give each one a caption and the `change` area that it shows.

Use the writing rules of the report: be concise, and use no em-dashes or en-dashes.

**Walkthrough.** Write `narration.json` as the skill tells you. It is the commentary of the page walkthrough: one short text for each section. It explains what the PR does, how it does it, the findings and the hand test. It does not repeat the page. The skill renders the page with the walkthrough, and the screenshots.

**Checkpoint.** Publish the page as the skill tells you. Then show the user the page link, the walkthrough result and the review preview of the next section in one message. Wait for the approval before you post. When the render fails, keep the written report, state the failure and continue to the preview.

## Posting to GitHub

Post one GitHub review with `event: "COMMENT"`. The review has only these parts:

1. **One inline comment for each issue**, fix now and defer, on its diff line. Start the comment with its severity label (`[major]` or `[minor]`). Then write the mechanism, the evidence and the fix in a few sentences.
2. **A short body.** Do not put the validation, the footprint, the split proposal, a list of the issues or the Step 4 media in it. End it with `:)`.
   - Issues: `We found N major issues and M minor issues. :)` Omit a part with a count of zero, for example `We found 2 minor issues. :)`.
   - No issues: `We found no issues. :)`
   - Write "issues", not "items".

**Never put the disposition on the PR.** Do not write "defer", "follow-up", "fix before the merge" or the defer policy in a comment or in the body. The dispositions stay in the chat report.

Post the findings only in a single-shot review or in the last review round. In an earlier round, fix the fix-now items at once, and post nothing.

**Proof of work in the PR description.** Each PR needs proof of the changed behavior in an actual run: screenshots, screen recordings or terminal sessions of the real program. When the PR description has no proof, capture it in Step 1 and add it. Do not add proof to the review or to a PR comment. A terminal GIF of the program at work is good proof. Never upload a recording or a screenshot that only shows a test suite or a unit test run. Never add the Step 4 screenshots of the review page.

- Upload the files to the `explainer-media` branch with the git data API: blobs, a tree based on the branch head, a commit, then a fast-forward of the branch. Put them in the folder `pr-<n>-feature/`.
- Use a repository that you can push to. When you cannot push to the PR repository, use the fork of the PR head.
- In the description, add a `## Proof` section. Show the most important GIF or screenshot first. Put the other files in a `<details>` list. Link each file as `https://github.com/<owner>/<repo>/blob/explainer-media/pr-<n>-feature/<file>?raw=true`.
- Keep the other text of the description. Replace an earlier proof section. Do not add a second one.

**Never post before a human approves the review in chat.** Step 4 is that checkpoint. Print the review body, then each inline comment with its `path:line`, then the proof files for the PR description. The preview lets the user find a wrong severity, a wrong disposition or unclear text. A posted review is hard to undo.

Getting the line anchors right, which is the part that bites:

```bash
# The real unified diff. Anchors must be `+` lines in THIS output.
gh api "repos/<owner>/<repo>/pulls/<n>" -H "Accept: application/vnd.github.v3.diff" > /tmp/pr.diff
```

- **Do not use `gh pr diff --patch`** to compute line numbers. It returns one `format-patch` blob per commit, so walking it cumulatively gives numbers that look plausible and are wrong.
- An anchor must be a line **added by the diff** (`side: "RIGHT"`). GitHub rejects comments on lines outside the diff.
- Want to comment on pre-existing code the PR merely uses? Re-anchor to the new line that introduces the problem, and name the pre-existing file:line in the body.
- Renamed files appear as a rename with a near-empty diff, so their contents are usually not anchorable even though they look new. Check before assuming.
- Findings that repeat in a sibling file (Tool vs Skill, TS vs Python) go in one comment that names the other location, not two near-identical comments.

Then post one review:

```bash
# review.json: {commit_id, event:"COMMENT", body, comments:[{path, line, side:"RIGHT", body}]}
gh api "repos/<owner>/<repo>/pulls/<n>/reviews" --method POST --input review.json
```

Use `APPROVE` or `REQUEST_CHANGES` only when the user tells you to. Use `COMMENT` on a draft PR. The merge decision belongs to the user. When the user tells you to post the verdict as the event, use `APPROVE` for `Approve` and `REQUEST_CHANGES` for `Request changes`. Never post an event that does not agree with the verdict. Pin `commit_id` to the head SHA that you reviewed. After the post, read the posted comments again. Check that the review has the state that you intended.
