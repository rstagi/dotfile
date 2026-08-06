# Exporting the deck to PDF — the verified recipe

> **This is the reveal.js → PDF fallback path** (used only when a PDF is the deliverable). The **vector/selectable rule** and the **poppler verification block** below are the shared PDF QA — apply them to any deck-to-PDF, however produced. The reveal-specific export mechanics (`?print-pdf`, the export driver, publishing) are this path's.

Goal: a **vector, text-selectable** PDF with **embedded fonts** and **exactly one page per slide**. That's what conferences want (crisp on a projector, searchable, small file) and it's what the bundled `export-pdf.mjs` produces. Background: reveal's print docs at https://revealjs.com/pdf-export/ and the entries in [`../../talk/references/sources.md`](../../talk/references/sources.md).

## The default path: `npm run export:pdf`

```bash
cd <working-dir>/deck
npm run export:pdf            # → deck.pdf
OUT=make-it-boring.pdf npm run export:pdf   # rename the output
```

What it does, so you can reason about failures:

1. Starts the local `serve.mjs` on a random port.
2. Launches **your system Google Chrome** (`channel: "chrome"`, headless). If Chrome isn't installed, it falls back to Playwright's bundled Chromium.
3. Loads `http://127.0.0.1:<port>/?print-pdf`. `?print-pdf` is reveal.js's **built-in** print stylesheet — there is **no** manual print-CSS `<link>` to add. It reflows every slide to one page at the deck's configured size.
4. Waits for `Reveal.isReady()` and `document.fonts.ready` so glyphs are loaded before capture (this is why fonts embed).
5. Calls `page.pdf({ preferCSSPageSize: true, printBackground: true, margin: 0 })`. `preferCSSPageSize` makes Chrome honour reveal's page size → correct 16:9 pages; `printBackground` keeps your dark canvas and accent fills.

Verified: 10-page 16:9 deck → vector, text-selectable, fonts embedded.

### If it can't find Chrome

```bash
cd <working-dir>/deck && npx playwright install chromium
```

One-time. Then re-run the export; the bundled Chromium fallback kicks in.

## The manual path (no Node tooling at all)

Anyone with Chrome can do this — useful on a locked-down machine:

1. `npm run dev` (or any static server for `public/`).
2. Open **`http://127.0.0.1:8000/?print-pdf`** in **Chrome or Chromium** (not Firefox, not Safari — `?print-pdf` is Chromium-only).
3. **Print** (Cmd/Ctrl+P) → **Save as PDF**.
4. Set: **Layout = Landscape**, **Margins = None**, **Background graphics = ON**. (Background graphics off = white slides.)

## The CI / scripted alternative: decktape

If you'd rather not depend on Playwright, [decktape](https://github.com/astefanutti/decktape) drives a headless browser and knows reveal natively:

```bash
npx decktape reveal "http://127.0.0.1:8000/" deck.pdf
```

Same vector/selectable result. It's the documented fallback when the Playwright script is inconvenient; the in-repo `export-pdf.mjs` is preferred because it's already wired to the project's server and font-readiness waits.

## CI / offline: self-host the fonts

The starter loads fonts from Google Fonts over the network. In CI, a container, or anywhere offline, that link can silently fail and the PDF renders **box glyphs** (□□□) instead of text. Two fixes:

- **Self-host** (the durable fix): download the `.woff2` files (e.g. via https://gwfh.mranftl.com/fonts), drop them in `deck/fonts/`, replace the `<link>` in `index.html` with an `@font-face` block pointing at them, and mirror the same family names in `accents.css`. Now the export needs no network.
- **Install system fonts in the CI image** if you're loading by family name from a font package.

Either way: **always embed images locally** (under `assets/`) — a remote `<img src="https://…">` will be missing in a headless/offline export. The starter's `assets/p99.svg` is already local; keep new images the same way.

## The vector / selectable rule

Prefer any path that goes through reveal's `?print-pdf` + `page.pdf` (the script, the manual Chrome route, or decktape). Those keep text as **vector glyphs you can select and search**. Avoid: screenshotting slides to PNG and stitching a PDF, or any "export to images" path — small code fonts turn to mush and the text isn't searchable. SVG charts stay vector through this pipeline; raster (PNG/JPG) images stay raster but embed fine.

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| PDF has **more pages than slides** | a stepped code highlight (`[1-2\|4-5]`) or fragments exported per-step | collapse the highlight to one range (`[5]`); confirm `pdfSeparateFragments: false` and `pdfMaxPagesPerSlide: 1` in `index.html`, re-export |
| **Box glyphs** (□□□) | font didn't load during export | network was down / `<link>` family names don't match `accents.css`; self-host the fonts |
| **White slides** in the PDF | background graphics off (manual path) or `printBackground` false | turn Background graphics ON; the script already sets `printBackground: true` |
| A **slide is missing** content / an image is blank | remote image URL, or a path that 404s | use a local path under `assets/`; check the dev server console for 404s |
| **`pdftotext` returns nothing** | the deck rastered | a fragment/animation forced image rendering — simplify the slide, confirm config flags, re-export |
| Export **hangs** then fails | reveal never reached ready, or a JS error on a slide | run `npm run dev`, open `?print-pdf` in Chrome, watch the console; fix the erroring slide |
| `channel: "chrome"` error | no system Chrome | `npx playwright install chromium` (Step from the default path) |

## Verifying the result (poppler)

```bash
pdfinfo deck.pdf | grep Pages     # == your slide count
pdffonts deck.pdf                  # every row: emb=yes
pdftotext deck.pdf - | head        # real words → selectable, vector
```

No poppler? Open the PDF, page through it (count == slide count), and drag-select a line of text — if it highlights as text, you're vector/selectable.
