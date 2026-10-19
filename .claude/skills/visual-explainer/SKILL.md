---
name: visual-explainer
description: >-
  Render one self-contained HTML page from a props JSON file. The page is a concept graph that a
  person can explore. The reader pans, zooms, searches, filters, opens a node for all of its facts
  and follows the links between the nodes. Mode `plan` shows the goal, the milestones and phases,
  the architecture, the decisions and the risks. Mode `ship` shows the outcome, the changes, each
  Done when result, the deviations and the setbacks. It also shows the review and its findings, the
  hand-test steps with code references, and the rollback steps. A narration JSON file gives a
  tour of the graph. The page plays the tour as captions, and a Remotion video moves a camera
  over the same graph with a voiceover. The page is the primary output. A failed video never
  blocks the page. Use it after multiphase-plan registers a plan, at a Loop milestone merge
  gate, at a Loop milestone resume, and in pr-review Step 4. You write only the props JSON and
  the narration JSON. The skill owns the HTML, the CSS, the JS, the layout and the video.
argument-hint: "plan|ship <output-dir>"
---

# Visual Explainer

This skill turns facts into a concept graph. You write the facts as a props JSON file and the tour
as a narration JSON file. A zero-dependency Node script checks the props against a schema. Then it
builds the graph: one node for each item, the edges between the items, and one fixed layout. It
writes one `index.html` with the graph. A second script speaks the narration and renders the same
graph as `video.mp4` with Remotion. The video uses the positions of the page.

`<skill>` below is the directory of this file. In a self-hosted Loop run, use
`$RUNTIME_ROOT/.claude/skills/visual-explainer`.

## Files

| File | Purpose |
|------|---------|
| `template/page.html` | The page: color tokens, dark mode, motion rules, styles, the page script |
| `scripts/render.mjs` | Checks the props, then writes `index.html` (Node 20.11 or later) |
| `scripts/render-video.mjs` | Checks the props and the narration, then renders `video.mp4` in a cache directory |
| `scripts/graph.mjs` | Props to nodes, edges, groups and the fixed layout. It also checks the relation ids |
| `scripts/page.mjs` | The graph SVG, the outline and the page data |
| `scripts/props.mjs` | The props check that both scripts use |
| `scripts/narration.mjs` | The narration check and the tour stops |
| `scripts/tts.mjs` | Text to speech with ElevenLabs or the macOS `say` command, with a clip cache |
| `scripts/validate.mjs` | The schema checker |
| `schema/plan.schema.json` | The props of `plan` mode |
| `schema/ship.schema.json` | The props of `ship` mode |
| `schema/narration.schema.json` | The narration, version 2: the stops of the tour |
| `schema/narration-v1.schema.json` | The legacy narration, version 1: one text for each section |
| `examples/*.props.json` | One valid example for each mode, with relation fields |
| `examples/*.narration.json` | The version 2 narration of each example |
| `video/package.json`, `video/package-lock.json` | The Remotion project and its pinned versions |
| `video/engine.mjs` | The Remotion bundle and render steps. It runs only in the cache directory |
| `video/src/storyboard.mjs` | The video stops: camera, card, timing |
| `video/src/*.jsx` | The Remotion compositions `plan` and `ship`: the graph, the card, the header |

## Rules

1. Write only the props JSON file and the narration JSON file. Do not write HTML, CSS or JS.
2. Do not edit the template, the scripts or the schemas during a run.
3. Write facts only. Take each fact from the plan, the ledger, a review report, the PR or
   the code. Do not invent a count, a result, a finding or a relation.
4. Omit an optional field when you do not know its value.
5. Write plain text. A pair of backticks marks inline code. A newline is a line break.
6. Do not put secrets, tokens or credentials in the props or in the narration.
7. `meta.title` has 2 to 4 words. It is the page title.
8. Do not commit the output directory to the reviewed branch.

## Workflow

### 1. Choose the mode and the output directory

The caller names the mode and the output directory. Write the props to
`<output-dir>/props.json` and the narration to `<output-dir>/narration.json`.

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
  `missed`. Take the evidence from the progress log, the ledger and the review. Give each row
  its `phase` number. The relation fields find a phase through this number.
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
- `reviews`: one entry for each repository or PR. `verdict` is the posted GitHub event.
  Copy the findings of the final review. Set each finding to `open`, `fixed` or `deferred`.
- `handTest`: the ordered steps for a human. Each step has `command`, `ref` and `why`.
- `rollback`: the ordered steps that undo the change.

**Relation fields (optional, both modes):** a relation field adds an edge to the graph. Add a
relation only when a source states it. These edges need no field:

- the goal to each milestone, and a milestone to each phase
- the `dependsOn` order of the phases
- a layer to each component, and each component flow
- a review to each finding, and a hand-test step or a rollback step to the next step

The relation fields add these edges:

| Field | Values | Edge |
|-------|--------|------|
| `milestones[].phases[].components` | component names | the phase touches the component |
| `decisions[].phases`, `decisions[].components` | phase numbers, component names | the decision affects the item |
| `risks[].mitigatedBy` | phase numbers | the phase mitigates the risk |
| `risks[].components` | component names | the risk affects the component |
| `changes[].phases` | phase numbers | the phase delivers the change |
| `deviations[].phases` | phase numbers | the deviation concerns the phase |
| `reviews[].findings[].phases`, `reviews[].findings[].changes` | phase numbers, change areas | the finding is about the item |
| `handTest[].phases` | phase numbers | the step checks the phase |

- A phase number in `plan` mode must be the number of a phase in `milestones`.
- A phase number in `ship` mode must be the `phase` of a `delivered` row.
- A component name must be the exact name of one component. A change area must be the exact
  `area` of one change.
- The check rejects an unknown id with its field path, for example
  `$.risks[0].mitigatedBy[1]: no phase 42 in the milestones`.
- The check also rejects a phase number or a milestone number that appears more than once.

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

The narration is the tour of the graph. The video moves its camera to each stop and speaks the
text. The page tour shows the same text as a caption. Read `schema/narration.schema.json` and the
example narration of the mode. The file has this shape:

```json
{ "version": 2, "mode": "<mode>", "stops": [{ "focus": ["<id>"], "title": "<optional title>", "text": "<spoken text>" }] }
```

Each stop has these fields:

- `focus`: 1 to 12 ids. An id is a node id, a group id or `all`. `all` shows the whole graph.
- `title`: optional, at most 80 characters. Without it, the stop takes the title of its focus.
- `text`: the spoken text of the stop, at most 1500 characters.

Get the ids from the props:

```bash
node <skill>/scripts/render.mjs --props <output-dir>/props.json --ids
```

Each output line is `<id>`, a tab, the kind, a tab and the title. The node ids follow these rules:

- `plan`: `goal`, `milestone-<n>`, `phase-<n>`, `layer-<i>`, `component-<slug>`, `decision-<i>`,
  `risk-<i>`.
- `ship`: `summary`, `process`, `change-<i>`, `phase-<n>`, `delivered-<i>`, `deviation-<i>`,
  `setback-<i>`, `review-<i>`, `finding-<slug>`, `step-<i>`, `rollback-<i>`.
- `<n>` is the number in the props. `<i>` is the position in its list, from 1. `<slug>` is the
  name in lowercase, with `-` for each run of other characters. In `ship` mode, `phase-<n>` is
  the node of the `delivered` rows of phase n.
- The group ids are `group-<name>`, for example `group-timeline`, `group-architecture`,
  `group-delivered` and `group-hand-test`.

Plan the tour by these rules:

1. Start with `all`. End with `all`. The first stop shows the title card. The last stop shows
   the end card.
2. Give each important item its own stop. Examples: the goal, a large phase, the biggest risk
   and the phase that mitigates it, a partial Done when, a major finding.
3. Use a group id for a stop about a whole cluster.
4. Keep the stops in an order that tells the story: the goal, the work, the decisions, the
   risks. In `ship` mode: the outcome, the delivered work, the deviations, the setbacks, the
   review, the hand test, the rollback.

Write the text by these rules:

1. Explain. Do not read the node cards aloud.
2. For each stop, say what it shows, why it matters and what to look at.
3. Take facts only from the sources of the props. Do not add a fact that the props do
   not support.
4. Write spoken English: short sentences, active voice and simple tenses.
5. Write each code name in a form that a voice can speak. For example, write "W S D" for
   `wsd` and "Z shell" for `zsh`. Do not spell out paths, hashes or long commands. Name
   them in words.
6. Do not use backticks, markup or lists in the text.
7. Use 20 to 120 words for each stop and 300 to 900 words in total. The check rejects a
   narration above 9000 characters in total.

A version 1 narration (`{ "mode", "sections": [{ "id", "text" }] }`) is still valid. Each
section becomes one stop on its cluster of nodes. Write new narrations as version 2.

### 5. Check the props and the narration

```bash
node <skill>/scripts/render-video.mjs --props <output-dir>/props.json --narration <output-dir>/narration.json --check
```

- Exit 0: the props and the narration are valid.
- Exit 1: each stderr line is `<field path>: <error>`. A narration path starts with
  `narration`. Fix each field, then run the check again.
- Exit 2: a usage error or an I/O error.

`node <skill>/scripts/render.mjs --props <output-dir>/props.json --check` checks the
props only.

### 6. Render

```bash
node <skill>/scripts/render.mjs --props <output-dir>/props.json --out <output-dir>
```

The script prints the absolute path of `<output-dir>/index.html`. It writes nothing for
invalid props.

The page tour uses `narration.json` in the directory of the props file. `--narration <file>`
names a different file. When `narration.json` beside the props is not valid, the script writes
one warning line to stderr and the page gets the default tour. When the `--narration` file is
not valid, the script exits 1.

### 7. Video

The video is an addition to the page. Do these steps in this order:

1. Render the page and check it first (steps 5 and 6).
2. Render the video. The timeout applies to the whole command:

   ```bash
   node <skill>/scripts/render-video.mjs --props <output-dir>/props.json --narration <output-dir>/narration.json --out <output-dir>
   ```

   The defaults are `--timeout-min 15`, `--max-mb 14` (1 MB is 1,000,000 bytes) and
   `--tts auto`.
3. When the command exits 0, render the page again with the video drawer:

   ```bash
   node <skill>/scripts/render.mjs --props <output-dir>/props.json --out <output-dir> --video video.mp4
   ```

4. When the command exits with a different code, keep the page without the video. Write
   the reason and the last stderr lines to `<output-dir>/video-error.txt`. Report the
   failure in step 9.

Exit codes of `render-video.mjs`:

- Exit 0: stdout has two lines. Line 1 is the absolute path of `<output-dir>/video.mp4`.
  Line 2 names the voice engine: `tts: elevenlabs`, `tts: say`, or
  `tts: say (fallback: <reason>)`.
- Exit 1: the props or the narration are not valid. Each stderr line is
  `<field path>: <error>`.
- Exit 2: a usage error or an I/O error.
- Exit 3: the text to speech or the render failed or timed out. The command writes no
  `video.mp4`.

Text to speech:

- `--tts auto` uses ElevenLabs when the environment has `ELEVENLABS_API_KEY`. Else it uses
  the macOS `say` command, which works offline.
- In `auto` mode, when ElevenLabs fails for one clip, the script speaks the whole narration
  again with `say`. A video has one voice only. These failures cause the fallback: an HTTP
  error after the one retry, a quota error, a network error, and a request above 90 s.
  Examples of HTTP errors are 401, 402, 429 and 5xx. The script writes one stderr line, for
  example
  `render-video: ElevenLabs failed (HTTP 401 quota_exceeded): the video uses the macOS voice`.
  Line 2 of stdout is then `tts: say (fallback: HTTP 401 quota_exceeded)`.
- When the `--timeout-min` deadline stops the text to speech, the script does not use
  `say`. It exits 3.
- `--tts elevenlabs` and `--tts say` select one engine. `--tts elevenlabs` never uses
  `say`: a failed ElevenLabs call exits 3, and the command writes no `video.mp4`.
- The default ElevenLabs model is `eleven_flash_v2_5`. It uses 0.5 credits for each
  character. `eleven_multilingual_v2` uses 1 credit for each character.
- `VE_TTS_VOICE` sets the voice. `VE_TTS_MODEL` sets the ElevenLabs model. In `auto` mode
  with the key, `VE_TTS_VOICE` is the ElevenLabs voice, so the `say` fallback uses the best
  installed English voice.
- The script makes one clip for each stop that has text. It caches each clip by the engine,
  the voice, the model and the text. A re-render of the same narration sends no text again.
  The log line `tts:` gives the number of characters sent.

The first run for each version of `video/` runs `npm ci` and downloads a headless browser.
It needs network access and takes up to about one minute more. On an Apple M4 laptop, a
later render takes about one half of the video length, plus the text to speech for new
text.

### 8. Publish

- When the session has the Artifact tool, publish `index.html` as a private artifact.
  When the video step succeeded, also pass the Artifact `files` map
  `{"video.mp4": "<output-dir>/video.mp4"}`. Write the artifact URL to
  `<output-dir>/link.txt`.
- When the session has no Artifact tool, write the absolute path of `index.html` to
  `<output-dir>/link.txt`.
- A later run with the same output directory publishes to the same artifact. Pass the URL
  from `link.txt` when the session did not publish it.

A headless runner has no Artifact tool. It writes and checks the props and the narration
only. Its caller renders the page and the video, and publishes them.

### 9. Report

Report one line: the mode, then the artifact URL or the absolute path of `index.html`.
Add the video result: `video: embedded`, or `video: failed` with the reason. When line 2 of
the `render-video.mjs` stdout starts with `tts: say`, write `video: embedded (macOS voice)`. When a step
fails, report the failure and the last stderr lines. A failed explainer never blocks the
work of the caller. A failed video never blocks the page.

## The graph

`scripts/graph.mjs` builds the graph from the props. The same props always give the same node
ids, the same edges and the same positions.

- Each node has an id, a kind, a title, a short summary and all the facts of its item.
- The root node (`goal` or `summary`) sits in the middle. Each group of nodes is a framed block
  around it: for example the timeline, the architecture, the decisions and the risks.
- Inside a block, the nodes are in a grid in reading order. In the timeline, each milestone
  heads a block of its phases in plan order. In the architecture, each layer heads a row of
  its components. The grid keeps a plan with 40 or more phases readable.
- The script places the blocks one by one, the largest first. Each block takes the free
  position that keeps the whole drawing small in a 16:9 frame. The position is also near the
  root and near the blocks that share edges with it. No two node boxes overlap.
- A legacy reference to an item that is not in the props, for example a `dependsOn` phase of
  an earlier milestone, makes no edge. The fact stays on the node.

## The page

The page shows the graph in the full window.

- Drag to pan. Use the wheel or two fingers to zoom.
- On the keyboard, Tab moves between the nodes and Enter opens a node. The arrow keys pan, `+`
  and `-` zoom, and `0` shows the whole graph.
- A click or Enter on a node opens the detail panel. The panel shows all the facts of the node
  and its connections as links.
- The URL hash `#node=<id>` opens a node. The browser back and forward buttons move between the
  opened nodes. A link with a hash opens that node.
- The search field finds nodes by their title, their summary or their id.
- The kind chips at the top are the legend. A chip shows or hides its kind.
- The Outline view lists every node, grouped by kind, with all its facts. It is the default view
  on a phone. Without the script, the page shows the static graph and the outline.
- Play tour steps through the stops of the narration at the pace of the reader, with the
  narration text as captions.
- With `--video <path>`, the bar has a Video button that opens a drawer with
  `<video controls preload="metadata" playsinline>`. The path must be relative, end in `.mp4` and
  have no `..` segment. Without `--video`, the page has no video.

The template meets the Claude artifact page contract:

- The `<title>` is `meta.title`, 2 to 4 words.
- Color tokens are on `:root`. Dark tokens are under `prefers-color-scheme: dark` and
  `:root[data-theme="dark"]`. `body` has an explicit background.
- There are no external scripts, styles or assets.
- The layout fits a phone of 360 px width with no horizontal page scroll.
- Motion runs only under `prefers-reduced-motion: no-preference`. With reduced motion, the
  camera jumps.
- The renderer escapes all props text. A link URL must start with `http://` or `https://`.

## The video

`video/` is a Remotion project with two compositions, `plan` and `ship`. Each composition takes
the graph, the tour stops and the voiceover clips as its input props. The video is 1920x1080 at
24 fps, with the voiceover as its audio. It has no captions.

- The video draws the graph of the page with the same positions.
- A camera moves from stop to stop. During the first 0.9 s of a stop, the camera eases from the
  previous stop. The focus nodes and their edges stay bright, their neighbours dim a little, and
  the other nodes dim.
- A card on the right shows the key facts of one focus node, or a list of the focus nodes. The
  first stop shows the title card and the last stop shows the end card.
- When the narration does not start with `all`, the video adds a title stop of 5 s. When it does
  not end with `all`, the video adds an end stop of 4 s.
- The colors are the dark color tokens of `template/page.html`. A test compares them.
- The narration sets the length. Each stop has a minimum visual time: reading time for its
  card, from 5 to 9 s. A stop lasts the longer of its minimum visual time and its audio time
  plus 0.6 s. The audio starts 0.3 s into the stop.

`render-video.mjs` never writes into the skill directory. The skill directory can be a
symlinked git checkout or a Loop integration worktree. The script copies `video/` to
`${XDG_CACHE_HOME:-$HOME/.cache}/visual-explainer/video-<key>`. The key is 12 hex
characters of a SHA-256 over the lock file and the video sources. The script runs
`npm ci`, the browser download and the bundle there, one time for each key. The voiceover
clips go to `${XDG_CACHE_HOME:-$HOME/.cache}/visual-explainer/tts/`. The script writes the
MP4 to a temporary file in the output directory, then renames it to `video.mp4`. Each run
removes the caches of other keys that no run used for 7 days.

The first render uses an x264 quality (CRF) that the script chooses from the video length
and `--max-mb`. A size model from measurements of the graph video predicts the file size.
The script uses the best CRF from 23 to 30 whose predicted file fits 90% of `--max-mb`. With
`--max-mb 14`, a video of up to about 3 minutes gets CRF 23. A video of 5 minutes gets CRF 29.
Measurements: the 166 s plan example had 7.9 MB at CRF 23. The 232 s ship example had 14.4 MB
at CRF 23 and 8.1 MB at CRF 30. The camera moves take most of the bytes, because each move
changes the whole frame. When the file is larger than `--max-mb`, the script renders again at
CRF 30, then at 1280x720.

Encoder settings: the defaults are the fastest combination of a measurement on an Apple M4
with 10 cores. The text stays sharp. The browser frame capture takes most of the time, so
fewer frames and GPU compositing help most. A faster x264 preset and more processes help a
little. Hardware encoding (VideoToolbox) was slower, and its file was larger than its
target. An environment variable overrides each default:

| Variable | Default | Values |
|----------|---------|--------|
| `VE_VIDEO_FPS` | `24` | an integer from 1 to 60 |
| `VE_VIDEO_CONCURRENCY` | the number of cores minus 2 | an integer from 1 to 64 |
| `VE_VIDEO_X264_PRESET` | `veryfast` | an x264 preset, from `ultrafast` to `veryslow` |
| `VE_VIDEO_HWACCEL` | `disable` | `disable`, `if-possible`, `required` |
| `VE_VIDEO_GL` | `angle` | `default` (Chrome chooses), `angle`, `egl`, `swiftshader`, `swangle`, `vulkan`, `angle-egl` |

With hardware encoding, the script sets a video bitrate that fits `--max-mb` for the
duration of the video. A value that is not valid is a usage error (exit 2).

The ElevenLabs key stays in the environment. The script never writes it to a log, an error
message, a file name, a file or the props. The npm, Remotion and browser processes do not
get it.

License: Remotion is free for an individual and for a company of up to 3 people. This
repository is a personal project, so the free license applies. A company with more people
needs a Remotion company license. The renderer passes no `licenseKey`, so Remotion sends no
usage data.

To change the template, the graph, the video or a schema, edit the skill files. Then run
`zsh tests/visual-explainer.test.sh` from the repository root. Set `VE_VIDEO_E2E=1` to
also render the two examples as real narrated videos. That run uses the real text to
speech engine.
