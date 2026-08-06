# Talk deck

A [reveal.js](https://revealjs.com) deck with a bespoke editorial theme. Slides are plain
Markdown; the deck runs as a website and exports to a vector, text-selectable PDF.

## Setup

```bash
npm install
# one-time, for PDF export if you don't have Google Chrome:
npx playwright install chromium
```

## Write

Edit **`slides.md`**. One `---` on its own line starts a new slide; `--` starts a vertical
sub-slide; everything after a `Note:` line is a speaker note (press **S** for speaker view).

Per-slide layout and background go in an HTML comment at the top of the slide:

```markdown
<!-- .slide: class="talk-section" data-background-image="assets/cover.jpg" -->
```

Layout classes (defined in `theme/talk.css`): `talk-title`, `talk-section`, `talk-statement`,
`talk-quote`, `talk-fullbleed`, `talk-2col`, `talk-data`. Reveal one element at a time with
`<!-- .element: class="fragment" -->`. Highlight code lines with
` ```py [1-2|5] ` or `data-line-numbers`.

**Composing a bigger deck from reusable files:** replace the single `<section data-markdown="slides.md">`
in `index.html` with one `<section data-markdown="slides/01-intro.md">` per file — reordering the
deck is then reordering those lines (or renaming the files).

## Re-skin

Edit **`theme/accents.css`** only — redefine the `--talk-*` tokens (the two accents are
meaning-mapped: pick colours that *stand for something* in this talk). Mirror any font change in
the Google Fonts `<link>` in `index.html`. Never ship the defaults unchanged.

## Present / export / publish

```bash
npm run dev          # http://127.0.0.1:8000  (S = speaker view, F = fullscreen)
npm run export:pdf   # → deck.pdf  (OUT=name.pdf to rename)
npm run build        # → public/  (deploy to any static host, or: npx serve public)
```

**Manual PDF (no tooling):** open `http://127.0.0.1:8000/?print-pdf` in Chrome → Print →
*Save as PDF*, Landscape, Margins **None**, **Background graphics ON**.

## Offline / CI fonts

The fonts load from Google Fonts over the network. For a fully offline build or reproducible CI
export, [self-host them](https://gwfh.mranftl.com/fonts): drop the `.woff2` files in `fonts/`,
swap the `<link>` for an `@font-face` block, and the PDF will embed real glyphs instead of
falling back. Always embed images locally (already done — see `assets/`).
