---
name: talk-slides-style
description: |
  Art-direct the look of a conference talk deck — pick ONE distinctive typeface, build a meaning-mapped 60-30-10 palette across the six neutral tokens plus two accents, run the hard accessibility checks, map a token role onto every layout class slides-content already chose, and aim the accent at each chart's takeaway — then write it as a style-spec ending in a paste-ready accents.css token block the deck builder consumes. Use whenever the structure is settled and the user asks to "style my deck", "pick fonts and colors for my talk", "art-direct the look", "choose the typography and palette", "make my deck not look AI-generated", or invokes `/talk-slides-style`. Trigger on phrases like "what colors should the deck use", "give me the accents.css", "set the type and palette", "theme the slides", "decide the look of the deck" — even if the user does not say "skill" or "talk-slides-style" by name. This skill writes the style spec (04-slides-style.md); it does not author assertion titles, choose layout classes, or write chart declutter logic (talk-slides-content does that), and it does not build the deck (talk-slides-build does that).
allowed-tools:
  - Read
  - Write
  - WebSearch
  - WebFetch
---

# talk-slides-style — decide the look, hand the build a paste-ready theme

This is stage 4 of the talk pipeline. It reads the content brief (`01-content-brief.md`), the
structure spec (`02-slides-content.md` — the layout class per slide and which element is each
chart's takeaway), and the delivery plan (`03-delivery-plan.md`, for context). It writes
`04-slides-style.md`: a house style — one distinctive typeface, a two-accent meaning-mapped palette,
a grid rule — plus a token-role note for every layout class and the accent assignment for each
chart, ending with a paste-ready `accents.css` token block + Google Fonts link the deck builder
([`talk-slides-build`](../talk-slides-build/SKILL.md)) consumes — by default a custom React deck
(reveal.js → vector-PDF is the fallback path), which reads the same token block verbatim.

You own the LOOK only. The argument is locked upstream: the **spine** — the throughline (≤15 words),
the ONE memorable moment, and the named specifics (names/dates/numbers/places) — was decided in
[`talk-content`](../talk-content/SKILL.md) and flows through here unchanged. The **structure** — slide
count, one-idea-per-slide, the assertion titles, which layout class each slide uses, the proving
visual, the chart message/type/declutter — was decided in
[`talk-slides-content`](../talk-slides-content/SKILL.md). You are deciding how all of that *looks*:
fonts, hex values, token roles, the accent that marks each takeaway. You do not author titles, pick
layout classes, or rewrite the declutter logic — if you find yourself wanting to, that is a loop back
to slides-content, not an edit here.

## When to use / honest skip

Use this once the structure exists — a slide count, assertion titles, a layout class per slide, the
chart takeaways named. Styling a deck whose structure is not settled means re-theming every slide
when the layout classes move.

**Honest skip path:**

- **No `02-slides-content.md` yet.** If the structure spec does not exist — no slide count, no
  layout classes, no chart takeaways — stop and route to
  [`talk-slides-content`](../talk-slides-content/SKILL.md). You cannot map token roles onto layout
  classes that have not been chosen, or aim an accent at a takeaway no one has named. Picking colors
  for a non-existent structure produces pretty noise.
- **The talk has no deck on purpose.** Some of the best talks use none (Bryan Stevenson, Jobs at
  Stanford). If the plan is "no slides," there is nothing to style — say so and route to
  [`talk-delivery`](../talk-delivery/SKILL.md). Do not invent a palette to justify the stage.

## Inputs

1. **Resolve the working dir.** Default `~/talks/<slug>/`; honor a runtime override if the
   orchestrator passed one. Auto-detect an existing talk project by the presence of
   `02-slides-content.md` (and the earlier `NN-*.md` artifacts) — write `04-slides-style.md`
   alongside it. If you cannot find a working dir, ask for the slug or path before doing anything
   else.
2. **Read `01-content-brief.md`.** Pull the throughline (≤15 words), the talk's tone, and the named
   specifics — the tone drives the typeface choice and the meaning the accents map onto. You are not
   re-deciding any of this; you are reading it so the look serves it.
3. **Read `02-slides-content.md`.** This is your primary input. Pull, explicitly: the slide count;
   the **layout class per slide** (you theme these, you do not choose them); which slides are
   `talk-data`; and for each `talk-data` slide, **which element slides-content named as the
   takeaway** (the focal bar/line/point). That focal element is what your accent will mark.
4. **Read `03-delivery-plan.md`** for context only. Any delivery-driven content edits are already
   applied to `02-slides-content.md` upstream — you read the latest `02`, not the delivery plan's
   proposed diffs. Note which slide carries the memorable moment so your accent does not fight it.
5. **Read the references** before specifying:
   [`references/typography-and-color.md`](references/typography-and-color.md) (yours), and Gate C in
   [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md). For the layout-class
   definitions and the chart takeaways you are theming, the source of truth is
   [`../talk-slides-content/SKILL.md`](../talk-slides-content/SKILL.md) and its
   [`references/data-viz.md`](../talk-slides-content/references/data-viz.md).

## Process

### Step 1 — Consume the structure, resolve the working dir

Do the Inputs above. Restate the throughline and the talk's tone in one line each at the top of your
working notes — the type and palette exist to serve the tone, not to look nice in isolation. List the
layout classes in play and flag every `talk-data` slide with its named takeaway; that list is the
spine of Step 3.

### Step 2 — Set the house style (the art direction)

Decide the look once, apply it everywhere. Three sub-decisions, each mapped to theme tokens — detail
in [`references/typography-and-color.md`](references/typography-and-color.md):

- **Typeface.** Pick ONE distinctive typeface with a pairing of at most one. **Not Inter / Roboto /
  Arial / Times chosen by reflex** — those are Gate C tells. Map your choices to
  `--talk-font-display` (headings, statements, quotes), `--talk-font-body` (body, data headlines),
  `--talk-font-mono` (kicker, section numbers, code, footer). The theme ships Fraunces / Spline
  Sans / JetBrains Mono as a deliberate, distinctive default — change it *to* something with
  character if the talk's tone wants that, not back to a default. Prefer web-loadable open fonts (Google Fonts) so
  the deck loads them reliably across browsers/exports.
- **Palette, 60-30-10, meaning-mapped.** The six neutral tokens carry the 60% field and the ink:
  `--talk-bg` (the field — warm/neutral flat, **never a blue→purple gradient, never `#1e1e2e`**),
  `--talk-ink`, `--talk-ink-bright`, `--talk-muted`, `--talk-rule`, `--talk-surface`. The two
  **accents are meaning-mapped** — assign each a concept in *this* talk and write the mapping down:
  `--talk-accent` = the subject / "now" / spotlight; `--talk-accent-2` = the contrast /
  "what-could-be". (e.g. "vermilion = the manual process we're killing; teal = the automated one".)
  The concepts come from the brief and the structure; you are choosing the hex, not the meaning.
  Spend the accent like money — ~10%, on the one thing that matters.
- **Accessibility checks (hard constraints, record results in the spec):** WCAG contrast ≥4.5:1
  text / ≥3:1 large+graphical; a colorblind-safe accent pair (Okabe-Ito / ColorBrewer; blue+orange
  is safest; viridis/cividis for sequential); survives a greyscale check (accents distinguishable by
  *lightness*, not just hue); **no AI blue→purple gradient, never `#1e1e2e`**. If any check fails,
  pick different colors — do not ship and hope.
- **Grid / alignment rule.** State one and hold it. The theme is left-aligned, single column, 4%
  margin, short measure caps. Do not center everything (a tell) and do not fight the left edge.

### Step 3 — Map a token role onto every layout class, and aim the accent at each chart takeaway

slides-content already chose a layout class per slide. Your job is to say **which tokens each class
leans on** — so the build applies the right ink, the right accent, the right font weight per class.
Walk the classes in play and record the role map:

| Layout class | Token roles it leans on |
| --- | --- |
| `talk-title` | display font; `--talk-accent` on the kicker; `--talk-muted` byline; `--talk-ink-bright` headline |
| `talk-section` | mono `--talk-muted` tabular number; `--talk-accent` top rule; `--talk-ink-bright` title |
| `talk-statement` | display font, large; `--talk-accent` on the `<em>`; rest in `--talk-ink-bright` |
| `talk-quote` | display font; `--talk-accent` quote mark; `--talk-muted` attribution |
| `talk-fullbleed` | bg image + the theme's scrim; `--talk-ink-bright` text; `--talk-accent` only if one word needs it |
| `talk-2col` | the two **meaning-mapped accents**, one per column (accent = concept A, accent-2 = concept B) |
| `talk-data` | body-font headline in `--talk-ink-bright`; `--talk-muted` for non-focal series; the takeaway in `--talk-accent` |

Only theme the classes the deck actually uses — do not pad the table to all seven. For the
**meaning-mapped pair on `talk-2col`**, state which accent maps to which column so the build does not
guess.

Then, for **every `talk-data` slide**, assign the accent to the focal element slides-content named:
the takeaway bar/line/point gets `--talk-accent`; every other series goes `--talk-muted` (or a tint
of the rule). One accent per chart, on the one element the audience should look at. You are colouring
the takeaway slides-content already identified — you are not re-deciding the chart's message, type,
or what gets deleted (that is the declutter spec, which lives in `02` and
[`../talk-slides-content/references/data-viz.md`](../talk-slides-content/references/data-viz.md)).

### Step 4 — Run the LOOK slice of Gate C + the legibility test

Before writing, run the LOOK bullets of **Gate C** from
[`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) across the whole deck —
this stage owns the look slice (slides-content runs the structure bullets; the build re-runs Gate C
against the rendered deck). Self-check these tells:

1. **No AI blue→purple gradient**, and never `#1e1e2e` (the default-theme background).
2. **No default / AI-era fonts** chosen by reflex (Inter / Roboto / Arial / Times).
3. **Palette meaning-mapped + accessible** — each accent maps to a written concept; WCAG, Okabe-Ito,
   greyscale all pass.
4. **No AI-image artifacts** in any sourced image (warped hands, garbled text, waxy skin, impossible
   shadows) — flag it for slides-content to reshoot if it slipped through; you do not source images,
   but a styled deck must not ship one.
5. **Charts get one accent on the takeaway** — never a rainbow chart.
6. **Not the default theme** — the deck reads as art-directed, not as the auto-deck look.

Then the **squint / back-row legibility test**: at thumbnail size, is the focal element and the
headline still legible, and does the accent still read as *the* one thing to look at? (Text and
contrast usually fail small, not large.) **4+ tells in Gate C's visual slice = re-style, not patch.**

### Step 5 — Write `04-slides-style.md`

Write the file using the template below. End it with the **exact `accents.css` token values** and the
Google Fonts `<link>` so the build skill can paste them in with no guesswork. That block is the build
input — keep its format unchanged.

## Output: `04-slides-style.md`

The skill always produces this file. Template:

```markdown
# Slides style — <talk title>

**Throughline (from the brief, unchanged):** <≤15 words>
**Tone (from the brief):** <one line — what the look must feel like>
**Slide count / layout classes in play:** <e.g. 16 slides · title, section, statement, 2col, data>

## House style

### Type → token map
- `--talk-font-display`: <Family> — <why it fits the talk's tone>
- `--talk-font-body`: <Family>
- `--talk-font-mono`: <Family>
- Type scale / alignment rule: <one base size; left-aligned; measure caps; all-caps tracking>

### Palette → token map (hex + meaning + accessibility)
| Token | Hex | Role / meaning | Contrast & colorblind note |
| --- | --- | --- | --- |
| `--talk-bg` | #… | the 60% field | n/a (field) · not `#1e1e2e`, no gradient ✓ |
| `--talk-ink` | #… | body | x.x:1 on bg ✓ |
| `--talk-ink-bright` | #… | headings/emphasis | x.x:1 on bg ✓ |
| `--talk-muted` | #… | captions, numbers, non-focal series | x.x:1 on bg ✓ |
| `--talk-rule` | #… | hairlines | — |
| `--talk-surface` | #… | code/cards | ink on it x.x:1 ✓ |
| `--talk-accent` | #… | **<concept A — e.g. the old world>** | x.x:1 ✓ · Okabe-Ito ✓ · greyscale ✓ |
| `--talk-accent-2` | #… | **<concept B — e.g. the new world>** | x.x:1 ✓ · pair colorblind-safe ✓ |

Accessibility: WCAG <pass>, colorblind pair <pass>, greyscale <pass>, no AI gradient / no `#1e1e2e` <pass>.

## Layout-class token roles
(slides-content chose the class per slide; this maps which tokens each class leans on)
- `talk-title`: <token roles>
- `talk-statement`: <token roles>
- `talk-2col`: accent = <concept A column>, accent-2 = <concept B column>
- `talk-data`: headline in `--talk-ink-bright`; non-focal in `--talk-muted`; takeaway in `--talk-accent`
- … (only the classes this deck uses)

## Chart-accent assignments
(the takeaway per chart was named by slides-content; this colours it)
- **Slide N (`talk-data`):** takeaway = "<the focal bar/line/point>" → `--talk-accent`; rest → `--talk-muted`.
- …

## accents.css tokens (paste-ready for the deck builder)
\`\`\`css
:root {
  --talk-bg:         #…;
  --talk-ink:        #…;
  --talk-ink-bright: #…;
  --talk-muted:      #…;
  --talk-rule:       #…;
  --talk-surface:    #…;
  --talk-accent:     #…;  /* <concept A> */
  --talk-accent-2:   #…;  /* <concept B> */
  --talk-font-display: "<Family>", <fallbacks>;
  --talk-font-body:    "<Family>", <fallbacks>;
  --talk-font-mono:    "<Family>", <fallbacks>;
}
\`\`\`
Google Fonts <link> the deck needs: <families + weights>.
```

## Keep it human

Run the **LOOK slice of Gate C** from
[`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) across the whole deck
before finishing — the slide-look tells are this stage's slice (slides-content runs the structure
tells; the build runs Gate C again on the rendered deck). **4+ tells in the visual slice = re-style,
not patch**, plus the squint / back-row legibility test. Also apply **Gate B** to your own prose in
the spec: the meaning notes and token rationale should read as specific and opinionated — name the
concept each accent maps to, say why a typeface fits the tone — not as slop. No "leverage", no
"seamless", no "it's not just X, it's Y", no decorative rule-of-three, no bookended summary. If you
catch yourself writing "a robust, modern palette," delete it and say which colour means what.

## Reference files

- [`references/typography-and-color.md`](references/typography-and-color.md) — Butterick's rules,
  type scale / letterspacing / measure, 60-30-10, WCAG, Okabe-Ito / ColorBrewer / viridis, the tool
  list, C.R.A.P. / grids / rule-of-thirds — all mapped to the theme tokens. This is the craft behind
  every choice in Step 2.

For the structure you are theming (you reference these, you do not own them):

- [`../talk-slides-content/SKILL.md`](../talk-slides-content/SKILL.md) — owns the assertion titles,
  the layout class per slide, the proving visual, the one-idea-per-slide breakdown, and the three
  structural test gates (3-second glance, squint, assertion-integrity).
- [`../talk-slides-content/references/design-principles.md`](../talk-slides-content/references/design-principles.md)
  — the design-principle craft and the three structural test gates.
- [`../talk-slides-content/references/data-viz.md`](../talk-slides-content/references/data-viz.md) —
  Tufte / Knaflic declutter logic, pie → bars, direct labels, one-message-per-chart. You colour the
  takeaway this defines; you do not write the declutter pass.

Shared (do not duplicate):

- [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) — Gate C's LOOK slice
  is this stage's anti-slop bar; Gate B applies to your own prose.
- [`../talk/references/sources.md`](../talk/references/sources.md) — the fact-checked bibliography
  behind every claim above.

## Handoff

| Direction | Skill | What flows |
| --- | --- | --- |
| Runs after | [`talk-slides-content`](../talk-slides-content/SKILL.md) | reads `02-slides-content.md` — slide count, layout class per slide, chart takeaways to theme |
| Reads for context | [`talk-delivery`](../talk-delivery/SKILL.md) | reads `03-delivery-plan.md` — any delivery-driven content edits are already applied to `02`; notes the memorable-moment slide |
| Hands to | [`talk-slides-build`](../talk-slides-build/SKILL.md) | reads `04-slides-style.md` — the deck builder (a React deck by default) consumes the token block, font link, layout-class token roles, chart-accent assignments |
| Also feeds | [`talk-refine`](../talk-refine/SKILL.md) | the shipped deck's resolved token values must match `04`; the look survives the final holistic gate pass |
