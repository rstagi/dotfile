---
name: visual-explainer
description: >-
  Render a plan or a PR review as one concise report page. Also render screenshots of its
  sections and a short narrated video. You write only a props JSON file and a narration JSON file. Mode `plan`
  shows the summary box with the requests to the reader, the Done when table and the phases. It
  also shows one before/after architecture diagram, the decision cards and the rabbit holes. Mode `ship`
  shows the verdict, the summary box, the Done when results and the changes in reading order with
  one sequence diagram. It also shows the findings by severity, the hand test and the rollback. With
  `meta.checkpoint`, `ship` mode shows a progress checkpoint of a running milestone. The video
  uses snapcn components on Remotion and a voiceover. A concept graph of the same props is a
  second page. Use it in multiphase-plan step 5 and in pr-review Step 4. Also use it at Loop
  progress checkpoints, at a Loop milestone merge gate and at a Loop milestone resume. A failed video
  never blocks the page.
argument-hint: "plan|ship <output-dir>"
---

# Visual Explainer

This skill turns facts into a report that a person reads from top to bottom in a few minutes.
You write the facts as a props JSON file and the voiceover as a narration JSON file. Zero-dependency
Node scripts check the props and write `index.html` (the report) and `graph.html` (a concept graph
of the same facts). A second script takes screenshots of the report sections. A third script
speaks the narration and renders `video.mp4` and `preview.gif` with Remotion and snapcn
components. A fourth script puts the screenshots and the video on GitHub for a PR.

`<skill>` below is the directory of this file. In a self-hosted Loop run, use
`$RUNTIME_ROOT/.claude/skills/visual-explainer`.

## Files

| File | Purpose |
|------|---------|
| `template/report.html` | The report page: color tokens, dark mode, styles |
| `template/page.html` | The concept graph page |
| `scripts/render.mjs` | Checks the props, then writes `index.html` and `graph.html` (Node 20.11 or later) |
| `scripts/report.mjs`, `scripts/diagrams.mjs` | The report sections, the before/after diagram and the sequence diagram |
| `scripts/graph.mjs`, `scripts/page.mjs` | The concept graph: nodes, edges, layout and page data |
| `scripts/shoot.mjs` | Screenshots of the report sections with a headless Chrome |
| `scripts/render-video.mjs` | Checks the props and the narration, then renders `video.mp4` and `preview.gif` |
| `scripts/narration.mjs` | The narration check |
| `scripts/tts.mjs` | Text to speech with ElevenLabs or the macOS `say` command, with a clip cache |
| `scripts/gh-media.mjs` | Uploads the screenshots and the video to GitHub, prints a Markdown block |
| `scripts/props.mjs`, `scripts/validate.mjs` | The props check and the schema checker |
| `schema/plan.schema.json`, `schema/ship.schema.json` | The props of each mode |
| `schema/narration.schema.json` | The narration, version 3: one text for each scene |
| `examples/*.props.json`, `examples/*.narration.json` | One valid example for each mode |
| `video/` | The Remotion project: `src/scenes.jsx` and `src/parts.jsx`, the vendored snapcn components in `src/components/snap-cn/` (MIT, see `NOTICE`) |

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

The narration is the voiceover of the video. The video has one scene for each narration entry.
Each scene shows its section of the page. Read `schema/narration.schema.json` and the example
narration of the mode. The file has this shape:

```json
{ "version": 3, "mode": "<mode>", "scenes": [{ "scene": "<id>", "text": "<spoken text>" }] }
```

The scene ids, in their fixed order:

- `plan`: `intro`, `tldr`, `done-when`, `phases`, `architecture`, `decisions`, `rabbit-holes`,
  `outro`.
- `ship`: `intro`, `tldr`, `done-when`, `changes`, `findings`, `hand-test`, `rollback`, `outro`.

Plan the scenes by these rules:

1. Start with `intro`. Keep the order above. Use each id one time at most.
2. Omit a scene to make the video shorter. The video has no scene that the narration omits.
3. Do not add a scene whose section has no data, for example `findings` in a checkpoint. The
   check rejects it.

Write the text by these rules:

1. Explain. Do not read the page aloud. Say what the section shows and why it matters.
2. Take facts only from the props. Do not add a fact that the props do not support.
3. Write spoken English: short sentences, active voice and simple tenses.
4. Write each code name in a form that a voice can speak. For example, write "W S D" for
   `wsd` and "Z shell" for `zsh`. Do not spell out paths, hashes or long commands.
5. Do not use backticks, markup or lists in the text.
6. Use 10 to 90 words for each scene and 80 to 450 words in total. The video then has 45 to
   180 seconds.

### 5. Check the props and the narration

```bash
node <skill>/scripts/render-video.mjs --props <output-dir>/props.json --narration <output-dir>/narration.json --check
```

- Exit 0: the props and the narration are valid.
- Exit 1: each stderr line is `<field path>: <error>`. A narration path starts with
  `narration`. Fix each field, then run the check again.
- Exit 2: a usage error or an I/O error.

`node <skill>/scripts/render.mjs --props <output-dir>/props.json --check` checks the props
only. `--ids` prints the node ids of the concept graph.

### 6. Render the page and the screenshots

```bash
node <skill>/scripts/render.mjs --props <output-dir>/props.json --out <output-dir>
node <skill>/scripts/shoot.mjs --page <output-dir>/index.html --out <output-dir>/shots
```

- `render.mjs` prints the absolute paths of `index.html` and `graph.html`. It writes nothing for
  invalid props.
- `shoot.mjs` writes `page.png` and one PNG for each section.
  - Plan sections: `header`, `tldr`, `done-when`, `phases`, `architecture`, `decisions`,
    `rabbit-holes`, `media`.
  - Ship sections: `header`, `tldr`, `done-when`, `changes`, `change-<i>`, `sequence`,
    `findings`, `hand-test`, `rollback`, `media`.
- `shoot.mjs` prints one absolute path for each file. `--theme dark` and `--width <px>` change
  the capture. Exit 0: done. Exit 2: usage. Exit 3: no browser opened.
- `shoot.mjs` uses `--chrome <bin>`, else `$VE_CHROME`, else the headless Chrome of the video
  cache, else Google Chrome. Without a browser, the page still works. Report the missing
  screenshots.

### 7. Video

The video is an addition to the page. Do these steps in this order:

1. Render the page first (step 6).
2. Render the video. The timeout applies to the whole command:

   ```bash
   node <skill>/scripts/render-video.mjs --props <output-dir>/props.json --narration <output-dir>/narration.json --out <output-dir>
   ```

   The defaults are `--timeout-min 15`, `--max-mb 14` (1 MB is 1,000,000 bytes) and
   `--tts auto`.
3. When the command exits 0, render the page again with the video player:

   ```bash
   node <skill>/scripts/render.mjs --props <output-dir>/props.json --out <output-dir> --video video.mp4
   ```

4. When the command exits with a different code, keep the page without the video. Write
   the reason and the last stderr lines to `<output-dir>/video-error.txt`. Report the
   failure in step 10.

Exit codes of `render-video.mjs`:

- Exit 0: stdout has two lines. Line 1 is the absolute path of `<output-dir>/video.mp4`.
  Line 2 names the voice engine: `tts: elevenlabs`, `tts: say`, or
  `tts: say (fallback: <reason>)`. The command also writes `preview.gif` when ffmpeg works.
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
- The script makes one clip for each scene. It caches each clip by the engine,
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

### 8. Publish

- When the session has the Artifact tool, publish `index.html` as a private artifact. Pass the
  Artifact `files` map `{"graph.html": "<output-dir>/graph.html"}`. When the video step
  succeeded, also add `"video.mp4": "<output-dir>/video.mp4"`. Write the artifact URL to
  `<output-dir>/link.txt`.
- When the session has no Artifact tool, write the absolute path of `index.html` to
  `<output-dir>/link.txt`.
- A later run with the same output directory publishes to the same artifact. Pass the URL
  from `link.txt` when the session did not publish it.

A headless runner has no Artifact tool. It writes and checks the props and the narration
only. Its caller renders the page, the screenshots and the video, and publishes them.

### 9. GitHub media (ship mode, when the caller asks)

GitHub cannot show the artifact page. Upload the screenshots, the preview and the video to the
orphan branch `explainer-media` of the repository, and get a Markdown block:

```bash
node <skill>/scripts/gh-media.mjs --repo <owner/repo> --key <key> --dir <output-dir> [--url <artifact url>] [--dry-run]
```

- `<key>` names the folder on the branch, for example `pr-27` or `m4-ship`.
- The block starts with the marker `<!-- visual-explainer:<key> -->`. It shows the header and
  summary box screenshots, the preview GIF with a link to the video, and the other sections in a
  collapsed list. GitHub does not play an MP4 from a branch inline, so the GIF is the preview.
- `--dry-run` uploads nothing and prints the same block. Use it for a preview.
- Exit 0: the block is on stdout. Exit 2: usage or I/O error. Exit 3: the gh or API call failed.
- The caller puts the block in the review body or in a PR comment.

### 10. Report

Report one line: the mode, then the artifact URL or the absolute path of `index.html`.
Add the video result: `video: embedded`, or `video: failed` with the reason. When line 2 of
the `render-video.mjs` stdout starts with `tts: say`, write `video: embedded (macOS voice)`.
Add the GitHub media result when step 9 ran. When a step fails, report the failure and the
last stderr lines. A failed explainer never blocks the work of the caller. A failed video never
blocks the page.

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

## The video

`video/` is a Remotion project with two compositions, `plan` and `ship`. The video is 1920x1080
at 30 fps, with the voiceover as its audio and burned-in captions.

- Each narration scene is one scene of the video. It shows the data of its page section with
  motion. Examples: the summary box lines, the Done when rows, the phase checklist, the change
  cards and the finding cards. The hand-test commands type in a terminal.
- The architecture diagram, the sequence diagram and the media come from dark screenshots of a
  temporary report page. Without screenshots, the scene draws a simpler native view.
- snapcn components (MIT): `text-reveal` for the titles, `text-highlight` for the summary box labels,
  `word-captions` for the captions, `terminal-simulator` for the hand test and `status-cycle`
  for the outro. Each local change is marked `visual-explainer patch` in the source and listed
  in `video/src/components/snap-cn/NOTICE`. The components use system fonts, so a render needs
  no network after the first `npm ci`.
- A scene lasts the longer of its minimum visual time and its audio time plus 0.6 s. The audio
  starts 0.3 s into the scene. A thin bar at the top shows the scene progress.
- `preview.gif` takes about 1.5 s from the middle of each scene, 720 px wide at 10 fps, and at
  most 4 MB. A failed GIF only warns.

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
`--max-mb 14`, a concise video of up to 3 minutes gets CRF 23.
Measurements at 30 fps and CRF 23, with the audio: the 107 s plan example has 5.7 MB. The
105 s ship example has 5.0 MB. When the file is larger than `--max-mb`, the script renders again at
CRF 30, then at 1280x720.

Encoder settings: the defaults are the fastest combination of a measurement on an Apple M4
with 10 cores. The text stays sharp. The browser frame capture takes most of the time, so
fewer frames and GPU compositing help most. A faster x264 preset and more processes help a
little. Hardware encoding (VideoToolbox) was slower, and its file was larger than its
target. An environment variable overrides each default:

| Variable | Default | Values |
|----------|---------|--------|
| `VE_VIDEO_FPS` | `30` | an integer from 1 to 60 |
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

To change a template, the video or a schema, edit the skill files. Then run
`zsh tests/visual-explainer.test.sh` from the repository root. Set `VE_VIDEO_E2E=1` to
also render the two examples as real narrated videos. That run uses the real text to
speech engine.
