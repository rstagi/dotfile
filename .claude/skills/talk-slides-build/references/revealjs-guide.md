# reveal.js authoring — the Markdown subset this deck uses

> **This is the reveal.js → PDF fallback path.** Use it only when the deliverable is a vector PDF (see SKILL.md's decision gate). The default build is a custom React world-deck — for that, see [`react-deck.md`](react-deck.md), not this file.

reveal.js 5.2.1, driven by the Markdown plugin. You write Markdown, not HTML `<section>`s. The starter's `index.html` already configures the separators below — you only edit `slides.md` (and `accents.css`). This file is the syntax you'll actually reach for; the upstream docs are at https://revealjs.com.

## Slide separators

The starter wires these regexes in `index.html` (`data-separator` / `data-separator-vertical` / `data-separator-notes`):

| You write | You get |
| --- | --- |
| a line that is only `---` | a new **horizontal** slide (the next one in the linear flow) |
| a line that is only `--` | a **vertical** sub-slide (stacks *below* the current one; down-arrow reaches it) |
| `Note:` then text to end of slide | a **speaker note** (hidden on screen; press `S` for speaker view) |

The `---` and `--` must be alone on their line with a blank line above and below. Vertical stacks are useful for an optional deep-dive you can skip live — but remember each vertical slide is its own PDF page.

```markdown
# First slide

---

# Second slide

--

A sub-point under the second slide.

Note:
This whole block is a speaker note. It does not render on the slide.
```

## Per-slide settings — the `<!-- .slide: ... -->` comment

Put an HTML comment as the **first thing** on a slide to set attributes on its `<section>`. This is where layout classes and backgrounds go:

```markdown
<!-- .slide: class="talk-statement" -->

The cleverest system in the room is usually the one on fire.
```

Useful attributes:

- `class="talk-…"` — the layout class (see the table below).
- `data-background-image="assets/cover.jpg"` — full-bleed image background (pair with `class="talk-fullbleed"` for the legibility scrim). Local paths only.
- `data-background-color="#17130f"` — solid colour background for one slide.
- `data-transition="fade"` / `"none"` — override the deck default for one slide. Keep it to cut or fade; no spin/zoom/convex.
- `data-auto-animate` — only if you genuinely have one object morphing between two slides; otherwise skip it.

## Per-element settings — the `<!-- .element: ... -->` comment

Put a comment *after* the element (same line or next line) to set attributes on that element — most often to make it a fragment (revealed on click):

```markdown
- A retry loop that "self-healed" — into a thundering herd <!-- .element: class="fragment" -->
- A cache smarter than its invalidation <!-- .element: class="fragment" -->
```

Fragment variants worth knowing: `fragment fade-up` (the theme gives this a calm rise), `fragment fade-out` (present then dim), `fragment highlight-current-blue` etc. Use fragments for *signaling/segmenting* — revealing one idea at a time — never for decoration. **Each fragment step is a separate PDF page unless you set `pdfSeparateFragments: false`** (the starter does set it false, so fragments collapse onto one printed page — good).

## Layout classes (defined in `theme/talk.css`)

Map the slide-spec's intent to one of these. They are the deck's vocabulary; using them is how you avoid centered-everything sameness.

| Class | Use it for | Shape |
| --- | --- | --- |
| `talk-title` | open / close | oversized display headline + `.kicker` + `.byline` |
| `talk-section` | section divider | big tabular `.num` + short `h2` with an accent rule |
| `talk-statement` | one huge sentence (the throughline, a turn) | Lessig/Takahashi single-line display |
| `talk-quote` | a pull quote | `blockquote` + `.attrib` |
| `talk-fullbleed` | photo with text in its negative space | needs `data-background-image`; auto scrim for legibility |
| `talk-2col` | two parallel ideas | wrap content in `<div class="cols"><div>…</div><div>…</div></div>` |
| `talk-data` | a chart that proves the headline | assertion `h2` on top, chart in `<div class="chart">` below |

Helper spans/paras the classes expect: `.kicker`, `.byline` (on `talk-title`); `.num` (on `talk-section`); `.attrib` (on `talk-quote`); `.chart` (on `talk-data`). See `slides.md` in the starter for a worked example of each.

## Code blocks with line highlighting

Fenced code runs through highlight.js (the theme ships a bespoke warm palette, not stock monokai). Add a language and an optional line spec in brackets:

````markdown
```python [5]
delay = min(MAX_DELAY, base * 2 ** attempt) + random_jitter()
```
````

- `[5]` highlights line 5.
- `[1-2|4-5]` is a **stepped** highlight: click to move from lines 1–2 to lines 4–5. Great live. **Collapse it to a single range (`[5]` or `[1-5]`) before exporting the PDF** — each step is otherwise its own printed page.
- Equivalent attribute form: a fence info string is converted to `data-line-numbers="…"` on the `<code>`. You rarely need the raw attribute when authoring in Markdown.

Keep code to what fits the back row at the theme's `0.6em` code size — roughly 10–12 lines. If it's longer, it's a handout, not a slide.

## Images, charts, tables

- **Images:** `![alt](assets/name.png)` or raw `<img>` for sizing control. Always local files under `assets/` (or `images/`) — remote URLs break offline/CI export, and AI-generated images fail Gate C.
- **Charts:** prefer a hand-built, decluttered **SVG** (one message, direct labels, accent points at the takeaway — see the starter's `assets/p99.svg`). SVG stays vector in the PDF and scales crisply. Drop it on a `talk-data` slide inside `<div class="chart">`.
- **Diagrams:** reveal has no built-in Mermaid. Render the diagram to a static SVG/PNG ahead of time and embed it like any image. Don't pull in a Mermaid plugin just for one box-and-arrow.
- **Tables:** plain Markdown tables work and pick up the theme's accent header rule.

## Plugins loaded by the starter

`index.html` loads exactly three: `RevealMarkdown` (the slide syntax above), `RevealHighlight` (code), `RevealNotes` (speaker view). That's all this deck needs. If you genuinely need math, add `RevealMath` and its script; otherwise don't add plugins — every extra script is one more thing that can break the headless PDF export.

## Presenter mode

Press **`S`** in the dev server to open speaker view in a second window: current slide, next slide, your `Note:` text, and a timer. This is what `talk-delivery` rehearses against — which is why the `Note:` blocks matter. Other live keys: `F` fullscreen, `B`/`.` blackout, `O` slide overview, `ESC` exit overview, arrows/space to navigate, `Alt+click` to zoom.

## Reveal config (already set in `index.html`)

The starter sets `width: 1280, height: 720` (16:9), `center: false` (the theme controls vertical rhythm — do **not** flip this to `true`), `transition: "fade"`, `slideNumber: "c/t"`, `pdfMaxPagesPerSlide: 1`, `pdfSeparateFragments: false`. The last two keep the PDF at one page per slide. Leave them unless you have a specific reason; if you change `pdfSeparateFragments`, your page count will no longer equal your slide count.
