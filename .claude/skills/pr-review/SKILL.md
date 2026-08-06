---
name: pr-review
description: Thoroughly review a pull request or branch by first checking it out into a local git worktree and actually running it, then auditing it across a fixed checklist — AI slop, obvious bugs / wrong behavior, test quality, code & file structure, implementation correctness, and docs consistency. Use this whenever the user asks to review a PR, review a pull request, review a branch, go over someone's changes before merge, or says things like "review #123", "can you look over this branch", "review this MR" — even if they don't say the word "skill". Prefer this over a quick diff-only read whenever the user wants a real review rather than a glance.
---

# PR review

The point of a review is to catch what a diff alone hides. Reading the patch tells you what changed; it doesn't tell you whether it works, whether the tests actually exercise it, or whether the docs still match reality. So this skill always starts by running the code, then works through a fixed set of lenses.

Don't skip the worktree step because the change "looks trivial." Trivial-looking changes are exactly where confident-but-wrong reviews come from.

**Writing style for the report and any posted comments:** be concise. Lead with the verdict, keep each finding to a few sentences, cut the narrative. Never use em-dashes; use commas, colons, parentheses, or separate sentences instead.

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

Lead with the verdict and whether it ran cleanly in Step 1. Then list findings grouped by the lenses above, each with a file:line reference, ordered most to least serious. Separate must-fix issues from nice-to-haves. Be honest when something is clean — don't manufacture findings to fill a section.

## Headless mode

When invoked with `--headless` (the loop-execute orchestrator runs this after opening the effort PR): never pause to ask anything, and never mutate the checkout you were started in. Skip `gh pr checkout` entirely. Instead, from a clone of the repo:

```bash
git fetch origin pull/<number>/head
git worktree remove --force ../<repo>-review-<number> 2>/dev/null || true   # leftover from a crashed run
git worktree add --detach ../<repo>-review-<number> FETCH_HEAD
```

The detached worktree sidesteps the branch-already-checked-out error when lane or integration worktrees still exist. Run Step 1 and Step 2 in that worktree as usual. Post the full report as a PR comment via `gh pr comment`: lead with the verdict line, include the stacked-PR-split section. Still clean up the worktree when done.
