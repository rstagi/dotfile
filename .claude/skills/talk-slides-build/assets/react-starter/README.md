# TALK_TITLE — the deck

A custom, world-built talk deck: React + Vite + Framer Motion, rendered on a fixed
16:9 stage that scales to any screen. Built from the `talk-slides-build` React starter.

## Run

```bash
npm install
npm run dev        # → http://localhost:5173
npm run build      # → dist/  (static site; deploy to any host)
npm run preview    # serve the built site
```

## Present

- **→ / Space / Enter** — next (advances fragments within a slide, then the next slide)
- **← / Backspace** — back
- **N** or **S** — toggle the **presenter-notes overlay** (your cue cards; it's in the
  same tab, so turn it OFF before recording / screen-sharing the take)
- **Home** — restart

The deck letterboxes to 16:9 in any window — share/record the browser tab full-screen.

## Where things live

| File | What |
| --- | --- |
| `src/data/slides.ts` | **The content.** One entry per slide: assertion title (verbatim from `02-slides-content.md`), `steps` (fragment count), `notes` (cue cards, verbatim from `03-delivery-plan.md`). Inline accents: `<a1>…</a1>`, `<a2>…</a2>`, `<em>…</em>`. |
| `src/data/sections.ts` | The narrative arc (the chrome's section label). |
| `src/index.css` | Design tokens. Fill the palette hex + font families from `04-slides-style.md`; the token **names**, the 16:9 stage, and the type scale stay fixed. |
| `index.html` | The Google Fonts `<link>` (match it to `index.css`) + `<title>`. |
| `src/components/scenes/` | One component per layout (title, statement, data, quote, twocol, image, code). |
| `src/components/world/` | **The per-talk world** — your metaphor's scenes + hand-built SVG primitives. Start here to make it yours (see its README). |
| `src/components/core/` | The engine: navigation, transitions, chrome, presenter notes. Leave it. |

## The two-axis navigation model

`currentIndex` = which slide; `subStep` = which fragment within it. A slide declares
`steps` (how many fragments). `→` advances the fragments, then the slide. Scenes read
`subStep` and gate reveals on `subStep >= N` (see `StatementScene`).

## Rules baked in

- Every visual is hand-built (SVG / real photography) — **no AI-generated or stock images**.
- Two **meaning-mapped** accents only (`--talk-accent` / `--talk-accent-2`); decide what each
  stands for in `04-slides-style.md` and keep it consistent.
- Everything sizes in container-query units (`cqw`/`cqh`) so the deck is resolution-independent.
- Need a vector **PDF** instead (conference upload, leave-behind)? That's the reveal.js
  fallback path — see the `talk-slides-build` skill.
