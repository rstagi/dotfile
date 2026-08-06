# Theming & composability — the `--talk-*` tokens, accents, and how to publish

> **Scope.** The `--talk-*` **token vocabulary** and the **meaning-mapped accent doctrine + accessibility checks** below are *renderer-agnostic* — the shared bridge from `04-slides-style.md` to **both** build paths. The default React world-deck consumes the same token block as CSS variables in `index.css` (see [`react-deck.md`](react-deck.md) §3). The **file mechanics** here (`theme/talk.css` + `theme/accents.css` loaded after `reveal.css`, `<section data-markdown>` authoring, `build.mjs` → `public/`) are specific to the **reveal.js → PDF fallback path**.

Two files own the look *on the reveal/PDF path*: `theme/talk.css` (the bespoke base — copy it, don't touch it) and `theme/accents.css` (the per-talk skin — the only file you edit to re-colour or re-type a talk). Everything visual derives from a small set of `--talk-*` CSS variables. This is how you escape the default reveal look *and* the generic auto-deck look without rebuilding CSS each time.

## Why the theme exists at all

`talk.css` is deliberately editorial: warm near-black canvas (never `#1e1e2e`), a display serif (Fraunces) for headings paired with a humanist sans (Spline Sans) for body and JetBrains Mono for code, left-aligned text with generous margins, thin rule list-markers instead of bullets, and `center: false`. It exists to pre-fail the Gate C tells: no AI-blue→purple gradient, no centered-everything, no Inter, no emoji bullets. You inherit all of that by copying it unchanged.

## The token system

`talk.css` defines every reveal variable (`--r-*`) in terms of a dozen `--talk-*` tokens at the top of `:root`. Redefine the `--talk-*` tokens in `accents.css` (loaded *after* `talk.css`) and the whole deck re-skins. The tokens:

| Token | Means | Default |
| --- | --- | --- |
| `--talk-bg` | canvas | `#17130f` warm near-black |
| `--talk-ink` | body text | `#ece3d4` warm off-white |
| `--talk-ink-bright` | headings, `<strong>` | `#fbf6ec` |
| `--talk-muted` | captions, slide numbers, de-emphasis | `#8a7f6d` |
| `--talk-rule` | hairlines, table borders | `#3a3128` |
| `--talk-surface` | code blocks, cards | `#211b15` |
| `--talk-accent` | **the subject / "now" / spotlight** | `#e8643c` vermilion |
| `--talk-accent-2` | **the contrast / "what could be"** | `#43b4a0` teal |
| `--talk-font-display` | headings | Fraunces |
| `--talk-font-body` | body | Spline Sans |
| `--talk-font-mono` | code, kickers, slide numbers | JetBrains Mono |

You should only ever edit these in `accents.css`. Don't reach into `talk.css` to restyle a heading — change the token. If you need a one-off style the tokens can't express, add a tiny rule to `accents.css` *below* the token block, not in `talk.css`.

## The two accents are meaning-mapped — this is the point

Don't pick accent colours because they "look nice". Pick them because each one *stands for something in this talk*, then use them consistently so the audience learns the code. The convention baked into the theme:

- `--talk-accent` = the subject under the spotlight / the "now" / the thing you're arguing about.
- `--talk-accent-2` = the contrast / the "what could be" / the alternative.

Worked mappings:

- A migration talk: `--talk-accent` = the legacy system, `--talk-accent-2` = the new one.
- A perf talk: `--talk-accent` = "slow / before", `--talk-accent-2` = "fast / after" (this is exactly what the starter's `p99.svg` chart does — the after-bar is the accent).

Put a one-line comment in `accents.css` naming what each accent means for *this* talk, so the next person (or the delivery skill) doesn't have to guess.

Before shipping, run the accents through Gate C's colour checks: WCAG ≥4.5:1 for ink-on-bg and ≥3:1 for large/graphical; prefer an Okabe-Ito / ColorBrewer pair (blue+orange is the safest colourblind-safe pair); confirm the two accents are still distinguishable in grayscale; and confirm you did **not** ship the defaults unchanged or a blue→purple gradient.

## Fonts: change them in two places, always

A font choice lives in **two** files and they must agree:

1. `accents.css` — the `--talk-font-display` / `-body` / `-mono` tokens (the CSS family names).
2. `index.html` — the Google Fonts `<link>` that actually loads those families.

If you change the token names but not the `<link>` (or vice versa), the browser silently falls back to a system font: an instant slop tell on screen and box glyphs / wrong glyphs in the PDF. Pick faces with character; avoid Inter/Roboto/Arial/Times as unconscious defaults. The starter ships a "warm paper" light-mode block commented out at the bottom of `accents.css` — uncomment it to flip the whole deck to a light background in one move.

## Single-file vs per-section-file authoring

**Default — single file.** Everything is in `slides.md`, split on lines of `---` (`--` vertical, `Note:` notes). `index.html` references it once:

```html
<section data-markdown="slides.md"
         data-separator="^\r?\n---\r?\n"
         data-separator-vertical="^\r?\n--\r?\n"
         data-separator-notes="^Note:"></section>
```

**Composable — one file per section.** For a longer deck, split into `deck/slides/01-intro.md`, `deck/slides/02-…md`, etc., and list each as its own `<section>` in `index.html`:

```html
<div class="slides">
  <section data-markdown="slides/01-intro.md" data-separator="^\r?\n---\r?\n" data-separator-notes="^Note:"></section>
  <section data-markdown="slides/02-cost.md"  data-separator="^\r?\n---\r?\n" data-separator-notes="^Note:"></section>
  <section data-markdown="slides/03-fix.md"   data-separator="^\r?\n---\r?\n" data-separator-notes="^Note:"></section>
</div>
```

Now **reordering the talk = reordering those lines** (or renaming the files), and git diffs stay clean and small. Each `data-markdown` file is itself split on `---` internally, so a "section" file can hold several slides. Use this when sections move around a lot or when two people edit different parts; use the single file for short decks.

## Publishing: `npm run build` → `public/`

```bash
cd <working-dir>/deck && npm run build      # → deck/public/
```

`build.mjs` assembles a self-contained static site in `public/`: it copies reveal's `dist/` and `plugin/` into `public/vendor/`, copies `theme/`, copies `index.html` unchanged (its paths are already relative), and copies whatever exists of `slides.md`, `slides/`, `assets/`, `images/`. Because `index.html` references `vendor/` and `theme/` relatively, the same file works in dev (where the server aliases `/vendor/`) and in the built site (where `vendor/` is real). Preview with `npx serve public`; deploy by pointing GitHub Pages, Vercel, or Netlify at `deck/public/`. The site and the PDF are independent artifacts — build whichever the user needs.
