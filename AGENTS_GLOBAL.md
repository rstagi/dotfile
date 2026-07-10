# Global Agent Instructions

- We are in 2026
- Be extremely concise in interactions and commit messages. Sacrifice grammar for concision when needed.

## Philosophy

The code you write will outlive you. Every shortcut becomes someone else's burden. Every hack compounds into technical debt that slows the whole team down.

You are not just writing code. You are shaping the future of every project you touch. The patterns you establish will be copied. The corners you cut will be cut again.

Fight entropy. Leave the code better than you found it.

## PR Comments

<pr-comment-rule>
  When I ask to add a comment to a PR with a TODO in it, use GitHub checkbox markdown:
  <example>
    - [ ] A description of the TODO goes here
  </example>
</pr-comment-rule>
- When tagging Claude in GitHub issues, tag `@claude`.

## GitHub

- The primary way to interact with GitHub must be the GitHub CLI.

## Plans

At the end of each plan give me a list of unresolved questions, if any. Make them extremely concise.

## TDD

- When working on the frontend, TDD is not mandatory unless working on business logic.
- For everything else, including backend, business logic, and libraries, use the tdd skill.

## Backend vs Frontend in Monorepos

- Unless the feature is frontend only, start with the backend in a TDD fashion, then connect it to the frontend.

## File Ordering Convention

Order code in files as follows:

1. Types and constants/global vars first
2. Most important function, the one giving the file its name or purpose
3. Other exported functions, starting with ones referenced earlier
4. Helper functions, with implementations after their first reference

Goal: top-down reading style. See high-level first, then dive into details. For circular references, keep them close; use first-referenced-in-main-function rule if in doubt.

## Commit Messages

- Do not mention Claude Code or Codex in commit messages.
- Never add Co-Authored-By agent lines.

## External Libraries

Always use context7 when I need code generation, setup or configuration steps, or library/API documentation. Automatically use the Context7 MCP tools to resolve library id and get library docs.

## Perplexity

Use Perplexity for up-to-date info beyond knowledge cutoff. Pick the right tool: `perplexity_search` for quick facts/news, `perplexity_ask` for conversational queries w/ web context, `perplexity_research` for deep investigation/reports, `perplexity_reason` for complex analysis/step-by-step reasoning.

## Browser Automation

Use the agent-browser skill for browser automation. Use `--session <name>` to preserve sessions.

## Documentation

- Don't create new docs proactively, but do update existing READMEs when changes affect them. Bundle README updates with the same commit.

## Ralph Integration

`ralph` runs autonomous agent iterations on GitHub/Linear issue PRDs.

**Flow:**
1. Prompts for source, GitHub or Linear
2. Fetches PRD from issue/project
3. Creates worktree at `~/.ralph-worktrees/<repo>-<suffix>`
4. Runs iterations, TDD for backend and browser validation for frontend
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

## When Inside Ralph Session

When you see "You are running inside a ralph session", follow these rules:

- Read PRD.md for requirements, progress.txt for completed work.
- Complete one task per iteration, commit, update progress, then stop.
- Backend/logic: strict TDD, write test -> RED -> implement -> GREEN -> refactor.
- Frontend: use frontend-design and validate with agent-browser after changes.
- Commit after each completed task with descriptive message.
- Update progress.txt after each task: mark complete, add notes.
- Output `<promise>COMPLETE</promise>` only when all PRD tasks are done.
- Output `<error>DESCRIPTION</error>` on blocking errors you can't resolve.
