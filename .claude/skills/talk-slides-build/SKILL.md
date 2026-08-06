---
name: talk-slides-build
description: |
  Build a conference talk deck — by default a custom, world-built React + Vite + Framer Motion app (scaffolded from a bundled, verified engine), or, when a vector PDF is required, a reveal.js site exported to a text-selectable PDF. Either way it translates the slide-content spec into the deck, applies the style skin, and lifts the delivery cue cards into the deck's per-slide speaker notes. Use whenever the user has slide content + a style spec + a delivery plan ready and asks to "build the deck", "make the slides", "turn the spec into a deck", "build the talk app", "give me a PDF of the talk", "publish the deck as a site", or invokes `/talk-slides-build`. Trigger on phrases like "build the deck", "scaffold the talk site", "ship the deck", "render my slides to PDF", "export the presentation" — even if the user does not say "skill" or "talk-slides-build" by name. This skill writes a deck/ directory (a React app by default; reveal.js + deck.pdf on the PDF path) into the talk working directory; it does not author the slide structure (talk-slides-content), choose the fonts and palette (talk-slides-style), or write the cue cards (talk-delivery).
allowed-tools:
  - Read
  - Write
  - Edit
  - Bash
  - Glob
---

# talk-slides-build — custom React world-deck (default) · reveal.js → PDF (fallback)

Fifth stop in the talk pipeline, and the first one where bits become an artifact you can present from. It reads three upstream files and produces a deck you can run live (and, on the PDF path, a file you can hand to the conference). From `02-slides-content.md` it takes the structure — the assertion titles, the layout class per slide, the proving visual, the chart specs, the slide count `N`. From `04-slides-style.md` it takes the look — the paste-ready token block (`--talk-*` palette + fonts), the Google Fonts `<link>`, the per-layout-class token roles. From `03-delivery-plan.md` it takes the per-slide cue cards and lifts each one into that slide's speaker-notes block.

**The spine is locked upstream.** The throughline, the one memorable moment, and the named specifics (names/dates/numbers/places) were decided in `talk-content` and carried through unchanged. This skill is a **renderer, not a rewrite** — titles go in verbatim, accents go in as authored, cue cards go in as written. The hard part is not the tooling; it is resisting the generic auto-deck look and (on the PDF path) proving the PDF is real.

**Two render targets, one default:**

- **DEFAULT — a custom React + Vite + Framer Motion world-deck.** A bespoke app where the talk's metaphor becomes a *world* (a Game-of-Thrones realm, a Pokémon map, a city you travel) with persistent chrome, hand-built SVG, and fragment-staged reveals. This is what makes a talk memorable and is the right call for a live or recorded presentation. The engine is bundled and verified in `assets/react-starter/`; the per-talk world is authored fresh each time. See [`references/react-deck.md`](references/react-deck.md).
- **FALLBACK — reveal.js → vector PDF.** Only when the deliverable is genuinely a **PDF** (conference upload, print leave-behind, a venue that accepts only a PDF). reveal.js renders Markdown and Playwright/Chrome exports a vector, text-selectable PDF. Bundled in `assets/starter/` + `assets/theme/`. See [`references/revealjs-guide.md`](references/revealjs-guide.md) and [`references/pdf-export.md`](references/pdf-export.md).

## Pick the path first (decision gate)

Decide before you scaffold — the two paths share the upstream specs but nothing else:

1. **Default to the React world-deck** for anything presented live or recorded (the common case). It is more visually distinctive and is the format the rest of this skill optimizes for.
2. **Use the reveal.js → PDF path when the user needs a vector PDF** — a conference asks for a PDF upload, they want a print/leave-behind, or they explicitly ask for "a PDF of the talk." A React app does not export a clean vector PDF; reveal.js is the tool for that.
3. **Both** is possible (present from the React deck, submit the reveal PDF) — but it means authoring the content twice (the React data model *and* reveal Markdown). Only do it if they truly need both artifacts; confirm before doing the double work.
4. **Honest skip — neither fits:** if the venue mandates an **editable `.pptx` with builds** or a **corporate master template**, this stack is not it; say so rather than producing something they cannot submit.

When the user's need is ambiguous ("build the deck"), default to React and *mention* the PDF path exists if they need to submit one.

## When to use / honest skip

Use this when all three upstream artifacts exist (`02-slides-content.md`, `04-slides-style.md`, `03-delivery-plan.md` in the working dir) and the user wants a presentable artifact: a deck to run live (React) or a PDF to submit (reveal).

**Honest skip — don't build yet if:**

- **No slide-content spec.** If there is no `02-slides-content.md`, you would be inventing slide structure here — that is `talk-slides-content`'s job. Route to [`/talk-slides-content`](../talk-slides-content/SKILL.md) (or, if there is not even a brief, [`/talk-content`](../talk-content/SKILL.md)).
- **No style spec.** If `04-slides-style.md` is missing, you have no token block to fill the theme and no font `<link>` to match — that is [`talk-slides-style`](../talk-slides-style/SKILL.md)'s job. Get it first.
- **No delivery plan.** If `03-delivery-plan.md` is missing, the speaker notes have nothing real to carry. You *can* build and leave note stubs, but flag it: cue cards are [`talk-delivery`](../talk-delivery/SKILL.md)'s output and delivery runs *before* build.
- **They only need to share content, not present.** A deck is the wrong shape for a doc/leave-behind — point them back to the brief.
- **The venue mandates `.pptx`-with-builds or a corporate master.** See the decision gate above — neither path produces that.

## Inputs

Resolve the working directory the same way the rest of the suite does:

- If the user names a directory or slug, use `~/talks/<slug>/`.
- Else auto-detect by the presence of the upstream artifacts; the file that anchors this stage is `02-slides-content.md`.

```bash
# auto-detect the working dir (prefer cwd, then ~/talks/*)
for d in . ~/talks/*; do
  [ -f "$d/02-slides-content.md" ] && echo "found talk dir: $d"
done
```

Then read all three upstream artifacts. **The handoff contract is tooling-agnostic — it feeds both render targets identically:**

1. **`02-slides-content.md`** — the structure. Per slide: the complete-sentence assertion title, the layout class, the proving-visual concept, the per-slide word budget, the chart message/type/declutter/takeaway, the motion/reveal intent, and which slide carries the memorable moment. The slide count `N` lives here.
2. **`04-slides-style.md`** — the look. The paste-ready `--talk-*` token block (palette + the two meaning-mapped accents + the three font families), the Google Fonts `<link>`, the per-layout-class token roles, and the accent that marks each chart takeaway.
3. **`03-delivery-plan.md`** — the cue cards, keyed by slide number to match `02`'s order.

**Before you touch a file, write down:** the per-slide line (title + layout class + visual), the slide count `N`, the accent mapping and what each accent stands for, the typeface names, which slide carries the memorable moment, and any chart/diagram/image assets the spec calls for. **Never reach for a remote URL or an AI-generated image** — the proving visual is real photography or a hand-built SVG/diagram, decided upstream.

---

## Process A — DEFAULT: the custom React world-deck

### A1 — Read the specs and decide the world

Read `02`, `04`, `03` end to end (Inputs above). Then make the call this path is really about: **what is the world?** The talk's metaphor becomes a navigable world — the GoT talk's realm of houses contending for a throne; the indie talk's RPG map of quests and bosses. Decide:

- the **metaphor** (and confirm it with the user if it isn't obvious from the content's own language — the GoT talk's content was already a throne story, so the world was latent in it);
- the **persistent chrome** (what the `DeckChrome` shows: the current section, an optional per-talk badge, the progress);
- which **scenes** are fully world-built (the narrative beats) vs kept **clean and editorial** (data and code slides — the "standard slides in the middle");
- which slide carries the **memorable moment** and how the world dramatizes it.

If the user has a constraint (e.g. "I don't want to be the main character"), honor it — make the protagonist an object or idea, not a person.

### A2 — Scaffold from the bundled engine

Do **not** `npm init` or hand-write the engine. Copy the verified starter:

```bash
SKILL=/Users/rstagi/dotfile/.claude/skills/talk-slides-build/assets/react-starter
DECK=<working-dir>/deck            # e.g. ~/talks/make-it-boring/deck

mkdir -p "$DECK"
cp -R "$SKILL/." "$DECK/"
cd "$DECK" && npm install          # React 19 + Vite 7 + Framer Motion 12
```

What you get (full tour in [`references/react-deck.md`](references/react-deck.md)): the two-axis navigation engine (`hooks/useSlideNavigation.ts` — `currentIndex` + `subStep` fragments), keyboard nav, the `SlideContainer` transition, a generalized `DeckChrome`, a toggleable `PresenterNotes` overlay (press **N**), an inline accent-tag parser (`rich.tsx` — `<a1>/<a2>/<em>`), the typed `data/slides.ts` model, `data/sections.ts`, seven generic scenes (title, statement, data, quote, twocol, image, code), and the `index.css` token scaffold. **The engine is proven to build** — keep `hooks/`, `components/core/`, and the token *names* as-is.

### A3 — Skin it: fill the token VALUES from `04`

In `src/index.css`, replace the placeholder token **values** with the block from `04-slides-style.md` — the 8 `--talk-*` palette hex values, the two accent-dim tints (`--accent-dim` / `--accent-2-dim`), and the three `--talk-font-*` families. **Leave the token names, the 16:9 stage math, the `cqw/cqh` type scale, and the scene shells unchanged** — scenes reference names only, so re-skinning is values-only. Then mirror the font choice in `index.html`: replace the Google Fonts `<link>` with the exact one from `04` so the families named in `index.css` are the ones loaded. **Mismatched names = a silent system-font fallback = a slop tell.** Decide what each of the two accents *means* (per `04`) and keep it consistent everywhere.

### A4 — Translate `02` into the data model + scenes

Edit `src/data/slides.ts`: one entry per slide, **assertion title verbatim from `02`**, `steps` = the fragment count from `02`'s motion plan, the layout mapped to a scene `type`. Map `02`'s layout classes onto scene components (`talk-statement` → `statement`, `talk-data` → `data`, `talk-quote` → `quote`, `talk-2col` → `twocol`, `talk-fullbleed` → `image` or a world scene, `talk-title` → `title`). Set `data/sections.ts` to the talk's narrative arc. Respect the per-slide word budget (Mayer redundancy — don't dump the spoken sentence on screen). Use `<a1>/<a2>` for the meaning-mapped accent words; reveal sequential points by gating on `subStep` (see `StatementScene`) following `02`'s reveal intent — don't invent builds. Sanity-check that `slides.length === N`.

### A5 — Build the world (the differentiator)

This is where the deck stops looking generic. In `src/components/world/` (and new scene types in `slides.ts` + the `App` dispatcher), build the metaphor: **hand-built animated SVG primitives** that read the `--talk-*` CSS vars and react to `subStep`/state (the throne whose occupant flips; a map marker that travels), plus any world scenes, plus the `DeckChrome` `badge`. Keep the data/code scenes clean and editorial. **Every visual is hand-built SVG or real photography — never generated or stock imagery.** The how-to, with the GoT and Pokémon decks as worked examples, is in [`references/react-deck.md`](references/react-deck.md).

### A6 — Lift the cue cards from `03` into the notes

For each slide, lift its cue card from `03-delivery-plan.md` **verbatim** into that slide's `notes` (`{ cues: [...], line?: '...', memorable?: true }`) in `slides.ts`, keeping the pace/pause/emphasis marks intact. Put the hand-written opening/closing line in `line`. Mark the memorable-moment slide `memorable: true`. The overlay (press **N**/**S**) shows them in the same tab — it is a presenter aid, toggled off for the take.

### A7 — Preview and verify

```bash
cd "$DECK" && npm run dev          # → http://localhost:5173   (→/Space next, ← back, N notes, Home restart)
```

Arrow through every slide. Check: fonts loaded (not a fallback), accents read as `04` designed, no slide overflows the stage, fragments land, the world reads, and the cue cards show under **N**. Then verify the build is real:

```bash
cd "$DECK" && npm run build        # tsc + vite build → dist/   (must succeed)
```

- **`npm run build` succeeds** (typecheck + bundle). A failing build ships nothing.
- **Slide count == `N`** (`slides.length`).
- **The `04` tokens are actually applied** (the palette/fonts render, not the placeholder defaults).
- **Keyboard nav + the presenter overlay work**, fragments reveal in order.
- **No AI-image / stock / generated assets** anywhere — every visual hand-built.

`dist/` is a self-contained static site — deploy it to any host. The deck is the artifact; for a recorded talk, screen-record the running app.

---

## Process B — FALLBACK: reveal.js → vector PDF

Use **only** when a PDF is the deliverable (see the decision gate). This is the previously-default path, unchanged and still verified.

### B1 — Scaffold the reveal deck

```bash
SKILL=/Users/rstagi/dotfile/.claude/skills/talk-slides-build/assets
DECK=<working-dir>/deck

mkdir -p "$DECK/theme"
cp -R "$SKILL/starter/." "$DECK/"
cp "$SKILL/theme/talk.css"            "$DECK/theme/talk.css"          # base theme, unchanged
cp "$SKILL/theme/accents.example.css" "$DECK/theme/accents.css"       # NOTE the rename → accents.css
cd "$DECK" && npm install                                            # reveal.js 5.2.1 + Playwright
```

### B2 — Author + skin + export

- **Translate `02` into `slides.md`** (reveal Markdown: `---` between slides, `<!-- .slide: class="talk-*" -->` per slide, assertion titles verbatim, `<!-- .element: class="fragment" -->` for reveals). Full syntax in [`references/revealjs-guide.md`](references/revealjs-guide.md). **Collapse stepped code highlights to a single range before export** or each step prints as its own page.
- **Skin:** overwrite `deck/theme/accents.css` with `04`'s token block; match the Google Fonts `<link>` in `index.html`. Token doctrine in [`references/theming-composability.md`](references/theming-composability.md).
- **Lift `03`'s cue cards into each slide's `Note:` block**, verbatim.
- **Preview:** `npm run dev` (S = speaker view). **Export:** `npm run export:pdf` → `deck/deck.pdf` (Playwright + Chrome, `?print-pdf`). Recipe + troubleshooting in [`references/pdf-export.md`](references/pdf-export.md). **Publish:** `npm run build` → `public/`.

### B3 — Verify the PDF (don't trust, check)

```bash
cd "$DECK"
pdfinfo deck.pdf | grep Pages     # Pages == N
pdffonts deck.pdf                  # every font row emb=yes
pdftotext deck.pdf - | head        # real words → text is selectable, not rastered
```

Page count must equal `N` (a higher count = a stepped highlight/fragment exported as multiple pages → collapse the range / confirm `pdfSeparateFragments: false`). `emb=yes` on every font (else the Google Fonts `<link>` didn't match `04` — fix it or self-host). Real text from `pdftotext` confirms vector/selectable. This vector-and-selectable rule and the poppler checks are the shared PDF QA — see [`references/pdf-export.md`](references/pdf-export.md).

---

## Output

**Default (React):**

```
deck/
├── index.html              # Google Fonts <link> matches 04; <title> = talk name
├── src/
│   ├── data/slides.ts       # titles from 02 (verbatim), notes from 03 (verbatim)
│   ├── data/sections.ts     # the narrative arc
│   ├── index.css            # token VALUES from 04; names/stage/scale unchanged
│   ├── components/core/      # the engine (nav, chrome, notes, transitions) — unchanged
│   ├── components/scenes/    # the layouts
│   └── components/world/     # the per-talk world (hand-built SVG, world scenes)
├── dist/                    # `npm run build` — self-contained static site
└── README.md
```

**Fallback (reveal/PDF):** the reveal `deck/` (`index.html`, `slides.md`, `theme/talk.css`+`accents.css`, `serve.mjs`, `export-pdf.mjs`, `build.mjs`) **plus a verified `deck.pdf`** (pages == N, emb=yes, selectable).

Tell the user the absolute path to `deck/`, the run command (`npm run dev`), and — on the PDF path — the verified `deck.pdf` page count and (if built) the `public/`/`dist/` site path. Don't paste slide bodies into chat.

## Keep it human

Run **Gate C** from [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) against the *rendered* deck (what shows on screen), both the structure and the look bullets, since this is where they hit pixels:

- **Structure:** one idea per slide; sentence-assertion titles intact; the 3-second-glance and assertion-integrity tests pass.
- **Look (React):** the failure mode is **generic component/library defaults** (shadcn-out-of-the-box, an unedited AI-gradient hero, copy-paste card grids), the AI blue→purple gradient, `#1e1e2e`, a default/AI-era font; the 60-30-10 palette holds and is meaning-mapped + accessible; no AI/stock imagery; each chart has one accent on its takeaway; squint/back-row legibility survives. (On the PDF path the look failure mode is instead "the default reveal theme.")

**4+ tells = redesign, not a patch.** A **look** problem routes to [`talk-slides-style`](../talk-slides-style/SKILL.md); a **structure** problem routes to [`talk-slides-content`](../talk-slides-content/SKILL.md).

Apply **Gate E** to your own process: you used the bundled engine and authored the world by hand — you did **not** auto-generate a layout and ship it unedited; titles came verbatim from `02`, accents as authored from `04`, cue cards verbatim from `03`; every number traces to the brief; no fabricated chart values; no generated/stock images. Apply **Gate B** to any prose you write yourself (a README tweak): no slop.

The shared bibliography for every claim about the React deck, reveal.js, PDF export, and slide design is [`../talk/references/sources.md`](../talk/references/sources.md).

## Reference files

- [`references/react-deck.md`](references/react-deck.md) — **the default path.** The bundled engine tour, the two-axis `subStep` model, the token/`cqw` scaffold, the seven generic scenes, the presenter overlay, and **how to build the per-talk world** (hand-built SVG primitives, world scenes, the chrome badge) — with the GoT and Pokémon decks as worked examples.
- [`references/theming-composability.md`](references/theming-composability.md) — the renderer-agnostic `--talk-*` token system and the meaning-mapped 60-30-10 accent doctrine + accessibility checks (shared by both paths), plus the reveal-specific theming mechanics (PDF path).
- [`references/revealjs-guide.md`](references/revealjs-guide.md) — **PDF path.** reveal.js Markdown/config: `---`/`--`/`Note:`, `<!-- .slide: -->`/`<!-- .element: -->`, `data-background-image`, fragments, code fences with `[lines]`, layout classes, plugins, presenter mode.
- [`references/pdf-export.md`](references/pdf-export.md) — **PDF path + shared PDF QA.** The verified `npm run export:pdf` recipe, the manual Chrome `?print-pdf` route, decktape, offline self-hosted fonts, the vector/selectable rule, and the poppler verification block.

## Handoff

| Direction | Skill | What flows |
| --- | --- | --- |
| ← reads | [`talk-slides-content`](../talk-slides-content/SKILL.md) | `02-slides-content.md` — assertion titles (verbatim), layout class per slide, proving visual, chart specs, slide count `N`, memorable-moment slide |
| ← reads | [`talk-slides-style`](../talk-slides-style/SKILL.md) | `04-slides-style.md` — paste-ready `--talk-*` token block, Google Fonts `<link>`, per-class token roles, chart-takeaway accent |
| ← reads | [`talk-delivery`](../talk-delivery/SKILL.md) | `03-delivery-plan.md` — per-slide cue cards (keyed by slide number) lifted into the deck's per-slide speaker notes |
| → hands to | [`talk-refine`](../talk-refine/SKILL.md) | `deck/` (the built deck — React by default) and, **only if produced**, `deck.pdf` — for the final Gate A–E pass and cross-artifact reconciliation (assertion titles == rendered titles == cue-card keys; rendered tokens == `04`; rendered slide count == `N`; if a PDF was exported, pages == `N`) |
