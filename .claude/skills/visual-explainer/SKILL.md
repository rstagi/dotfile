---
name: visual-explainer
description: >-
  Render a plan or a PR review as one concise report page with a guided walkthrough. Also render
  screenshots of its sections. You write only a props JSON file and a narration JSON file. Mode `plan`
  shows the summary box with the requests to the reader, the Done when table and the phases. It
  also shows one before/after architecture diagram, the decision cards and the rabbit holes. Mode `ship`
  shows the verdict, the summary box, the Done when results and the changes in reading order with
  one sequence diagram. It also shows the findings by severity, the hand test and the rollback. With
  `meta.checkpoint`, `ship` mode shows a progress checkpoint of a running milestone. The
  walkthrough focuses one section at a time and shows the narration text as the commentary. A
  concept graph of the same props is a second page. Use it in multiphase-plan step 5 and in pr-review Step 4. Also use it at Loop
  progress checkpoints, at a Loop milestone merge gate and at a Loop milestone resume. Explore mode draws any free-form graph that you write as JSON (an
  architecture, a flow, a data model, dependencies) as one interactive page. The reader hovers a node
  to see what it touches, clicks it for details, drags nodes, turns the layout and steps through a
  walkthrough. Use it when a picture explains better than prose and there is no plan or ship report.
  The run graph shows one Loop milestone as it ran: the plan lanes, what happened to each phase,
  the review and a time bar with the checkpoints. Use it at a Loop milestone merge gate.
argument-hint: "plan|ship <output-dir>"
---

# Visual Explainer

This skill turns facts into a report that a person reads from top to bottom in a few minutes.
You write the facts as a props JSON file and the walkthrough commentary as a narration JSON file.
Zero-dependency Node scripts check both files and write `index.html` (the report with its
walkthrough) and `graph.html` (a concept graph of the same facts). A second script takes
screenshots of the report sections. A third script puts the screenshots on GitHub for a PR.

`<skill>` below is the directory of this file. In a self-hosted Loop run, use
`$RUNTIME_ROOT/.claude/skills/visual-explainer`.

## Files

| File | Purpose |
|------|---------|
| `template/report.html` | The report page: color tokens, dark mode, styles |
| `template/walkthrough.js` | The walkthrough script that the report inlines when it has a narration |
| `template/page.html` | The concept graph page |
| `template/explore.html` | The explore page: ELK layout in the browser, hover, details, drag, minimap, walkthrough |
| `scripts/explore.mjs`, `scripts/explore-spec.mjs` | Explore mode: the graph check and the page writer |
| `scripts/timeline.mjs` | The check of the optional time bar data of an explore page |
| `scripts/run-graph.mjs` | The run graph: one Loop milestone as it ran, as an explore page with a time bar |
| `scripts/run-facts.mjs`, `scripts/run-model.mjs` | The run graph facts of both Loop engines, and the graph and timeline from those facts |
| `vendor/elk.bundled.js` | elkjs 0.11.0, inlined into the explore page (EPL-2.0, see `vendor/NOTICE`) |
| `examples/explore-login.graph.json` | One valid example of an explore graph |
| `scripts/render.mjs` | Checks the props and the narration, then writes `index.html` and `graph.html` (Node 20.11 or later) |
| `scripts/report.mjs`, `scripts/diagrams.mjs` | The report sections, the before/after diagram and the sequence diagram |
| `scripts/graph.mjs`, `scripts/page.mjs` | The concept graph: nodes, edges, layout and page data |
| `scripts/shoot.mjs` | Screenshots of the report sections with a headless Chrome |
| `scripts/narration.mjs` | The narration check |
| `scripts/gh-media.mjs` | Uploads the screenshots to GitHub, prints a Markdown block |
| `scripts/props.mjs`, `scripts/validate.mjs` | The props check and the schema checker |
| `schema/plan.schema.json`, `schema/ship.schema.json` | The props of each mode |
| `schema/narration.schema.json` | The narration, version 3: one commentary text for each walkthrough step |
| `examples/*.props.json`, `examples/*.narration.json` | One valid example for each mode |

## Rules

1. Write only the props JSON file and the narration JSON file. Do not write HTML, CSS or JS.
2. Do not edit the templates, the scripts or the schemas during a run.
3. Write facts only. Take each fact from the plan, the ledger, a review report, the PR or
   the code. Do not invent a count, a result, a finding, a file or a relation.
4. Omit an optional field when you do not know its value.
5. Write plain text. A pair of backticks marks inline code. A newline is a line break.
6. Do not put secrets, tokens or credentials in the props, the narration or the media.
7. `meta.title` has 2 to 4 words. It is the page title.
8. Be concise. The reader must get the point from the summary box alone. Each sentence has one fact.
9. Do not commit the output directory to the reviewed branch.

## Workflow

### 1. Choose the mode and the output directory

The caller names the mode and the output directory. Write the props to
`<output-dir>/props.json` and the narration to `<output-dir>/narration.json`.

- `plan` explains work that starts. Use it for a whole plan or for one milestone.
- `ship` explains work that is ready to merge. Use it for a milestone gate or for one PR.
- `ship` with `meta.checkpoint` explains the progress of a running milestone.

### 2. Collect the facts

Read `schema/<mode>.schema.json` and the example of that mode. Then collect these facts. Each
section of the page comes from the fields after its name.

**`plan` mode, in page order:**

- Header: `meta` with `facts`, for example the skill path, the route, the flag and the estimate.
- Summary box: `tldr.goal` is the goal in one or two sentences. `tldr.ask` is what you need
  from the reader: the approvals and the open questions. `tldr.risks` has the top 2 or 3 risks.
- Done when: `doneWhen`, each criterion with how a person checks it.
- Phases: `milestones` with their phases. Copy the numbers, titles, kinds, lanes, repositories,
  `dependsOn` edges and estimates. Give each phase a one-line `summary`.
- Architecture: `architecture.layers` from the user-facing layer to the storage layer, and
  `architecture.flows`. Give each component and each flow a `status`: `new`, `changed`,
  `existing` or `removed`. The page draws the before diagram and the after diagram from these
  statuses.
- Decisions: `decisions`, each with a short `title`, the decision, at least one rejected option
  and the reason. When the plan names no alternative, use the current behavior that the
  decision replaces.
- Rabbit holes: `rabbitHoles`, one line for each scope limit or deferred item. Start a no-go
  with `No `.
- Full detail: `risks`, each with an impact and a mitigation, and `goal`.

**`ship` mode, in page order:**

- Header: `meta` with `facts` (the PR, the branch, the head), and `stats` (files, additions,
  deletions). The page shows the verdict from `reviews`.
- summary box: `tldr.changed`, `tldr.lookFirst` (the file or the module to read first) and
  `tldr.risk` (the risk level and the reason). `summary` is a longer version for Full detail.
- Done when: `delivered`, one row for each Done when in scope. Set `status` to `met`,
  `partial`, `missed` or, in a checkpoint only, `pending`. Give each row its `phase` number.
- Changes: `changes` in reading order: the order in which a reviewer reads the change. Each
  group has `area`, `how`, `why` and its `files` with their real additions and deletions. Give
  a `sequence` to the one group where the interactions between components changed. Only one
  group can have a sequence. Take each message of the sequence from the code.
- Findings: `reviews`, one entry for each repository or PR. `verdict` is the posted GitHub
  event. Copy the findings of the final review. Set each finding to `open`, `fixed` or
  `deferred`.
- Hand test: `handTest`, the ordered steps for a human. Each step has `command`, `ref` and `why`.
- Rollback: `rollback`, the ordered steps that undo the change.
- Full detail: `deviations` (each difference between the plan and the result, or an empty
  array), `process` and `setbacks`.
  - `process`: in a Loop run, copy each number from the `totals` of the retro facts
    (`loop-retro.mjs --json`). Omit a number that you do not know. Do not count by hand.
  - `setbacks`: the process costs, for example a retry, a timeout, a HIL or a review fix round.
    In a Loop run, take each setback from the `setbackCandidates` of the retro facts. Find the
    cause in the evidence files of the candidate, or write `Cause: unknown`. Write `cost` from
    the candidate facts. Write `lesson` only when the evidence supports a rule.

**Checkpoint (`ship` mode):** set `meta.checkpoint` to `{number, phasesDone, phasesTotal, since}`.
`reviews` is an empty array. A phase that has not merged gets `delivered` rows with the status
`pending`. The page shows the progress instead of a verdict.

**Media (both modes):** `media` adds screenshots that you captured, for example a UI state or a
terminal output. `path` is relative to the props file and ends in `.png`, `.jpg`, `.jpeg`, `.webp`
or `.gif`. Each file is at most 2 MB. In `ship` mode, `change` puts the figure in that change
group. The page inlines each image.

**Relation fields (optional, both modes):** these fields add edges to the concept graph:
`milestones[].phases[].components`, `decisions[].phases`, `decisions[].components`,
`risks[].mitigatedBy`, `risks[].components`, `changes[].phases`, `deviations[].phases`,
`reviews[].findings[].phases`, `reviews[].findings[].changes` and `handTest[].phases`. Add a
relation only when a source states it. A phase number must be a phase of the props. A component
name or a change area must match exactly. The check rejects an unknown id with its field path.

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

### 4. Write the narration

The narration is the commentary of the walkthrough. The walkthrough has one step for each
narration entry. Each step focuses its section of the page and shows the text in a card. The
reader moves with Next and Back. Read `schema/narration.schema.json` and the example narration of
the mode. The file has this shape:

```json
{ "version": 3, "mode": "<mode>", "scenes": [{ "scene": "<id>", "text": "<commentary>" }] }
```

The scene ids, in their fixed order:

- `plan`: `intro`, `tldr`, `done-when`, `phases`, `architecture`, `decisions`, `rabbit-holes`,
  `outro`.
- `ship`: `intro`, `tldr`, `done-when`, `changes`, `findings`, `hand-test`, `rollback`, `outro`.

`intro` focuses the header. `outro` focuses no section: its card is in the middle of the page.
Each other id focuses the section with that id.

Plan the steps by these rules:

1. Start with `intro`. Keep the order above. Use each id one time at most.
2. Omit a step to make the walkthrough shorter. The walkthrough has no step that the narration
   omits.
3. Do not add a step whose section has no data, for example `findings` in a checkpoint. The
   check rejects it.

Write the text by these rules:

1. Explain. Do not repeat the section. Say what the reader must see in it and why it matters.
2. Take facts only from the props. Do not add a fact that the props do not support.
3. Write short sentences, active voice and simple tenses.
4. Write code names as the page writes them, for example `wsd`. Do not spell them for a voice.
5. Do not use backticks, markup or lists in the text.
6. Use 10 to 90 words for each step and 80 to 450 words in total. Keep most steps under
   60 words, so that the card stays short.

### 5. Check the props and the narration

```bash
node <skill>/scripts/render.mjs --props <output-dir>/props.json --narration <output-dir>/narration.json --check
```

- Exit 0: the props and the narration are valid.
- Exit 1: each stderr line is `<field path>: <error>`. A narration path starts with
  `narration`. Fix each field, then run the check again.
- Exit 2: a usage error or an I/O error.

Without `--narration`, the command checks the props only. `--ids` prints the node ids of the
concept graph.

### 6. Render the page and the screenshots

```bash
node <skill>/scripts/render.mjs --props <output-dir>/props.json --narration <output-dir>/narration.json --out <output-dir>
node <skill>/scripts/shoot.mjs --page <output-dir>/index.html --out <output-dir>/shots
```

- `render.mjs` prints the absolute paths of `index.html` and `graph.html`. It writes nothing for
  invalid props or an invalid narration. Without `--narration`, the page has no walkthrough.
- `shoot.mjs` writes `page.png` and one PNG for each section.
  - Plan sections: `header`, `tldr`, `done-when`, `phases`, `architecture`, `decisions`,
    `rabbit-holes`, `media`.
  - Ship sections: `header`, `tldr`, `done-when`, `changes`, `change-<i>`, `sequence`,
    `findings`, `hand-test`, `rollback`, `media`.
- `shoot.mjs` prints one absolute path for each file. `--theme dark` and `--width <px>` change
  the capture. Exit 0: done. Exit 2: usage. Exit 3: no browser opened.
- `shoot.mjs` uses `--chrome <bin>`, else `$VE_CHROME`, else Google Chrome. The screenshots
  do not show the walkthrough button. Without a browser, the page still works. Report the
  missing screenshots.

### 7. Publish

- When the session has the Artifact tool, publish `index.html` as a private artifact. Pass the
  Artifact `files` map `{"graph.html": "<output-dir>/graph.html"}`. When `run.html` is in the
  output directory, also put `"run.html": "<output-dir>/run.html"` in the map. Write the artifact
  URL to `<output-dir>/link.txt`.
- When the session has no Artifact tool, write the absolute path of `index.html` to
  `<output-dir>/link.txt`.
- A later run with the same output directory publishes to the same artifact. Pass the URL
  from `link.txt` when the session did not publish it.
- The URL fragment `#walkthrough` starts the walkthrough at step 1. `#walkthrough-<n>` starts it
  at step n.

A headless runner has no Artifact tool. It writes and checks the props and the narration
only. Its caller renders the page and the screenshots, and publishes them.

### 8. GitHub media (ship mode, when the caller asks)

GitHub cannot show the artifact page. Upload the screenshots to the orphan branch
`explainer-media` of the repository, and get a Markdown block:

```bash
node <skill>/scripts/gh-media.mjs --repo <owner/repo> --key <key> --dir <output-dir> [--url <artifact url>] [--dry-run]
```

- `<key>` names the folder on the branch, for example `pr-27` or `m4-ship`.
- The block starts with the marker `<!-- visual-explainer:<key> -->`. It shows the header and
  summary box screenshots, and the other sections in a collapsed list. With `--url`, the header
  screenshot links to the page.
- `--dry-run` uploads nothing and prints the same block. Use it for a preview.
- Exit 0: the block is on stdout. Exit 2: usage or I/O error. Exit 3: the gh or API call failed.
- The caller puts the block in the review body or in a PR comment.

### 9. Report

Report one line: the mode, then the artifact URL or the absolute path of `index.html`.
Add `walkthrough: <n> steps`, or `walkthrough: none` when the page has no narration. Add the
GitHub media result when step 8 ran. When a step fails, report the failure and the last stderr
lines. A failed explainer never blocks the work of the caller.

## Explore mode

Use explore mode to draw one graph that is not a plan or a ship report. Examples: how parts of a
system talk, a request flow, a data model, dependencies, what a change touched. You write one JSON
file. `explore.mjs` checks it and writes one self-contained page, `explore.html`. Do not write HTML,
CSS or JS. Do not write coordinates. The page lays the graph out with ELK.

```bash
node <skill>/scripts/explore.mjs --graph <output-dir>/graph.json --check
node <skill>/scripts/explore.mjs --graph <output-dir>/graph.json --out <output-dir>
```

- Exit 0: the graph is valid. Each stderr line `warning: ...` names one item that the page does not draw.
  Read the warnings and fix the JSON.
- Exit 1: the graph cannot be drawn. The stderr line says why.
- Exit 2: a usage error or an I/O error.
- `--out` prints the absolute path of `explore.html`. Publish it as an Artifact with the `files`
  map, or give the path to the person.
- `--timeline <file>` adds a time bar to the page. The format is in `scripts/timeline.mjs`. Each
  stderr line `warning: timeline: ...` names one item that the bar does not use. Without
  `--timeline`, the page has no time bar.

### The format

```json
{
  "title": "How login works",
  "summary": "One short line under the title",
  "direction": "right",
  "nodes": [
    { "id": "web", "type": "client", "label": "Web app", "sublabel": "React" },
    { "id": "backend", "type": "group", "label": "Backend" },
    { "id": "api", "type": "service", "label": "Auth API", "group": "backend", "status": "done",
      "details": "Markdown for the details panel", "links": [{ "label": "server.ts", "href": "src/server.ts" }] },
    { "id": "db", "type": "database", "label": "users", "group": "backend",
      "fields": [{ "name": "id", "type": "uuid", "key": "pk" }] }
  ],
  "edges": [
    { "from": "web", "to": "api", "label": "POST /login", "kind": "call" },
    { "from": "api", "to": "db", "kind": "data" }
  ],
  "steps": [
    { "title": "The app asks", "text": "One or two sentences.", "focus": ["web", "api"] }
  ]
}
```

| Field | Rule |
|-------|------|
| `direction` | `right` (default), `down`, `left` or `up`. The page turns the graph when the other way reads bigger in the window. |
| `nodes` | 1 to 150. Each has an `id` (letters, digits and `. : / - _`) and a `label` of 80 characters at most. |
| `type` | `client`, `service`, `database`, `queue`, `storage`, `external`, `actor`, `person`, `function`, `file`, `decision`, `note`, `task`, `milestone`, `metric` or `group`. The page maps a near miss such as `db` or `api` to the closest type. Any other word becomes `service`. |
| `sublabel`, `tag` | A line under the label (120 characters), a small badge (24 characters). |
| `status` | `todo`, `active`, `review`, `done`, `blocked`, `failed` or `idle`. The node shows a colored dot. |
| `group` | The id of a `group` node. The group draws a box around its nodes. |
| `details` | Markdown for the details panel (4000 characters): paragraphs, `- ` lists, `code` and `**bold**`. The page shows raw HTML as text. |
| `links` | Up to 8 `{label, href}`. The page keeps an `http`, `https` or path `href` and drops other schemes. |
| `fields` | The columns of a `database` node: `{name, type?, key?: "pk" or "fk", note?}`. |
| `value` | The number of a `metric` node, for example `"87%"`. |
| `emphasis` | `true` puts an accent ring on the node or the line. Use it on one or two items. |
| `edges` | Up to 300. `from` and `to` are node ids. `label` is 60 characters at most. |
| edge `kind` | `flow` (default), `call`, `data` (a dot moves along it), `async` (dashed and moving), `depends` (dotted) or `blocks` (red). |
| `steps` | Up to 30 walkthrough stops. `focus` lists node ids and line ids. A line without an `id` is `<from>-><to>`. |

### How to write a good graph

- Draw one idea in each graph. Two small graphs are better than one crowded graph.
- Use 4 to 25 nodes. Use a label of 2 to 4 words. Put the rest in `sublabel` or `details`.
- Use real names from the code. Use a `group` for each boundary, for example a service or a layer.
- Write an edge `label` that says what moves, for example `POST /login`. Pick the `kind` that fits.
- Write 3 to 7 steps when the graph tells a story. Write a title without a number. The page
  numbers the stops. A step lights up the nodes and the lines in `focus`, and the lines between
  them.
- Do not repeat the `details` in the chat. Keep the text around the graph short.

### What the reader can do

- Point at a node: it and everything it touches light up. The rest fades.
- Click a node: the details panel opens with the notes, the fields, the links and the
  connections. A click on a connection goes to that node.
- Drag a node: its lines follow and its group resizes. Drag the background to pan. Scroll with two
  fingers to pan. Pinch or press Ctrl and scroll to zoom. The buttons zoom and fit.
- Press `Walk me through it`: the page frames each step. Use the arrow keys to move and Esc to leave.
- Press the layout button to turn the graph. Use the minimap to jump.
- Without JavaScript, the page shows a list of the nodes and where each one leads.

The page follows the Artifact page contract of the report (see `## The page`). It has one
exception: it inlines elkjs (about 1.6 MB) as its only script.

## Run graph

The run graph shows one Loop milestone as it ran. Its base is the plan of the milestone: one box
for each lane, one node for each work phase, the dependency lines and the gate node at the end.
Each phase node shows its attempts, its setbacks, its merge commit and its converge rows. The gate
node shows the review rounds, the verdict, the converge result, the deviations, the hand test,
the rollback and the links to the PR, the ship page and each checkpoint page.

The time bar has a slider, a play button and one mark for each event: the start, each progress
checkpoint, the review and the ship. At a time, each phase shows its status: `todo` is faded,
`active` has a blue ring, `failed` is red and `done` is green. A checkpoint mark links to its
page. Each walkthrough step moves the slider to its time.

```bash
node <skill>/scripts/run-graph.mjs --loop <.loop dir> --milestone <n> [--notes <file>] [--ws-log <file>] --out <dir>
```

- It writes `run.html`, `run.graph.json` and `run.timeline.json` in `<dir>`. It prints the
  absolute path of `run.html`. It writes nothing in the Loop dir.
- The phases come from `milestones/m<n>.md` when the milestone is archived. Else they come from
  `plan.md`.
- A skills loop: the facts come from the saved output of `ws log <loop> --type loop.`. The default
  file is `explainers/m<n>-ship/loop-log.json`. Use `--ws-log` for another file.
- An old-engine loop: the facts come from `events.jsonl`, the retro facts, `state.json`, the
  review ledger, `converge/p<gate>.md` and the git commit times of the merges.
- Both engines: the checkpoints come from `explainers/m<n>-cp<k>/`. The verdict, the hand test
  and the rollback come from `explainers/m<n>-ship/props.json`.
- When no fact has a time, the page has no time bar. It still shows the graph with the outcomes.
  A stderr line `warning: no timestamps ...` tells this.
- `--notes` adds text that the facts do not have: `{title, summary, story, lanes: {<lane>:
  {what, why}}, joins: {<phase>: text}, findings: {<phase>: text}, gate: text}`. Each key is
  optional. A finding puts a ring on its phase and adds a walkthrough step.
- Exit 0: the page is written. Exit 1: the milestone has no phases, or the plan cannot be
  parsed. Exit 2: a usage error or an I/O error.

## The page

The report page is one centered column. It reads from top to bottom:

- `plan`: the header and the summary box. Then 1 Done when, 2 Phases, 3 Architecture
  before/after, the screenshots, 4 Decisions, 5 Rabbit holes and no-gos, and Full detail.
- `ship`: the header with the verdict pill or the checkpoint progress, and the summary box.
  Then 1 Done when planned vs delivered, 2 Changes in reading order and the screenshots.
  Then 3 Findings by severity, 4 Hand test, 5 Rollback, and Full detail.
- A Done when table with more than 8 rows shows the rows that are not met first and folds the
  other met rows. A hand test with more than 8 steps shows each step in a compact form.
- The verdict pill is red when a review has an open blocker or `REQUEST_CHANGES`. Else it is
  green with the most severe finding class that is not fixed.
- Full detail is collapsed. It holds every remaining fact and a link to `graph.html`.
- With a narration, a "Start the walkthrough" button follows the summary box. The walkthrough
  dims the page, puts a spotlight on one section, scrolls to it and shows the commentary in a
  card. Keys: right arrow for Next, left arrow for Back, Escape to close. The card stays in the
  viewport, also for a section that is taller than the screen.

The concept graph page shows the same facts as nodes and edges. The reader pans, zooms,
searches, filters and opens a node for its facts. It links back to the report.

The templates meet the Claude artifact page contract:

- The `<title>` is `meta.title`, 2 to 4 words.
- Color tokens are on `:root`. Dark tokens are under `prefers-color-scheme: dark` and
  `:root[data-theme="dark"]`. `body` has an explicit background.
- There are no external scripts, styles or assets. Media images are inlined.
- The layout fits a phone of 360 px width with no horizontal page scroll. Tables and diagrams
  scroll inside their own box.
- Motion runs only under `prefers-reduced-motion: no-preference`.
- The renderer escapes all props text. A link URL must start with `http://` or `https://`.

To change a template, the walkthrough or a schema, edit the skill files. Then run
`zsh tests/visual-explainer.test.sh` from the repository root.
