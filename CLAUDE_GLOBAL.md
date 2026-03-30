- We are in 2026
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
- For EVERYTHING ELSE (backend, business logics in general, libraries, etc), use the /tdd skill.

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

Use `/agent-browser` skill for browser automation. Use `--session <name>` to preserve sessions.

## Documentation

- Don't create new docs proactively, but DO update existing READMEs when changes affect them (bundle with same commit)

## Ralph Integration

`ralph` - autonomous Claude on GitHub/Linear issue PRDs

**Flow:**
1. Prompts for source (GitHub or Linear)
2. Fetches PRD from issue/project
3. Creates worktree at `~/.ralph-worktrees/<repo>-<suffix>`
4. Runs Claude iterations (TDD for backend, Playwright for frontend)
5. Commits per task, logs progress as source comments
6. Opens draft PR when complete

**Commands:**
```bash
ralph                        # Interactive mode
ralph --github 123           # GitHub issue #123
ralph --github 123 30        # Max 30 iterations
ralph --linear ENG-456       # Linear issue
ralph --linear <project-uuid> # All issues in Linear project
```

**Cleanup:**
```bash
git worktree remove ~/.ralph-worktrees/<repo>-<suffix>
git branch -d ralph/<branch>
```

**Issue template:** `~/dotfile/templates/issue-prd-template.md`

## When inside ralph session

When you see "You are running inside a ralph session", follow these rules:

- Read PRD.md for requirements, progress.txt for completed work
- **ONE TASK PER ITERATION** - complete one task, commit, update progress, then stop
- **Backend/logic**: strict TDD (write test → RED → implement → GREEN → refactor), use the tdd skill
- **Frontend**: use the frontend-design skill and validate with `/agent-browser` after changes using the agent-browser skill
- Commit after each completed task with descriptive message
- Update progress.txt after each task (mark complete, add notes)
- Output `<promise>COMPLETE</promise>` only when ALL PRD tasks done
- Output `<error>DESCRIPTION</error>` on blocking errors you can't resolve
