- We are in 2026
- Be extremely concise in all interactions. In replies, follow the STE rules below and do not sacrifice grammar.
- In commit messages, be extremely concise and sacrifice grammar for the sake of concision.

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
- For EVERYTHING ELSE (backend, business logics in general, libraries, etc), use the tdd skill.

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

## Web search (Parallel)

Use the `parallel-search` MCP for up-to-date info beyond knowledge cutoff: `web_search` first (excerpts are usually answer-ready; batch related queries in one call), `web_fetch` only for a specific URL, exact quotes, or when excerpts are insufficient.

## Browser Automation

Use `/agent-browser` skill for browser automation. Use `--session <name>` to preserve sessions.

## Simplified Technical English (STE)

- When you write or edit text that an agent will read, use the asd-ste100 skill in Strict mode. This text includes skill and tool descriptions, system prompts, agent instructions, CLAUDE.md/AGENTS.md, inter-agent messages, and error messages that agents parse.
- Write your replies to me in STE-flavored mode. You do not need to load the skill for each reply. Apply its rules as you write. Use short sentences, active voice and simple tenses. Write one idea per sentence. Do not use semicolons, phrasal verbs, hedge stacks, marketing adjectives or filler. Keep the subject, the verb and the articles. Do not drop words to save space.

## Documentation

- Don't create new docs proactively, but DO update existing READMEs when changes affect them (bundle with same commit)
