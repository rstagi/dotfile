---
name: pr-review
description: Review a pull request or a branch in one round. First, check it out into a local git worktree and run it. Then put each finding into one of two classes. Blocking is only for security, unprotected breakage of existing behavior or a failing deterministic check. Deferred is for all other items. Report the footprint of the PR and a verdict that follows the Blocking section. Use this skill when the user asks to review a PR, a pull request or a branch, or to examine changes before a merge. Examples are "review #123", "can you look over this branch" and "review this MR". Use it also when the user does not say the word "skill". Use it instead of a quick diff-only read when the user wants a real review.
---

# PR review

> Source of truth for these rules: [`CODING_STANDARDS.md`](../../../CODING_STANDARDS.md). When this skill and that file differ, the file wins. Update the file first.


A review judges a PR on two tests. The PR does not break existing behavior. The PR is easy to remove. A diff alone does not show if the code works or if the tests exercise it. Thus this skill always runs the code first. Then it puts each finding into one of two sections: Blocking or Deferred.

Do not skip the worktree step when the change looks trivial. A confident but wrong review often starts with a change that looks trivial.

**Writing style for the report and the posted comments:** Be concise. Start with the verdict. Keep each finding to a few sentences. Do not tell a story. Never use em dashes or en dashes. Use commas, colons, parentheses or separate sentences. [Finding labels](#finding-labels) and [Writing the findings](#writing-the-findings) tell how to label and write each finding.

## One round

Review a PR one time. Do not review the same PR again to find new small issues. When the user asks you to check a fix, check only the earlier Blocking items. Also check that the fix adds no new Blocking item. Do not add Deferred items in that check.

## Step 1 — Check it out and run it (do this first)

Create a detached worktree before you build or test the branch.
Run this block and all review commands in one persistent shell.
Exit that shell only after the review ends.

```bash
# Fetch a PR. For a branch review, use: git fetch origin <branch>
git fetch origin pull/<number>/head
REVIEW_REPO=$(git rev-parse --show-toplevel)
REVIEW_REF=FETCH_HEAD

if command -v ws >/dev/null 2>&1 && ws ping >/dev/null 2>&1; then
  if REVIEW_SESSION=$(ws whoami 2>/dev/null); then
    : # The daemon collects the worktree when this session finishes or fails.
  else
    REVIEW_SESSION=$(ws session create) || exit 1
    trap 'cd "$REVIEW_REPO"; ws session stop "$REVIEW_SESSION" >/dev/null' EXIT
  fi
  REVIEW_WORKTREE=$(ws phase worktree --session "$REVIEW_SESSION" "$REVIEW_REPO" "$REVIEW_REF") || exit 1
else
  REVIEW_ROOT="${TMPDIR:-/tmp}/pr-review"
  mkdir -p "$REVIEW_ROOT" || exit 1
  REVIEW_ROOT=$(cd "$REVIEW_ROOT" && pwd -P) || exit 1
  REVIEW_TMP=$(mktemp -d "$REVIEW_ROOT/review.XXXXXX") || exit 1
  REVIEW_WORKTREE="$REVIEW_TMP/worktree"
  trap 'cd "$REVIEW_REPO"; git worktree remove --force "$REVIEW_WORKTREE"; git worktree prune; rmdir "$REVIEW_TMP"' EXIT
  git worktree add --detach "$REVIEW_WORKTREE" "$REVIEW_REF" || exit 1
fi
cd "$REVIEW_WORKTREE" || exit 1
```

In the worktree, use the project's own commands (check the README / CLAUDE.md / package manifests) to:

- install dependencies and build,
- run the test suite,
- run each other deterministic check that the project has, for example architecture tests, the flag off/on test matrix and the delete test,
- run the existing tests with the flag off when the PR adds a feature flag,
- and exercise the actual change: run the app, call the endpoint or call the function that this PR touches.

Record each check that fails and each existing behavior that changes. These results are the evidence for the Blocking categories `failing-check` and `unprotected-breakage`. Report them before you continue. The daemon removes managed worktrees when their sessions finish or fail.
The exit trap removes fallback worktrees.

## Step 2 — Classify each finding

Give each finding exactly one class: `blocking` or `deferred`. Point to the file and the line. Say what is wrong and why.

### Blocking

A finding is `blocking` only when it is in one of these 3 categories. Each Blocking item names its category and its evidence.

| Category | The finding is blocking when | Evidence |
|---|---|---|
| `security` | The PR leaks a secret, bypasses auth or a permission check, exposes data or handles user data unsafely. Or the change can cause real harm in production. | The code path, and a repro when you can make one. |
| `unprotected-breakage` | The PR changes current behavior, and no feature flag or equivalent off switch protects the change. Or existing tests fail with the flag off. | The Step 1 result: the old and the new behavior, or the failing existing test. |
| `failing-check` | A deterministic check fails: the build, the tests, the architecture tests, the flag off/on matrix or the delete test. | The command and its failing output from Step 1. |

A `failing-check` item blocks because the check fails, not because you judge the code. When the project does not have a check, a probable failure of that check is `deferred`.

### Deferred

All other findings are `deferred`. They never block. Use these lenses to find them:

- **Bugs in protected code**: wrong logic or missing edge cases in new code that a flag or an off switch protects. A new entry point that no caller uses yet is an off switch.
- **Tests**: tests that do not exercise the new behavior, and important cases that are missing.
- **AI slop**: comments that only repeat the code, dead scaffolding, abstractions with one use, invented APIs.
- **Structure and names**: names, style, extra abstractions, code in the wrong place, cleaner refactors.
- **Docs**: docs that do not agree with the code.
- **Performance**: ideas that make the code faster.

Keep the Deferred list short and concrete. Remove each item that does not deserve a follow-up ticket.

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

For each Blocking item, give the smallest fix that removes the block. Then find the condition that let the problem occur, for example a missing check, test, type, validation or boundary. When the evidence shows a structural cause, add the preventive change as a Deferred item. When the evidence shows an isolated problem, say so. Do not invent a broad refactor.

## Step 3 — Report

Write the report in this order:

1. **Verdict**: `Approve` when Blocking is empty. `Request changes` when Blocking has one or more items. Add the counts, for example `Request changes. 1 Blocking, 3 Deferred.`
2. **Validation**: the Step 1 commands and the result of each command.
3. **Footprint**: the count, the list, the discussion flag when the count is more than 3, and the split proposal or `No split recommended.`
4. **Blocking**: each item with its label, its file:line, its evidence and its fix. When Blocking is empty, write `None.`
5. **Deferred**: each item with its label, its file:line and one or two sentences. When Deferred is empty, write `None.`

The report has exactly these two finding sections. Do not add other finding sections. When the PR is clean, say so. Do not invent findings to fill a section.

Before you give the report, compare the verdict with the sections. The verdict is `Approve` only when Blocking is empty. When the counts in the verdict do not agree with the lists, fix the counts.

## Finding labels

Each finding has exactly one label at the start of its title. A short summary in lowercase follows the label:

```
[blocking: failing-check] the migration test fails on an empty database
[deferred] the retry helper has one caller
```

The labels are `[blocking: security]`, `[blocking: unprotected-breakage]`, `[blocking: failing-check]` and `[deferred]`.

How to choose:

- **Never raise the class of an item to put it in Blocking.** When you are not sure, the item is `deferred`.
- **A Blocking item needs evidence.** Use the repro or the failing output from Step 1. When you only reasoned about a problem, the item is `deferred`.
- **The consequence sets the class, not the size of the fix.** A one-line fix for an unprotected break is still Blocking. A large refactor that only makes the code clearer is still Deferred.
- **Say what the problem does not affect.** For example: "Plain semantic search is not affected." This makes the class credible. It also stops a reaction from the author that is too strong.
- **Keep the label and the text in agreement.** Do not write a `[deferred]` item whose text says that the process becomes unusable.
- When the user changes the class of an item, change the label and keep the technical content. When the new label does not agree with the text, tell the user. Do not make a verified finding weaker without a statement.

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
- `process` and `setbacks`: optional. Take them from the review rounds, the fix rounds and the CI failures of the PR. Count only what the PR shows. Omit both fields when the PR shows no process cost. Do not write a retro file outside a Loop run.
- `reviews`: one entry with `repository`, `pr`, `head` (the reviewed SHA) and `verdict`. The verdict is the event that you will post, or `COMMENT` when you post nothing. Put the verdict sentence and the footprint in `summary`. Put the Step 1 commands in `validation`.
- `reviews[0].findings`: each Blocking item and each Deferred item. For a Blocking item, set `severity` to `blocker` and `status` to `open`, and start `title` with its category. For a Deferred item, set `severity` to `minor` and `status` to `deferred`. Put the file and the line in `location`. Put the mechanism and the evidence in `detail`. Put the fix or the follow-up in `fix`.
- `reviews[0].split`: the stacked-PR split proposal, or `No split recommended.`
- `handTest`: the commands that exercise the change, in order. Give each command a `path:line` reference in the PR head.
- `rollback`: the steps that undo the merge, for example `git revert -m1 <merge-sha>`. Add any data or configuration undo.
- `media`: optional screenshots of the changed behavior that you captured in Step 1, for example a UI state or a terminal output. Give each one a caption and the `change` area that it shows.

Use the writing rules of the report: be concise, and use no em-dashes or en-dashes.

**Walkthrough.** Write `narration.json` as the skill tells you. It is the commentary of the page walkthrough: one short text for each section. It explains what the PR does, how it does it, the findings and the hand test. It does not repeat the page. The skill renders the page with the walkthrough, and the screenshots.

**Checkpoint.** Publish the page as the skill tells you. Then show the user the page link, the walkthrough result and the review preview of the next section in one message. Wait for the approval before you post. When the render fails, keep the written report, state the failure and continue to the post.

## Posting to GitHub

Default to one GitHub review with `event: "COMMENT"`. Put each Blocking item in an inline review comment on the relevant diff line. Do not repeat Blocking items in the overall body. Put the Deferred items only in the overall body, so that the author does not fix them in this PR.

The overall review body must be concise and contain:

- the verdict and the counts,
- the validation result,
- the footprint, and the split proposal or `No split recommended.`,
- the Deferred list, with a `path:line` for each item,
- the media block of Step 4, when the page rendered,
- and `:)` as its final characters.

**Media block.** Upload the screenshots of Step 4 to the `explainer-media` branch of the repository, and get the Markdown block:

```bash
node <visual-explainer>/scripts/gh-media.mjs --repo <owner>/<repo> --key pr-<n> --dir pr-review-output/<repo>-<pr> --url "$(cat pr-review-output/<repo>-<pr>/link.txt)"
```

`<visual-explainer>` is the directory of the `visual-explainer` skill. For the preview, add `--dry-run`: the command uploads nothing and prints the same block. Upload only after the approval. Put the printed block in the review body before `:)`. Omit `--url` when `link.txt` holds a local path. The block shows the header and the TL;DR screenshots, and the other sections in a collapsed list. The header screenshot links to the page. When the command fails, post the review without the block and state the failure.

**Always show a preview and wait for approval before posting.** Step 4 is that checkpoint. Print the review body, then each comment with its `path:line`. The preview lets the user find a wrong class or unclear text. A posted review is hard to undo.

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

Use `APPROVE` or `REQUEST_CHANGES` only when the user tells you to. The merge decision belongs to the user. When the user tells you to post the verdict as the event, use `APPROVE` for `Approve` and `REQUEST_CHANGES` for `Request changes`. Never post an event that does not agree with the verdict. Pin `commit_id` to the head SHA that you reviewed. After the post, read the posted comments again. Check that the review has the state that you intended.

## Headless mode

In `--headless` mode, do not ask questions.
Do not change the starting checkout.
Do not run `gh pr checkout`.
Use the Step 1 block with `git fetch origin pull/<number>/head` and `REVIEW_REF=FETCH_HEAD`.
Use `ws phase worktree` when `ws ping` succeeds.
Skip Step 4 unless the prompt asks for it. A headless session has no person to show the checkpoint to.
Otherwise, use the absolute `${TMPDIR:-/tmp}/pr-review/` path and the `trap … EXIT` cleanup from Step 1.
Keep the same shell active until the review ends.

The detached worktree sidesteps the branch-already-checked-out error when lane or integration worktrees still exist. Run Step 1 and Step 2 in that worktree as usual. Post one `COMMENT` review as described above. Put the Blocking items inline. Put the verdict, the footprint, the split proposal and the Deferred list in a concise overall body. End the body with `:)`. Headless mode skips the preview gate because nobody is present to approve it. Never use `APPROVE` or `REQUEST_CHANGES` unprompted. Let the session cleanup or the exit trap remove the worktree.
