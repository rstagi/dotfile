# The custom React world-deck (default build path)

This is the default render target: a bespoke **React + Vite + Framer Motion** app where the talk's metaphor becomes a navigable *world*. The engine is bundled and verified in `assets/react-starter/`; you skin it from `04`, fill it from `02`/`03`, and **author the per-talk world by hand**. reveal.js → PDF is the fallback (see [`pdf-export.md`](pdf-export.md)); use it only when a vector PDF is the deliverable.

Why a custom app and not a slide template: a template flattens the metaphor into bullet points. The world *is* the memorability device — the GoT talk's throne that changes hands, the indie talk's RPG map you progress through. The cost is that you build the world each time; the engine below makes that the only thing you build.

---

## 1. The engine (bundled — don't rewrite it)

`cp -R assets/react-starter/. <deck>/ && npm install`. What you get:

| File | Role | Touch it? |
| --- | --- | --- |
| `hooks/useSlideNavigation.ts` | Two-axis nav state machine (`currentIndex` + `subStep`). | No — engine. |
| `hooks/useKeyboardNav.ts` | →/Space/Enter next, ← back, N/S notes, Home restart. | No. |
| `components/core/SlideContainer.tsx` | Framer cross-fade-with-drift between slides. | Rarely (easing/duration). |
| `components/core/DeckChrome.tsx` | Persistent chrome: section label, optional `badge` slot, counter, progress rule. | Pass props; the world fills `badge`. |
| `components/core/PresenterNotes.tsx` | Toggleable cue-card overlay (N/S). | No — feed it `notes`. |
| `components/core/rich.tsx` | Inline accent parser: `<a1>/<a2>/<em>`. | No. |
| `components/scenes/*` | Seven generic layouts (below). | Extend / restyle. |
| `components/charts/ExampleChart.tsx` | One decluttered chart showing the conventions. | Replace with real charts. |
| `components/world/` | **Empty.** The per-talk world lives here. | **Yes — this is your job.** |
| `data/slides.ts` | The typed content model + an example deck. | **Yes — replace with the talk.** |
| `data/sections.ts` | The narrative arc (chrome label). | Yes. |
| `index.css` | Token scaffold + 16:9 stage + scene shells + type primitives. | Fill token VALUES only. |
| `App.tsx` | Dispatcher: wires hooks, `switch(type)→scene`, chrome, notes. | Add `case`s for world scenes. |

The starter **builds clean out of the box** (`npm run build` = tsc + vite). Verify it still does after your edits.

## 2. The two-axis navigation / `subStep` model

The deck is not a list of slides you click through one-per-press. It has two axes:

- **`currentIndex`** — which slide.
- **`subStep`** — which fragment *within* that slide. `subStep 0` is the slide at rest; `subStep N` reveals fragment N.

Each slide declares `steps` (its fragment count) in `slides.ts`; `App` feeds `getStepCount = slides[i].steps` to the hook. `→` advances `subStep` until it hits `steps`, **then** advances to the next slide at `subStep 0`. `←` decrements `subStep`, and crossing a slide boundary backward lands on the previous slide **fully revealed** (so backtracking shows the completed slide, not its cold state). `Home` jumps to slide 0.

Scenes receive `subStep` as a prop and gate reveals on it with Framer `AnimatePresence`:

```tsx
{subStep >= 1 && <motion.span initial={{opacity:0,y:8}} animate={{opacity:1,y:0}}>{reveal}</motion.span>}
```

This is how you honor `02`'s motion plan: one reveal per click, signaling/segmenting only — never decorative. Set `steps` to the number of reveals the slide needs.

## 3. The token + `cqw` scaffold (skin from `04`, don't re-derive)

The deck is a single fixed **16:9 stage** (`.deck`), letterboxed to any window:

```css
.deck { width: min(100vw, 177.78vh); height: min(56.25vw, 100vh);
        container-type: size; container-name: deck; }
```

Because `.deck` is a query container, **every size on the stage is in container-query units** — `cqw` (1% of stage width), `cqh` (1% of stage height). The whole type scale (`--fs-kicker: 1.25cqw` … `--fs-display: 7.4cqw`), `--margin`, scene padding, chart widths — all `cqw/cqh`. Result: **resolution-independent** — projector, laptop, 4K recording all scale proportionally with zero media queries and zero JS. px is used *only* inside fixed-`viewBox` SVGs, which then scale via a `width:%/cqw` wrapper.

**The invariant that makes this reusable:** scenes and world primitives reference CSS variable **names** (`--talk-accent`, `--fs-*`, `--margin`) and the `cqw/cqh` scale — never literal colors, fonts, or px layout. So re-skinning is **values-only**:

- In `index.css`, paste `04`'s token block over the placeholder values (8 `--talk-*` palette hex + the 2 accent-dim tints `--accent-dim`/`--accent-2-dim` + 3 font families). Leave the names, the stage math, the scale, the scene shells.
- In `index.html`, paste `04`'s exact Google Fonts `<link>`. **It must load the families named in `index.css`** or the browser silently falls back to a system font — a slop tell. (Names must agree in two places.)
- Decide what each of the **two meaning-mapped accents** stands for (per `04`) and keep it consistent. Don't add a third accent. `<a1>` = `--talk-accent`, `<a2>` = `--talk-accent-2`.

When you add a new size, write it in `cqw/cqh`. When you add a color, it must be one of the existing tokens.

## 4. The seven generic scenes

Map `02`'s layout classes onto these (then add world scenes as needed):

| Scene | For | Notes |
| --- | --- | --- |
| `title` | `talk-title` | Left title block (kicker/flourish/title/subtitle/byline). `chrome:false`. The world can add a right-hand decorative column. |
| `statement` | `talk-statement` | The spine line. `main` + optional `subStep 1` `reveal` clause + optional muted `anchor`. The canonical reveal demo. |
| `data` | `talk-data` | `scene--flow` (top-aligned). `flow-head` assertion + one decluttered chart + `Source ·` caption + footnote. |
| `quote` | `talk-quote` | Big quote mark + italic blockquote + attribution + note. |
| `twocol` | `talk-2col` | 1fr / divider / 1fr; color per column via `accent-1`/`accent-2`; staggered reveals. |
| `image` | `talk-fullbleed` | Full-stage media + lower-third caption scrim. Real photography / hand-built figure only. |
| `code` | technical `talk-data` | Monochrome code on surface; the accent reserved for the token(s) in `highlight`, revealed one per `subStep`. |

**`scene` vs `scene--flow`:** `.scene` vertically centers (statements, titles, quotes). `.scene--flow` top-aligns below the chrome (data, code, twocol — anything with a tall stack). If a slide's content collides with the chrome or the bottom counter, it wants `scene--flow` and/or a smaller `flow-head`.

## 5. The presenter-notes overlay

Press **N** or **S** to toggle. It reads each slide's `notes: { cues: string[], line?: string, memorable?: boolean }` — lifted **verbatim** from `03-delivery-plan.md`. `line` is the hand-written opening/closing line (accent-bordered); `memorable: true` stars the header. It renders in the same tab, above the stage — **a presenter aid; toggle it OFF before recording or screen-sharing the take.** This is how the delivery cue cards ship *inside* the deck.

## 6. Building the per-talk world (this is the work)

The engine is shared; the world is not. A world is three additive things, none of which touch the engine:

1. **New scene types.** Add the `type` to the `SceneType` union + a content interface in `slides.ts`, write `XScene.tsx`, add a `case` to `renderScene` in `App.tsx`.
2. **Hand-built SVG primitives** in `components/world/`. Animated SVG that reads `--talk-*` CSS vars and reacts to `subStep`/state. Size them with a fixed `viewBox` + a `width:100%` (or `cqw`) wrapper so they scale with the stage. **Hand-built only — never AI-generated or stock imagery.**
3. **A chrome badge.** Pass `badge={…}` into `<DeckChrome>` to dress the top-right slot.

**Worked example — GoT "AIron Throne" deck** (`~/Dev/playground/aiewf-online-talk/`):
- `world/Throne.tsx`: an Iron-Throne SVG whose **occupant color + draped sigil flip** JS→Python→TypeScript via a `holder` prop driven by `subStep`/slide. The throne is neutral; the *holder's* accent is the meaning.
- `world/HouseSigil.tsx`, `world/Raven.tsx`: heraldic crests, a messenger raven for the "news arrives" beat.
- `RealmChrome` (a themed `DeckChrome`): showed the current "Book" + a "who holds the throne" sigil that flipped teal→vermilion exactly at the memorable-moment slide.
- World scenes: a `ThroneScene` with a full-bleed "crowning" variant (headlines slam in as royal proclamations), a `RoadScene` (an unbroken road vs a wall at the boundary), a `BannermenScene` (the tools as houses declaring).
- Sections were "Books"; the protagonist was the *crown*, not a person (the speaker's constraint).

**Worked example — Pokémon/RPG deck** (`~/Dev/playground/talk-indie-milano-20251218/`): an SVG world map with terrain/paths/sprite markers, HP bars, boss fights, loot drops, level-ups, chiptune music. Same engine, entirely different world.

**Keep the metaphor tasteful, not kitsch.** The GoT deck rendered the world as an *illuminated-manuscript / heraldic* treatment that matched the editorial type and palette — elegant, not fantasy clip-art. Let `04`'s look discipline the world.

## 7. Framed beats + clean data (the balance)

Don't world-build every slide. The pattern that works: **narrative beats are fully world-built** (title, the metaphor scenes, the memorable moment), **data and code slides stay clean and editorial** (decluttered charts, monochrome code) — these are the "standard slides in the middle." The persistent chrome ties the two together. Burying a chart in heavy world chrome reads as cheerleading and kills the data's credibility.

Charts: hand-built SVG, **no gridlines / no legend / no border**, direct labels, tabular-mono numerals, **one accent on the takeaway**, everything else muted. Add each as a component and switch on `content.chart` in `DataScene`. `ExampleChart.tsx` is the template.

## 8. Verify (don't trust, check)

- **`npm run build` succeeds** — tsc + vite. A failing build ships nothing.
- **`slides.length === N`** (the count from `02`).
- **`04`'s tokens are actually applied** — the palette and fonts render, not the placeholder defaults (open the deck and look; a system-font headline = the font `<link>` didn't match).
- **Keyboard nav + presenter overlay work**; fragments reveal in order per `02`'s motion plan.
- **No generated/stock images** — every visual hand-built SVG or real photography.
- **Run Gate C** (`../talk/references/keep-it-human.md`) on the rendered deck. The React look-failure mode is **generic component-library defaults** (shadcn out-of-the-box, an unedited AI-gradient hero, copy-paste card grids) — not "the default reveal theme."
- **Visually walk it** at presentation size; nothing overflows the 16:9 stage (if it does → `scene--flow` / smaller `flow-head`). A quick way to catch overflow/collision: drive the dev server with a headless browser, press through the deck, and screenshot the dense slides.

## 9. Gotchas

- **Silent font fallback.** The single most common slop tell. The `index.html` `<link>` families must exactly match `--talk-font-*` in `index.css`.
- **Overflow into chrome / counter.** Tall scenes (data/code/twocol) must use `scene--flow` (top-aligned). Shrink `flow-head` or the chart width (`cqw`) if it still collides.
- **SVG sizing.** Draw in a fixed `viewBox` (px inside is fine) and scale via a wrapper `width` in `cqw` + `width:100%` on the `<svg>` — never hardcode px on the stage outside a viewBox.
- **Redundant labels.** If the persistent chrome already shows the section/book, don't repeat it as a slide kicker.
- **Fragments as decoration.** Only add `steps`/reveals where `02` calls for signaling or segmenting. Default is a clean cut/fade.
- **No PDF here.** This path presents live / records. If they need a vector PDF, that's the reveal.js fallback — don't try to print the React app to PDF as a substitute (it rasters).
