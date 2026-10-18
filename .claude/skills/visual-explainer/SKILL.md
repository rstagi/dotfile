---
name: visual-explainer
description: >-
  Render one animated, self-contained HTML explainer page from a props JSON file.
  Mode `plan` shows the goal, the architecture, the milestone and phase timeline,
  the decisions with their rejected options, and the risks. Mode `ship` shows what
  each Done when delivered and the deviations from the plan. It also shows the review
  verdict and findings, a hand-test checklist with code references, and the rollback
  steps. Then it renders the same props as an MP4 video with Remotion. A narration JSON
  file gives the video its voiceover. The page shows the video at the top. The page is
  the primary output. A failed video never blocks the page. Use it after multiphase-plan
  registers a plan, at a Loop milestone merge gate, at a Loop milestone resume, and in
  pr-review Step 4. You write only the props JSON and the narration JSON. The skill owns
  the HTML, the CSS, the JS and the video.
argument-hint: "plan|ship <output-dir>"
---

# Visual Explainer

This skill turns facts into one fixed, animated HTML page and a narrated MP4 video. You
write the facts as a props JSON file and the spoken text as a narration JSON file. A
zero-dependency Node script checks the props against a schema. Then it escapes all text,
puts it into fixed scene components, and writes one `index.html`. A second script speaks
the narration and renders the same props as `video.mp4` with Remotion.

`<skill>` below is the directory of this file. In a self-hosted Loop run, use
`$RUNTIME_ROOT/.claude/skills/visual-explainer`.

## Files

| File | Purpose |
|------|---------|
| `template/page.html` | The page: color tokens, dark mode, motion rules, scene styles, a small script |
| `scripts/render.mjs` | Checks the props, then writes `index.html` (Node 20.11 or later) |
| `scripts/render-video.mjs` | Checks the props and the narration, then renders `video.mp4` in a cache directory |
| `scripts/props.mjs` | The props check that both scripts use |
| `scripts/narration.mjs` | The narration check |
| `scripts/tts.mjs` | Text to speech with ElevenLabs or the macOS `say` command, with a clip cache |
| `scripts/scenes.mjs` | The fixed scene components of the page |
| `scripts/validate.mjs` | The schema checker |
| `schema/plan.schema.json` | The props of `plan` mode |
| `schema/ship.schema.json` | The props of `ship` mode |
| `schema/narration.schema.json` | The narration of both modes |
| `examples/*.props.json` | One valid example for each mode |
| `examples/*.narration.json` | The narration of each example |
| `video/package.json`, `video/package-lock.json` | The Remotion project and its pinned versions |
| `video/engine.mjs` | The Remotion bundle and render steps. It runs only in the cache directory |
| `video/src/storyboard.mjs` | The video scenes: item limits, text cuts, sections and timing |
| `video/src/*.jsx` | The Remotion compositions `plan` and `ship` and their scenes |

## Rules

1. Write only the props JSON file and the narration JSON file. Do not write HTML, CSS or JS.
2. Do not edit the template, the scripts or the schemas during a run.
3. Write facts only. Take each fact from the plan, the ledger, a review report, the PR or
   the code. Do not invent a count, a result or a finding.
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

### 4. Write the narration

The narration is the voiceover of the video. It helps a person follow the explanation.
Read `schema/narration.schema.json` and the example narration of the mode. The file has
this shape:

```json
{ "mode": "<mode>", "sections": [{ "id": "<section id>", "text": "<spoken text>" }] }
```

The section ids, in video order:

- `plan`: `intro`, `goal`, `architecture`, `milestones`, `decisions`, `risks`, `outro`.
- `ship`: `intro`, `summary`, `changes`, `delivered`, `deviations`, `setbacks`,
  `reviews`, `handTest`, `rollback`, `outro`.

Write one entry for each section that the props show. `changes` needs `changes` items.
`setbacks` needs `setbacks` items or `process` counts. The check rejects an unknown id, a
repeated id, and an id whose props section is absent. A section without narration plays
for its minimum visual time, with no voice.

Write the text by these rules:

1. Explain. Do not read the slides aloud.
2. For each section, say what it shows, why it matters and what to look at.
3. Take facts only from the sources of the props. Do not add a fact that the props do
   not support.
4. Write spoken English: short sentences, active voice and simple tenses.
5. Write each code name in a form that a voice can speak. For example, write "W S D" for
   `wsd` and "Z shell" for `zsh`. Do not spell out paths, hashes or long commands. Name
   them in words.
6. Do not use backticks, markup or lists in the text.
7. Use 30 to 180 words for each section and 300 to 900 words in total. The check rejects
   a section above 1500 characters and a narration above 9000 characters in total.

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

### 7. Video

The video is an addition to the page. Do these steps in this order:

1. Render the page and check it first (steps 5 and 6).
2. Render the video. The timeout applies to the whole command:

   ```bash
   node <skill>/scripts/render-video.mjs --props <output-dir>/props.json --narration <output-dir>/narration.json --out <output-dir>
   ```

   The defaults are `--timeout-min 15`, `--max-mb 14` (1 MB is 1,000,000 bytes) and
   `--tts auto`.
3. When the command exits 0, render the page again with the video block:

   ```bash
   node <skill>/scripts/render.mjs --props <output-dir>/props.json --out <output-dir> --video video.mp4
   ```

4. When the command exits with a different code, keep the page without the video. Write
   the reason and the last stderr lines to `<output-dir>/video-error.txt`. Report the
   failure in step 9.

Exit codes of `render-video.mjs`:

- Exit 0: stdout is the absolute path of `<output-dir>/video.mp4`.
- Exit 1: the props or the narration are not valid. Each stderr line is
  `<field path>: <error>`.
- Exit 2: a usage error or an I/O error.
- Exit 3: the text to speech or the render failed or timed out. The command writes no
  `video.mp4`.

Text to speech:

- `--tts auto` uses ElevenLabs when the environment has `ELEVENLABS_API_KEY`. Else it uses
  the macOS `say` command, which works offline.
- `--tts elevenlabs` and `--tts say` select one engine.
- `VE_TTS_VOICE` sets the voice. `VE_TTS_MODEL` sets the ElevenLabs model.
- The script caches each clip by the engine, the voice, the model and the text. A
  re-render of the same narration sends no text again. The log line `tts:` gives the
  number of characters sent.

The first run for each version of `video/` runs `npm ci` and downloads a headless browser.
It needs network access and takes up to about one minute more. On an Apple M4 laptop, a
later render takes about one third of the video length, plus the text to speech for new
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
Add the video result: `video: embedded`, or `video: failed` with the reason. When a step
fails, report the failure and the last stderr lines. A failed explainer never blocks the
work of the caller. A failed video never blocks the page.

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

With `--video <path>`, the page shows a `<video controls preload="metadata" playsinline>`
block between the title and the section navigation. The path must be relative, end in
`.mp4` and have no `..` segment. Without `--video`, the page has no video block.

## The video

`video/` is a Remotion project with two compositions, `plan` and `ship`. Each composition
takes the props JSON and the voiceover clips as its input props. The video is 1920x1080
at 24 fps, with the voiceover as its audio. It has no captions.

- The scenes follow the page. `plan`: title, goal, architecture, timeline, decisions,
  risks. `ship`: title, summary, changes, delivered, deviations, setbacks and hiccups,
  review, hand test, rollback.
- The colors are the dark color tokens of `template/page.html`. A test compares them.
- Each scene shows few items, with large text for a phone screen. A long list continues in
  the next scene, up to a limit. After the limit, the scene shows `+N more on the page`.
  Long text ends in an ellipsis.
- The narration sets the length. Each scene has a minimum visual time: reading time for
  its words, from 5 to 10 s. A section lasts the longer of the minimum visual time of its
  scenes and its audio time plus 0.6 s. A section with several scenes shares its time
  between them.

`render-video.mjs` never writes into the skill directory. The skill directory can be a
symlinked git checkout or a Loop integration worktree. The script copies `video/` to
`${XDG_CACHE_HOME:-$HOME/.cache}/visual-explainer/video-<key>`. The key is 12 hex
characters of a SHA-256 over the lock file and the video sources. The script runs
`npm ci`, the browser download and the bundle there, one time for each key. The voiceover
clips go to `${XDG_CACHE_HOME:-$HOME/.cache}/visual-explainer/tts/`. The script writes the
MP4 to a temporary file in the output directory, then renames it to `video.mp4`. When the
file is larger than `--max-mb`, the script renders again at a lower quality, then at
1280x720. Each run removes the caches of other keys that no run used for 7 days.

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

To change the template, the scenes, the video or a schema, edit the skill files. Then run
`zsh tests/visual-explainer.test.sh` from the repository root. Set `VE_VIDEO_E2E=1` to
also render the two examples as real narrated videos. That run uses the real text to
speech engine.
