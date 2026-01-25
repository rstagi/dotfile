- In all interactions and commit messages, be extremely concise and sacrifice grammar for the sake of concision.

## Philosophy

The code you write will outlive you. Every shortcut becomes someone else's burden. Every hack compounds into technical debt that slows the whole team down.

You are not just writing code. You are shaping the future of every project you touch. The patterns you establish will be copied. The corners you cut will be cut again.

Fight entropy. Leave the code better than you found it.

## PR Comments

<pr-comment-rule>
  When I ask to add a comment to a PR with a TODO in it, use the GitHub 'checkbox' markdown format to add the TODO. For instance:
  <example>
    - [ ] A description of the TODO goes here
  </example>
</pr-comment-rule>
- When tagging Claude in GitHub issues, tag '@claude'

## GitHub

- The primary way to interact with GitHub must be the github cli

## Plans

At the end of each plan give me a list of unresolved questions, if any. Make them extremely concise, sacrificing grammar for the sake of concision.

## TDD

- When working on the frontend, TDD is not mandatory unless we're working on a piece of business logic.
- For EVERYTHING ELSE (backend, business logics in general, libraries, etc), you ALWAYS need to start with the tests first.
- The TDD cycle MUST ALWAYS go through the following phases:
  - implement the test
  - run the test (RED)
  - implement the minimum amount of code to make the test pass
  - run the test (GREEN)
  - refactor if needed (REFACTOR)
  - end

## Backend vs Frontend in monorepos

- Unless the feature is frontend only, you must always start with implementing it on the backend (in a TDD fashion, as specified above), and only when that's done you tap that into the frontend as well

## File Ordering Convention

Order code in files as follows:
1. Types and constants/global vars first
2. Most important function (the one giving the file its name/purpose)
3. Other exported functions (starting with ones referenced earlier)
4. Helper functions (implementations come *after* their first reference)

Goal: top-down reading style - see high-level first, then dive into details. For circular references, keep them close; use first-referenced-in-main-function rule if in doubt.

## Commit messages

- don't mention claude code in commit messages
- never add Co-Authored-By Claude lines

## External Libraries

Always use context7 when I need code generation, setup or configuration steps, or library/API documentation. This means you should automatically use the Context7 MCP tools to resolve library id and get library docs without me having to explicitly ask.

## Perplexity

Use Perplexity for up-to-date info beyond knowledge cutoff. Pick the right tool: `perplexity_search` for quick facts/news, `perplexity_ask` for conversational queries w/ web context, `perplexity_research` for deep investigation/reports, `perplexity_reason` for complex analysis/step-by-step reasoning.

## Browser Automation

Use `/browser-use` skill for browser automation. Prefer `--browser real --session main` to preserve logins/cookies unless isolation needed.

## Documentation

- Don't create new docs proactively, but DO update existing READMEs when changes affect them (bundle with same commit)

## Ralph GitHub Integration

`ralphg <issue-number>` - autonomous Claude on GitHub issue PRD

**Flow:**
1. Fetches PRD from issue body
2. Creates worktree at `~/.ralph-worktrees/<repo>-issue-<N>`
3. Runs Claude iterations (TDD for backend, Playwright for frontend)
4. Commits per task, logs progress as issue comments
5. Opens draft PR when complete

**Commands:**
```bash
ralphg 123           # Run Ralph on issue #123
ralphg 123 30        # Max 30 iterations
```

**Cleanup:**
```bash
git worktree remove ~/.ralph-worktrees/<repo>-issue-<N>
git branch -d ralph/issue-<N>
```

**Issue template:** `~/dotfile/templates/issue-prd-template.md`

## When inside ralphg session

When you see "You are running inside a ralphg session", follow these rules:

- Read PRD.md for requirements, progress.txt for completed work
- **ONE TASK PER ITERATION** - complete one task, commit, update progress, then stop
- **Backend/logic**: strict TDD (write test → RED → implement → GREEN → refactor)
- **Frontend**: validate with `browser-use --browser real --session main --headed` after changes
- Commit after each completed task with descriptive message
- Update progress.txt after each task (mark complete, add notes)
- Output `<promise>COMPLETE</promise>` only when ALL PRD tasks done
- Output `<error>DESCRIPTION</error>` on blocking errors you can't resolve
