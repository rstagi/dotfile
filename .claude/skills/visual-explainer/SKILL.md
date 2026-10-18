---
name: visual-explainer
description: >-
  Render one animated, self-contained HTML explainer page from a props JSON file.
  Mode `plan` shows the goal, the architecture, the milestone and phase timeline,
  the decisions with their rejected options, and the risks. Mode `ship` shows what
  each Done when delivered and the deviations from the plan. It also shows the review
  verdict and findings, a hand-test checklist with code references, and the rollback
  steps. Use it after multiphase-plan registers a plan, at a Loop milestone
  merge gate, at a Loop milestone resume, and in pr-review Step 4. You write only the
  props JSON. The skill owns the HTML, the CSS and the JS.
argument-hint: "plan|ship <output-dir>"
---

# Visual Explainer

This skill turns facts into one fixed, animated HTML page. You write the facts as a props
JSON file. A zero-dependency Node script checks the props against a schema. Then it
escapes all text, puts it into fixed scene components, and writes one `index.html`.

`<skill>` below is the directory of this file. In a self-hosted Loop run, use
`$RUNTIME_ROOT/.claude/skills/visual-explainer`.

## Files

| File | Purpose |
|------|---------|
| `template/page.html` | The page: color tokens, dark mode, motion rules, scene styles, a small script |
| `scripts/render.mjs` | Checks the props, then writes `index.html` (Node 20.11 or later) |
| `scripts/scenes.mjs` | The fixed scene components |
| `scripts/validate.mjs` | The schema checker |
| `schema/plan.schema.json` | The props of `plan` mode |
| `schema/ship.schema.json` | The props of `ship` mode |
| `examples/*.props.json` | One valid example for each mode |

## Rules

1. Write only the props JSON file. Do not write HTML, CSS or JS.
2. Do not edit the template, the scripts or the schemas during a run.
3. Write facts only. Take each fact from the plan, the ledger, a review report, the PR or
   the code. Do not invent a count, a result or a finding.
4. Omit an optional field when you do not know its value.
5. Write plain text. A pair of backticks marks inline code. A newline is a line break.
6. Do not put secrets, tokens or credentials in the props.
7. `meta.title` has 2 to 4 words. It is the page title.
8. Do not commit the output directory to the reviewed branch.

## Workflow

### 1. Choose the mode and the output directory

The caller names the mode and the output directory. Write the props to
`<output-dir>/props.json`.

- `plan` explains work that starts. Use it for a whole plan or for one milestone.
- `ship` explains work that is ready to merge. Use it for a milestone gate or for one PR.

### 2. Collect the facts

Read `schema/<mode>.schema.json` and the example of that mode. Then collect these facts.

**`plan` mode:**

- `goal`: the Goal section of the plan.
- `architecture`: layers from the user-facing layer to the storage layer. Each layer has
  components. Each component has a role and an optional status:
  `new`, `changed`, `existing` or `removed`.
- `milestones`: copy the numbers, titles, kinds, lanes, repositories, `Depends on` edges
  and estimates of the phases. Shorten each Done when to one or two sentences.
- `decisions`: the key decisions of the plan. Each decision has at least one rejected
  option and the reason. When the plan names no alternative, use the current behavior
  that the decision replaces.
- `risks`: the risks of the plan and of its plan review. Give each risk an impact and a
  mitigation.

**`ship` mode:**

- `delivered`: one row for each Done when in scope. Set `status` to `met`, `partial` or
  `missed`. Take the evidence from the progress log, the ledger and the review.
- `deviations`: each difference between the plan and the result. Use an empty array when
  there is no difference.
- `process`: the process numbers. This field is optional. In a Loop run, the caller gives
  the output file of `loop-retro.mjs --json` (the retro facts). Copy each number from its
  `totals`. Omit a number that you do not know. Do not count by hand.
- `setbacks`: the process costs. This field is optional. A deviation is a result that is
  different from the plan. A setback is a cost on the way to the result, also when the result
  agrees with the plan. Examples: a retry, a failed test run, a timeout, a rate limit, a
  question, a HIL, a review fix round.
  - In a Loop run, take every setback from the `setbackCandidates` of the retro facts. Do
    not add a setback that is not in that list. Put candidates of one phase with one cause
    into one entry.
  - The evidence paths of a candidate are relative to the `.loop` directory.
  - Find the cause in the evidence files of the candidate: the `status.json` or `last.md` of
    the run, or the HIL brief and its answer. Write the cause in `what`. When no file gives
    the cause, write `Cause: unknown`.
  - Write `cost` from the candidate facts: attempts, minutes, rounds. Do not invent a count.
  - Write `lesson` only when the evidence supports a rule that an agent can apply.
  - The page shows the `process` numbers and the `setbacks` in one scene after the
    deviations. The page omits that scene when both fields are absent.
- `reviews`: one entry for each repository or PR. `verdict` is the posted GitHub event.
  Copy the findings of the final review. Set each finding to `open`, `fixed` or `deferred`.
- `handTest`: the ordered steps for a human. Each step has `command`, `ref` and `why`.
- `rollback`: the ordered steps that undo the change.

### 3. Check every hand-test command against the code

Do this step for each `handTest` entry before you write it.

1. Find the code that the command runs or depends on. Examples: the installer, the CLI
   parser, the daemon start.
2. Open that file at the reviewed head. Use `git show <head>:<path>` when the checkout
   is on a different commit.
3. Compare the command name, the flags, the paths, the labels and the working directory
   with the code.
4. Find each side effect of each step. Examples: a daemon start, an import, a migration,
   a file write.
5. Put a step that reads the old state before the step that changes that state.
6. Write `ref` as `path:line` of the line that proves the command or its position.
7. Write `why` as one or two sentences that name that effect.

When the code contradicts the plan or a checklist that you received, follow the code.
Then add a `deviations` entry for the difference. The schema rejects a step without
`ref`.

### 4. Check the props

```bash
node <skill>/scripts/render.mjs --props <output-dir>/props.json --check
```

- Exit 0: the props are valid.
- Exit 1: each stderr line is `<field path>: <error>`. Fix each field, then run the
  check again.
- Exit 2: a usage error or an I/O error.

### 5. Render

```bash
node <skill>/scripts/render.mjs --props <output-dir>/props.json --out <output-dir>
```

The script prints the absolute path of `<output-dir>/index.html`. It writes nothing for
invalid props.

### 6. Publish

- When the session has the Artifact tool, publish `index.html` as a private artifact.
  Write the artifact URL to `<output-dir>/link.txt`.
- When the session has no Artifact tool, write the absolute path of `index.html` to
  `<output-dir>/link.txt`.
- A later run with the same output directory publishes to the same artifact. Pass the URL
  from `link.txt` when the session did not publish it.

A headless runner has no Artifact tool. It writes and checks the props only. Its
caller renders and publishes.

### 7. Report

Report one line: the mode, then the artifact URL or the absolute path of `index.html`.
When a step fails, report the failure and the last stderr lines. A failed explainer never
blocks the work of the caller.

## The page

The template meets the Claude artifact page contract:

- The `<title>` is `meta.title`, 2 to 4 words.
- Color tokens are on `:root`. Dark tokens are under `prefers-color-scheme: dark` and
  `:root[data-theme="dark"]`. `body` has an explicit background.
- There are no external scripts, styles or assets.
- The layout fits a phone with a 16px side gutter and no horizontal page scroll.
- Motion runs only under `prefers-reduced-motion: no-preference`.
- The renderer escapes all props text. A link URL must start with `http://` or `https://`.

The page animates with SVG and CSS. A small script reveals each scene on scroll, copies
commands, and runs the Play tour. Without the script, the page shows every scene.

To change the template, the scenes or a schema, edit the skill files. Then run
`node --test tests/visual-explainer.test.mjs` from the repository root.
