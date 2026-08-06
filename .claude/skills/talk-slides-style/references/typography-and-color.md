# Typography & color — how to fill the theme tokens

This file is the detail behind Step 2 of the skill. The output is concrete: a typeface decision
mapped to `--talk-font-display / --talk-font-body / --talk-font-mono`, and a six-swatch palette
mapped to the six neutral tokens plus the two **meaning-mapped** accents — every one passing the
accessibility checks below. Sources are in
[`../../talk/references/sources.md`](../../talk/references/sources.md).

## Typography

### Butterick's presentation rules (Practical Typography)

Matthew Butterick's presentations chapter is the working baseline:

- **Don't use pure white on pure black.** It vibrates. Start body text around 50% gray on the dark
  field and brighten only until it's comfortably legible. The theme already does this: `--talk-ink`
  is a warm off-white (`#ece3d4`), not `#fff`, and `--talk-ink-bright` (`#fbf6ec`) is reserved for
  headings. Keep that two-step ink hierarchy when you pick a palette.
- **One base size for 12–15 lines.** Pick a body size and hold it across the deck. The theme's
  `--r-main-font-size` is 34px; that is the floor, not a thing to shrink per slide.
- **Avoid centered text.** Centered ragged blocks are hard to scan and read as "default template."
  The theme is left-aligned with a 4% margin by design. Don't fight it.
- **Drop sub-line bullets.** If a point fits on one line, it isn't a bullet — it's a line. The
  theme styles `ul` with a thin accent rule marker, not a dot or emoji, precisely so short lists
  don't look like a corporate agenda.

Depth references when you want them: Ellen Lupton, *Thinking with Type*; Robert Bringhurst,
*The Elements of Typographic Style*.

### Choosing the typeface (the anti-default rule)

**Do not ship Inter, Roboto, Arial, or Times by reflex.** Inter/Roboto are the AI-era unconscious
defaults; Arial/Times are the system defaults. Any of them, chosen without a reason, is a Gate C
tell. Pick **one typeface with real character** and at most **one pairing**.

- A pairing is usually **display serif + body sans** (or the reverse). The theme default —
  **Fraunces** (a high-contrast "old-style with wonk" display serif with optical sizing) for display
  + **Spline Sans** for body + **JetBrains Mono** for code — is a deliberate, distinctive starting
  point, not a default to leave unexamined. Change it when the talk's tone wants something else, but
  change it *to* something with character, not back to Inter.
- Map your three choices to the tokens:
  - `--talk-font-display` → headings, `talk-statement`, `talk-quote`, the title `h1`.
  - `--talk-font-body` → body, list items, `talk-data` headlines (the theme deliberately sets data
    headlines in the body face so charts read as analysis, not rhetoric).
  - `--talk-font-mono` → the kicker, section numbers, code, slide numbers, footer. Mono is doing
    *structural* work in this theme (kicker letter-spaced caps, tabular section numbers) — keep a
    mono with a strong, even rhythm.
- Tone guide: geometric/grotesque sans = technical, modern, neutral. Humanist sans = warm,
  approachable. High-contrast serif = editorial, considered, premium. Slab/mono display = systems,
  engineering, a little brutalist. Match the *talk's* feeling, then commit.
- Practicalities: free + web-loadable (Google Fonts / open licenses) so the build's headless PDF
  export embeds them; a variable font with weights 300–700 gives hierarchy from one family; check
  the figures are **tabular** if you show numbers.

### Type-scale, letterspacing, measure

- **Hierarchy by size, weight, case — not by color.** Color is reserved for meaning (the accents).
- **Letterspacing:** tighten large display (the theme uses `-0.015em` on headings); *add* 12–22%
  tracking to all-caps labels (the kicker uses `0.22em`) — all-caps without tracking looks cramped.
- **Measure (line length):** ~45–75 characters. The theme caps body at `34ch` and headings at
  `22ch` for exactly this. Respect those caps; don't widen text to fill the slide.
- **Line spacing:** ~120–145% for body (theme: 1.4); tight (~1.05) for display headlines.
- Use real curly quotes and a real em/en dash, never straight quotes or `--`.

## Color

### 60-30-10 and the meaning-mapped accents

Allocate roughly **60% dominant (the background/neutral field) / 30% secondary (ink + surfaces) /
10% accent**. The accent is a *pointer*, spent sparingly on the one thing that matters on a slide
(Knaflic). The moment everything is accented, nothing is.

This theme gives you **two** accents, and they are **meaning-mapped** — each stands for a concept in
*this* talk, used consistently so the audience learns the code:

- `--talk-accent` — the subject / the "now" / the thing under the spotlight (theme convention).
- `--talk-accent-2` — the contrast / the "what-could-be" / the other side of the argument.

Examples (steal the pattern, not the colors): migration talk → accent = legacy system, accent-2 =
new system. Perf talk → accent = slow/before, accent-2 = fast/after. Risk talk → accent = the
threat, accent-2 = the mitigation. **Write the mapping down in the spec** ("vermilion = the manual
process we're killing; teal = the automated one") so the build keeps it and every chart obeys it.

The six neutral tokens carry the 60-30-10 field:

| Token | Role | Theme default | Constraint |
| --- | --- | --- | --- |
| `--talk-bg` | the 60% field | `#17130f` warm near-black | Never `#1e1e2e` cold; never a blue→purple gradient |
| `--talk-ink` | body text | `#ece3d4` | ≥4.5:1 on bg |
| `--talk-ink-bright` | headings, emphasis | `#fbf6ec` | ≥4.5:1 on bg |
| `--talk-muted` | captions, numbers, de-emphasis | `#8a7f6d` | ≥4.5:1 on bg for any real text |
| `--talk-rule` | hairlines | `#3a3128` | ≥3:1 vs bg only if load-bearing |
| `--talk-surface` | code blocks, cards | `#211b15` | text on it still ≥4.5:1 |

You can flip the whole deck warm-light (the commented block in `accents.example.css`) when the talk
wants daylight — re-run every contrast check after flipping.

### Accessibility as a hard constraint (not a nicety)

Run **all** of these before locking the palette; record the results in the spec:

1. **WCAG contrast** — body/ink-on-bg ≥ **4.5:1**; large text (≥24px bold / ≥30px) and graphical
   objects ≥ **3:1**. Check `--talk-ink`, `--talk-ink-bright`, `--talk-muted`, and each accent
   *as used* (accent-on-bg for an emphasized word; ink-on-surface for code). Tool: WebAIM Contrast
   Checker.
2. **Colorblind-safe accent pair.** ~8% of men have a red-green deficiency; a red/green
   before-after coding is invisible to them. Prefer an **Okabe-Ito** pair (the Color Universal
   Design 8-hue set, popularized by Bang Wong, *Nature Methods* 2011) or a **ColorBrewer**
   colorblind-safe pair. **Blue + orange is the safest two-accent pair.** For sequential data use
   **viridis / cividis**. Tool: Coblis / Color Oracle simulators.
3. **Greyscale survival.** Desaturate the whole palette. The two accents must still be
   distinguishable (different *lightness*, not just hue) — because projectors are bad, rooms are
   bright, and some people see in greyscale. If they collapse, push their lightness apart or add a
   second channel (shape, position, label).
4. **Not an AI gradient.** Confirm the field is a real flat (or barely-textured) warm/neutral, not
   the blue→indigo→lavender diagonal that screams auto-deck.

The theme defaults (warm vermilion `#e8643c` + deep teal `#43b4a0` on `#17130f`) are a deliberately
distinctive, mostly-safe starting pair — but they are *defaults*. Pick colors that mean something
for this talk, then push them through the four checks above.

### Tool list

WebAIM Contrast Checker (WCAG ratios) · Viz Palette (palette-level contrast + colorblind preview) ·
Coblis / Color Oracle (colorblind simulation) · the Okabe-Ito CUD palette and ColorBrewer
(colorblind-safe categorical sets) · viridis / cividis (perceptually-uniform sequential) · plus a
plain greyscale export as the final gut check.

## Layout: C.R.A.P., the grid, rule-of-thirds

- **Grid & alignment.** The theme is a left-aligned single column with a 4% margin and short measure
  caps. State one alignment rule in the spec and hold it ("everything hangs off the left margin;
  headlines top-aligned, body below"). `talk-2col` is the only split (a 1fr/1fr grid) — use it for
  genuine comparisons (before/after, claim/counter), not to cram two ideas onto one slide.
- **Contrast & repetition** (C.R.A.P.): big size jumps between display and body; repeat the type
  scale and accent meaning on every slide so the deck is one object.
- **Proximity:** group with whitespace, not boxes. The theme avoids card-grids on purpose (they're
  an AI-deck tell) — a label sits *next to* its element, not in a bordered card.
- **Rule of thirds:** in `talk-fullbleed`, the theme drops text to the lower third over a legibility
  scrim; place the photo's subject / leading lines so the eye travels *toward* that text.
