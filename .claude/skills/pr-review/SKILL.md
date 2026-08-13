---
name: pr-review
description: Thoroughly review a pull request or branch by first checking it out into a local git worktree and actually running it, then auditing it across a fixed checklist — AI slop, obvious bugs / wrong behavior, test quality, code & file structure, implementation correctness, and docs consistency. Use this whenever the user asks to review a PR, review a pull request, review a branch, go over someone's changes before merge, or says things like "review #123", "can you look over this branch", "review this MR" — even if they don't say the word "skill". Prefer this over a quick diff-only read whenever the user wants a real review rather than a glance.
---

# PR review

The point of a review is to catch what a diff alone hides. Reading the patch tells you what changed; it doesn't tell you whether it works, whether the tests actually exercise it, or whether the docs still match reality. So this skill always starts by running the code, then works through a fixed set of lenses.

Don't skip the worktree step because the change "looks trivial." Trivial-looking changes are exactly where confident-but-wrong reviews come from.

**Writing style for the report and any posted comments:** be concise. Lead with the verdict, keep each finding to a few sentences, cut the narrative. Never use em-dashes or en-dashes; use commas, colons, parentheses, or separate sentences instead. See [Severity labels](#severity-labels) and [Writing the findings](#writing-the-findings) for how each finding is labelled and worded.

## Step 1 — Check it out and run it (do this first)

Get the branch into an isolated git worktree so you can build and run it without disturbing the user's working tree, then confirm it actually behaves as expected.

```bash
# GitHub PR by number:
gh pr checkout <number>            # or note the branch name it checks out

# Then put it in its own worktree (run from the repo root):
git fetch origin
git worktree add ../<repo>-review-<branch> <branch>
cd ../<repo>-review-<branch>
```

In the worktree, use the project's own commands (check the README / CLAUDE.md / package manifests) to:

- install dependencies and build,
- run the test suite,
- and exercise the actual change — run the app, hit the endpoint, call the function, whatever path this PR touches.

Note anything that doesn't build, doesn't pass, or doesn't behave as the PR claims. **That** is the most valuable thing a review produces — surface it before moving on. When you're done, clean up with `git worktree remove`.

## Step 2 — Review across these lenses

Go through the change with each of these in mind. For every issue, point to the specific file and line and say what's wrong and why.

- **AI slop** — generated filler that doesn't belong: hollow comments restating the code, dead scaffolding, invented abstractions used once, boilerplate that pads without adding meaning, hallucinated APIs.
- **Obvious bugs and wrong behavior** — logic that doesn't do what it claims, mishandled edge cases, off-by-ones, wrong conditions, broken error paths, regressions in existing behavior.
- **Tests** — are they coherent, complete, and well-informative? Do they actually exercise the new behavior (not just assert trivia)? Would they fail if the implementation were wrong? What important cases are missing?
- **Code & file structure** — is the overall shape clean? Are things in sensible places, named well, at the right altitude? Does it respect the project's conventions and general best practices, or fight them?
- **Implementation** — is it formally correct, clean, and coherent? No needless complexity, no copy-paste divergence, consistent with how the rest of the codebase solves similar problems.
- **Docs** — were they updated to match the change? Is the information complete and coherent, and are there any inconsistencies between what the docs say and what the implementation actually does?
- **Stacked-PR split** — should this single PR ship as a stack? Signals: over ~800 changed LOC, 2+ independently reviewable and revertable units, mechanical churn mixed with behavioral change. If `.loop/plan.md` exists in the repo, use its phases/lanes as candidate seams. If a split is warranted, propose it concretely: ordered list of PRs, each with title, base, and which commits/paths it takes (plain `gh` branch stack; no stacking tool assumed). Propose only, never execute the split.

## Step 3 — Report

Lead with the verdict and whether it ran cleanly in Step 1. Then list findings, each with a `[label]` (below) and a file:line reference, ordered most to least serious. Be honest when something is clean; don't manufacture findings to fill a section.

Before you hand the report over, reconcile the headline against the list. If the verdict says "two things to fix" and the list has six entries, one of them is wrong. Fix the mismatch rather than leaving the reader to spot it.

## Severity labels

Every finding carries exactly one label, first thing in its title, lowercase summary after it:

```
[minor] the artifact read yields before the dense permit is claimed
```

- `[blocker]` merge-stopping. Wrong behavior, data loss or corruption, a security hole, a broken published contract, or a regression in something that used to work.
- `[major]` should be fixed in this PR. A real defect or significant design problem, but there is a safe reason it could ship without it: narrow blast radius, fails loud, behind a flag, experimental surface.
- `[minor]` worth fixing, fine to defer. Degraded diagnostics, a documented contract that quietly stopped holding, a missing test for new behavior, an inconsistency that will mislead the next reader.
- `[nit]` small and optional. Naming, wording, a comment that is now wrong, a tidier way to say the same thing.
- `[style]` formatting and convention only, no behavior change. File ordering, import order, unrelated churn in the diff.

How to pick:

- **Default down, not up.** Reviewers inflate. `[blocker]` and `[major]` need a demonstrated failure, ideally the repro you ran in Step 1. Anything you only reasoned about is at most `[minor]`.
- **Severity is consequence, not effort.** A one-line fix for wrong behavior is still a blocker. A large refactor that only improves clarity is still a nit.
- **Fails loud beats fails silent.** Something that errors clearly is less severe than something that silently returns a wrong result, even when the error is more disruptive.
- **Say what is not affected.** "Plain semantic search is unaffected" is what makes a severity claim credible, and it stops the author over-reacting.
- **Keep label and wording in agreement.** Don't file a `[nit]` whose body says the process becomes unusable.
- If the user reassigns a severity, change the label and keep the technical content. Tell them if the new label now clashes with the body; don't silently water down a verified finding.

## Writing the findings

What makes a comment land:

- **Lead with the mechanism, not the symbol names.** "The permit is claimed in the synchronous NAPI body, so its timing depends on when JS calls it" beats "`resolveEmbeddingArtifact` yields before `experimentalWarmEmbeddingsFromArtifact`". If the author has to reconstruct the causal chain themselves, rewrite it.
- **Show the evidence you actually gathered.** Paste the repro steps, the command output, the failing assertion, the empty grep. A finding with observed output attached is not arguable; one without it is an opinion.
- **A timeline beats a paragraph** for anything involving ordering, concurrency, or lifecycle. Two short interleaved traces (works / broken) explain a race faster than prose.
- **Cut the connective tissue.** No "worth fixing because", "it's important to note", "flagging this since". State the defect, the proof, the fix.
- **Sound like a colleague.** Ask real questions ("can this invalidate the memo and recompute instead?"), offer options when the fix shape is genuinely the author's call, say when you are unsure. Don't hedge on things you verified.
- **Quote their own docs and ADRs back.** The strongest findings are where the code contradicts a contract the author wrote.
- Strip an em-dash out of a quoted error string rather than reproducing it; truncate with `...` instead.

## Posting to GitHub

Default to one GitHub review with `event: "COMMENT"`. Put every finding, including `[blocker]`, `[major]`, `[minor]`, `[nit]`, and `[style]`, in an inline review comment anchored to the relevant diff line. Do not duplicate findings in the overall body.

The overall review body must be concise and contain:

- the verdict,
- the validation result,
- the stacked-PR split proposal, or a short statement that no split is recommended,
- and `:)` as its final characters.

**Always show a preview and wait for approval before posting.** Print the review body, then each comment with its `path:line`. This is where severity mistakes and unclear wording get caught, and posting is not undoable in any tidy way.

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

Use `APPROVE` or `REQUEST_CHANGES` only when the user says so; gating a merge is their call, not yours. Pin `commit_id` to the head SHA you actually reviewed. Afterwards, verify: re-read the posted comments and confirm the review state landed as intended.

## Headless mode

When invoked with `--headless` (the loop-execute orchestrator runs this after opening the effort PR): never pause to ask anything, and never mutate the checkout you were started in. Skip `gh pr checkout` entirely. Instead, from a clone of the repo:

```bash
git fetch origin pull/<number>/head
git worktree remove --force ../<repo>-review-<number> 2>/dev/null || true   # leftover from a crashed run
git worktree add --detach ../<repo>-review-<number> FETCH_HEAD
```

The detached worktree sidesteps the branch-already-checked-out error when lane or integration worktrees still exist. Run Step 1 and Step 2 in that worktree as usual. Post one `COMMENT` review as described above: findings inline, concise overall body, stacked-PR split proposal included, and `:)` at the end. Headless mode skips the preview gate because nobody is present to approve it. Never use `APPROVE` or `REQUEST_CHANGES` unprompted. Clean up the worktree when done.
