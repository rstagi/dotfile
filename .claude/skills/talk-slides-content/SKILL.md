---
name: talk-slides-content
description: |
  Decide the structure of a conference talk's deck — break the brief into one-idea-per-slide, write a complete-sentence assertion title for each (Alley assertion-evidence), choose a layout class per slide, name the visual that proves each assertion, set the per-slide word budget, declutter every chart to one message, plan the motion, and pin the memorable moment to a slide — and write it as a slide-content spec the look-designer and deck builder work from. Use whenever the user has their talk content sorted and asks to "structure my slides", "what goes on each slide", "turn my brief into a slide plan", "write assertion titles", "lay out the deck's content", "plan the slides before styling", or invokes `/talk-slides-content`. Trigger on phrases like "plan my slides", "what's the visual for each point", "give me a slide structure spec", "one idea per slide", "make my titles real sentences", "spec the charts" — even if the user does not say "skill" or "talk-slides-content" by name. This skill writes the slide-content spec (02-slides-content.md); it does not choose fonts or colors (talk-slides-style does that) or build the deck (talk-slides-build does that).
allowed-tools:
  - Read
  - Write
  - WebSearch
  - WebFetch
---

# talk-slides-content — structure the deck, one assertion per slide

This is stage 2 of the talk pipeline. It reads the content brief (`01-content-brief.md`) and writes a
**slide-content spec** (`02-slides-content.md`): the deck broken into one-idea slides, each with a
complete-sentence assertion title (Alley), a chosen **layout class**, a proving visual, a word
budget, a declutter spec for every chart, a motion plan, and the slide that carries the memorable
moment. It decides **what each slide says and how it is structured** — not what it looks like. Fonts,
colors, hex values, and the per-class token-role mapping belong to
[`talk-slides-style`](../talk-slides-style/SKILL.md), which reads this file next.

The spine is locked here and does not move: the brief's throughline (≤15 words), its one memorable
moment, and its named specifics (names/dates/numbers/places) carry through this stage **unchanged**.
You break the argument into slides; you do not rewrite the argument. If a slide reveals that the
argument itself is wrong, that's a loop back to [`talk-content`](../talk-content/SKILL.md) — you do
not patch the spine here.

## When to use / honest skip

Use this once the content exists — a throughline, a structure, evidence, a hook and a close. Don't
structure slides for an argument that isn't settled; you'll re-slice every slide when the content
moves.

**Honest skip path:**

- **No brief yet.** If there's no `01-content-brief.md` and the user can't tell you the throughline,
  the structure, and the memorable moment, stop and route to
  [`talk-content`](../talk-content/SKILL.md). Structuring slides for a non-existent argument produces
  pretty noise.
- **The talk has no slides on purpose.** Some of the best talks use none (Bryan Stevenson, Jobs at
  Stanford). If the brief's plan is "no deck," say so and route to
  [`talk-delivery`](../talk-delivery/SKILL.md). Don't invent slides to justify the stage.
- **A live demo or a single prop is the deck.** If the memorable moment is a terminal session or a
  physical object, spec only the one or two framing slides and hand the rest to delivery — don't pad
  it to a slide count.

## Inputs

1. **Resolve the working dir.** Default `~/talks/<slug>/`; honor a runtime override if the
   orchestrator passed one. Auto-detect an existing talk project by the presence of
   `01-content-brief.md` (or other `NN-*.md` artifacts) — write `02-slides-content.md` alongside it.
   If you can't find a working dir, ask for the slug or path before doing anything else.
2. **Read `01-content-brief.md`.** Pull out, explicitly: the throughline (≤15 words), the structure
   and its beats, every named specific (names/dates/numbers/places), the evidence list with its
   primary sources, the hook, the close, and the **memorable moment**. These are your raw material;
   you do not invent or rewrite them here.
3. **Note talk type + time.** Lightning / keynote / webinar / workshop and the minutes — this sets a
   rough slide count and how dense each slide can be (a 5-minute lightning talk is one beat per
   slide; a workshop has practice slides between teaching slides).
4. **Read the references** before specifying:
   [`references/design-principles.md`](references/design-principles.md),
   [`references/data-viz.md`](references/data-viz.md), and Gate C in
   [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md).

## Process

### Step 1 — Consume the brief, restate the spine

Do the Inputs above. At the top of your working notes, restate the **throughline** (≤15 words) and
the **memorable moment** in one line each, verbatim from the brief. Everything downstream hangs on
those two, and this spec's job is to make them *visible* and *structurally placed* — not to reword
them. If the brief is ambiguous about either, ask before you slice slides; don't guess and propagate
a wrong spine into every layout.

### Step 2 — One idea per slide

Walk the brief's structure and break it into slides — **one idea per slide**. Cramming is the #1
talk-killer (Anderson), and on slides it shows up as "three points and a chart, all at 18pt." If you
can't say the slide's one idea in a single breath, split it. Don't pad the other direction either: a
5-minute lightning talk is one beat per slide and does not need sixteen of them. Let the talk type
and time set the rough count, then hold the count honest — do not inflate to look thorough.

### Step 3 — Convert each title into a complete-sentence assertion

For every slide, write the title as a **complete-sentence assertion** (Alley assertion-evidence —
the one bullet-alternative with controlled-study support, Penn State). The headline makes a claim
that ends in a period; it is not a topic label.

- `Methodology` → `We measured p99 under synthetic load, not averages.`
- `Q3 results` → `Revenue grew, but only because two accounts carried it.`
- `Performance` → `Deleting the cleverness cut p99 from 1,840ms to 210ms.`

A topic label makes the audience guess what you'll say; a sentence assertion tells them, so their
attention is free for your evidence. It also forces *you* to know your point before the slide exists.
Keep the assertion specific and opinionated — it must read like a claim a person would defend, not a
section heading.

### Step 4 — Choose the layout class per slide (structure, not style)

Pick the **layout class** that matches the slide's structural job. This is a structural choice —
*which kind of slide this is*. The **token roles, fonts, and colors each class leans on are defined
by [`talk-slides-style`](../talk-slides-style/SKILL.md)**; do not put hex or font names here. Name
the class; style themes it.

| Layout class | Use it for (the structural job) |
| --- | --- |
| `talk-title` | the opening / closing title — kicker + big headline + byline |
| `talk-section` | a section divider — a number + a short section title |
| `talk-statement` | one huge sentence — the throughline, the turn, the memorable line |
| `talk-quote` | a pull quote with attribution |
| `talk-fullbleed` | a real full-bleed photo with text in a lower-third scrim |
| `talk-2col` | a genuine comparison — before/after, claim/counter, one idea per column |
| `talk-data` | an assertion headline + one chart |

Use `talk-statement` for rhetorical beats (Takahashi/Lessig minimalism) — the throughline, the turn,
the memorable line — not for reference material; a whole deck of statements is exhausting, a few
placed ones are unforgettable. Use `talk-2col` only for a *genuine* comparison, not to fill space
with two half-thoughts.

### Step 5 — Decide the visual that proves the assertion

For each slide, name the visual and say **how it proves the assertion** (the assertion-integrity
test, Step 8). The body is *evidence that proves the claim* — a photo, a diagram, a chart — not a
bullet list restating the headline.

- **Real photography or hand-built diagrams only.** No AI-generated images, no Corporate Memphis /
  Alegria flat illustration, no cheap stock (handshakes, lightbulb=idea). These read as filler and
  trip Gate C. If you can't source a real image for a full-bleed concept, switch the slide to a
  `talk-statement` or a hand-built diagram rather than forcing stock. Use **WebSearch / WebFetch only
  to confirm a real photo or diagram for a full-bleed concept is actually sourceable** — if it isn't,
  change the structural plan now, not at build time.
- The **color treatment** of the visual (which accent marks what) is assigned by
  [`talk-slides-style`](../talk-slides-style/SKILL.md). Here you specify the *concept* and the
  *structure* of the visual; style paints it.

### Step 6 — Set the per-slide word budget (Mayer redundancy)

Keep on-screen words short. The slide carries the **assertion + the evidence**; your **voice carries
the prose**. Do **not** put on screen the full sentence you are speaking — identical text plus speech
splits one channel against itself and reads slower than either alone (Mayer redundancy principle). For
each slide, set a word budget: a `talk-statement` is one line; a `talk-data` is the assertion
headline plus direct labels; a `talk-2col` is a few words per column. If a slide is creeping toward a
paragraph, it's a document, not a slide — cut it back or split it.

### Step 7 — Declutter every data slide to one message

For each chart, write a declutter spec per [`references/data-viz.md`](references/data-viz.md). Give:

- the **one-message assertion headline** (the sentence the chart proves — this *is* the slide's
  title; if the chart supports two messages, it's two charts);
- the **chart type after the rules**: **pie/donut → sorted horizontal bars** (length beats angle);
  **no dual axes** (split into two charts); **honest baselines** (bars start at zero; a line may use
  a non-zero baseline only when the change is the message — say so, don't manufacture a slope);
- **what to delete**: border/frame, gridlines, redundant data markers, legend, axis titles the
  assertion already states;
- **direct labels** color-matched to series (kill the legend; contiguity), placed at the end of each
  line/bar;
- **rounded numbers** ("1,840ms → 210ms", not "1,839.7ms → 210.4ms" — manufactured precision is a
  slop tell);
- **which element is the takeaway** — name it in words: "the 2024 bar is the takeaway," "the
  after-line is the focal element." You say *which* element; the **accent color that marks it is
  assigned by [`talk-slides-style`](../talk-slides-style/SKILL.md)**, so do not write a color here.

Note that charts ship as clean SVG so the chart scales crisply at any size and the accent token
colors it — never a spreadsheet screenshot. A chart that trips several anti-patterns is a redesign, and it counts toward Gate C.

### Step 8 — Motion plan: cut/fade is the default

**Cut/fade is the default.** Animation is justified only by **signaling or segmenting** (Mayer):
reveal one list item or one chart layer per click via staged reveals, or mark the one element the
audience should look at. No spins, wipes, zooms, or flying text — those are amateur / auto-deck tells.
Reserve at most one transition type for section breaks. **List exactly which slides use incremental
reveals and why** (what is revealed when, and what justifies it); every other slide is static. State
the *intent* of each reveal — the build skill turns intent into its reveal mechanism (reveal
fragments on the PDF path, or React subStep reveals by default).

### Step 9 — Pin the memorable moment to a slide

The brief named one **memorable moment** (a STAR moment, a demo, a prop, a jaw-dropping stat). Pin it
to a *specific* slide and structure that slide to land it — usually a `talk-statement` (one line), a
`talk-fullbleed` (one image), or a `talk-data` (one stat with the takeaway element pointing straight
at it). Name the slide number in the spec and say how the structure delivers the moment. This is the
spine the whole deck is built around; don't bury it on slide 14 of 14.

### Step 10 — Run the gates you own

Run the **structure slice of Gate C** and the **test gates you own** on every slide before writing:

1. **3-second glance (Duarte)** — can a viewer get the gist in ~3 seconds without reading? If they
   have to read to understand, it's a document, not a slide.
2. **Assertion-integrity (Alley)** — read the sentence-assertion headline, look at the visual: does
   the visual actually *prove* that sentence? If not, the chart is wrong or the headline is bluster —
   fix one.

The **structure bullets of Gate C** are yours: one-idea-per-slide, sentence-assertion titles, the two
tests above. The **squint / back-row legibility test and the visual tells** (AI gradient, default
fonts, palette accessibility, AI-image artifacts, rainbow charts, the default theme) belong to
[`talk-slides-style`](../talk-slides-style/SKILL.md) — run the structure slice here and say so. The
accumulation rule still applies, and Gate C is the one gate set at four: **4+ structure flags in Gate C = rewrite, not patch.**

### Step 11 — Write `02-slides-content.md`

Write the file using the template below. It carries the spine unchanged at the top, the slide-by-slide
table, the chart-message specs, the motion plan, and the memorable-moment slide. **No accents.css, no
fonts, no hex** — those are style's output.

## Output: `02-slides-content.md`

The skill always produces this file. Template:

```markdown
# Slide content — <talk title>

**Throughline (from the brief, unchanged):** <≤15 words>
**Memorable moment (from the brief, unchanged):** <one line> → lands on **slide N**.
**Talk type / time / slide count:** <e.g. keynote / 18 min / ~16 slides>

## Slide-by-slide

| # | Layout class | Assertion title (complete sentence) | Proving visual + how it proves the assertion | Staged reveals / notes |
| --- | --- | --- | --- | --- |
| 1 | `talk-title` | <title line> | kicker + big headline + byline; opener from the brief | — |
| 2 | `talk-statement` | <the throughline, as one sentence> | one line; the spine, stated | — |
| … | … | … | … | … |

## Chart-message specs (per data slide)

- **Slide N (`talk-data`):** message = "<assertion>"; type = <sorted bars / line / dumbbell / slope
  / small multiples>; delete <border, gridlines, legend, axis titles>; direct labels on <series>;
  values rounded to <…>; **takeaway element = <which bar/point/line>**; ships as SVG.
  *(Color/accent for the takeaway is assigned by talk-slides-style.)*

## Motion plan

- Default: cut/fade. Staged reveals on slides <N, M> — <intent: what is revealed when, and why
  (signaling/segmenting)>. No other motion. All other slides static.

## Memorable-moment slide

- **Slide N** (`talk-…`): <how the slide's structure lands the moment>.

## Notes for talk-slides-style

- Layout classes used: <list>. Each chart's takeaway element is named above (assign the accent).
- (This spec sets structure only — no fonts, colors, or hex.)
```

## Keep it human

Run the **structure slice of Gate C** from
[`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) across the whole deck
before finishing: one-idea-per-slide, sentence-assertion titles, the 3-second-glance and
assertion-integrity tests. **4+ structure flags = rewrite, not patch** (Gate C's threshold is four, not three). The squint/legibility test
and the visual tells (AI gradient, default fonts, AI-image artifacts, rainbow charts, the default
theme) are run by [`talk-slides-style`](../talk-slides-style/SKILL.md) and re-run on the rendered
deck by [`talk-slides-build`](../talk-slides-build/SKILL.md) — don't duplicate them here.

Also apply **Gate B** to your own prose in the spec: the assertion titles and the proving-visual
notes must read as specific and opinionated, not slop — no "leverage", no "it's not just X, it's Y",
no decorative rule-of-three, no bookended summary. The opening title slide and the closing slide
carry the talk's hand-written first and last lines from the brief; keep them intact (spine).

## Reference files

- [`references/design-principles.md`](references/design-principles.md) — Presentation Zen
  (signal-to-noise, ma, picture superiority), Duarte, Alley assertion-evidence, Mayer/Sweller
  cognitive load, Lessig/Takahashi minimalism, one-idea-per-slide, the test gates this skill owns.
- [`references/data-viz.md`](references/data-viz.md) — Tufte data-ink / chartjunk / small multiples,
  the Knaflic declutter pass (step list), pie → bars, preattentive attributes, direct labels,
  one-message-per-chart.

Shared (do not duplicate):

- [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) — the structure slice
  of Gate C is this stage's anti-slop bar.
- [`../talk/references/sources.md`](../talk/references/sources.md) — the fact-checked bibliography
  behind every framework above.

## Handoff

| Direction | Skill | What flows |
| --- | --- | --- |
| Runs after | [`talk-content`](../talk-content/SKILL.md) | reads `01-content-brief.md` — throughline, structure, specifics, memorable moment, hook, close |
| Hands to | [`talk-delivery`](../talk-delivery/SKILL.md) | reads `02-slides-content.md` to align cue cards to slide order; MAY propose approved edits back to this file (never the spine) |
| Hands to | [`talk-slides-style`](../talk-slides-style/SKILL.md) | reads `02-slides-content.md` for the layout classes to theme and each chart's named takeaway element to accent |
| Also feeds | [`talk-slides-build`](../talk-slides-build/SKILL.md) | the slide table + assertions + chart specs become the deck's slides |
| Also feeds | [`talk-refine`](../talk-refine/SKILL.md) | each assertion title and slide count is cross-checked against the rendered deck |
```
