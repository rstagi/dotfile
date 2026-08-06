# Design principles — the why behind the slide-spec

The slide-spec is opinionated on purpose. Each rule below traces to a named source in
[`../../talk/references/sources.md`](../../talk/references/sources.md). Read this once; you don't
re-derive it per talk.

## 1. The slide is a visual aid, not a document

A deck projected on a wall and a document someone reads at their desk are two different artifacts.
The hybrid — the "slideument," text-dense slides that try to be both — fails at both jobs. Garr
Reynolds (*Presentation Zen*) frames the whole craft as **maximizing signal-to-noise**: every pixel
that isn't carrying the one idea is noise, and noise competes with *you*, the speaker. If the talk
genuinely needs a dense reference, that's a separate leave-behind PDF — never the projection deck.

Three ideas from Reynolds worth holding in your head while you spec:

- **Signal-to-noise.** Strip until removing one more thing would break the meaning. Then stop.
- **Ma (間) — negative space is active.** Empty space isn't wasted; it's what makes the one element
  land. The warm-dark theme this suite ships is built around generous margins for exactly this.
- **Picture superiority.** A concrete image is recalled better than the same idea in words (Paivio's
  dual-coding). Cite the *effect*, never the folklore numbers ("60,000× faster", "65% recall") —
  those are fabricated and listed as such in sources.md.

## 2. One idea per slide

The single most common failure (Anderson calls cramming the #1 talk-killer) shows up on slides as
"three points and a chart, all at 18pt." A slide carries one idea. If you can't say what the one
idea is in a breath, split the slide. This is also what makes the *3-second glance test* (below)
passable at all.

The slide-build theme makes this physically easier: `talk-statement` holds one sentence at 2.6em;
`talk-data` holds one assertion + one chart. The layout classes are biased toward one-idea slides.

## 3. Assertion-Evidence (Alley) — the spine of the spec

Michael Alley's *Assertion-Evidence* model (*The Craft of Scientific Presentations*) is the one
bullet-alternative with controlled-study support behind it (Penn State; state it as "statistically
significant in controlled studies," not a bare p-value). The model:

- The headline is a **complete-sentence assertion** — a claim that ends in a period — not a topic
  label. `Methodology` becomes `We measured p99 under synthetic load, not averages`. `Q3 results`
  becomes `Revenue grew, but only because two accounts carried it`.
- The body is **visual evidence that proves the assertion** — a photo, a diagram, a chart, an
  equation. Not a bullet list restating the headline.
- The speaker supplies the connective sentences **live**. The slide is the skeleton; you are the
  muscle.

Why it works: a topic label ("Performance") makes the audience guess what you'll say about it. A
sentence assertion tells them, so their attention is free to follow your evidence instead of
decoding your headline. It also forces *you* to know your point before you design the slide — you
can't write the assertion if you don't have one.

The **assertion-integrity test** (test gate 3) falls straight out of this: read the headline, look
at the visual, ask "does this visual *prove* that sentence?" If the chart doesn't support the claim,
either the chart is wrong or the claim is bluster. Fix one.

## 4. Cognitive load & multimedia learning (Mayer / Sweller)

Richard Mayer's multimedia principles and John Sweller's Cognitive Load Theory explain *why* the
rules above aren't just taste. Working memory is narrow; a slide that overloads it costs
comprehension. The principles that bite hardest for slide design:

- **Coherence** — cut extraneous material ("seductive details": the decorative photo, the cute
  animation, the logo on every slide). Each one steals a slice of attention.
- **Redundancy** — do **not** put on screen the full sentence you are speaking. Identical text +
  speech splits one channel against itself; it reads slower than either alone. Slides carry the
  *evidence* and a short assertion; your voice carries the prose.
- **Signaling** — cue where to look (an accent color on the one data point that matters, a fragment
  that reveals the relevant line). This is the *only* honest justification for animation.
- **Segmenting** — let the viewer control the pace; progressive disclosure (one fragment per click)
  segments a complex slide into digestible beats.
- **Contiguity** — put the label next to the thing it labels (direct labels on charts, not a legend
  across the slide).

"Death by PowerPoint" was coined by Angela Garber (*Small Business Computing*, April 2001); the
mechanism Garber named is exactly the redundancy + overload Mayer and Sweller measured.

## 5. Radical minimalism is a legitimate register (Takahashi / Lessig)

Some talks want the opposite of a chart: one enormous word or a single short sentence on a plain
field, advancing in tight sync with speech (the Takahashi method; Lawrence Lessig's machine-gun
style). The slides become rhetorical beats — punctuation for the voice. This suite's `talk-statement`
class is built for it. Use it for the throughline, the turn, the memorable line — not for reference
material. A whole deck of statements is exhausting; a few placed statements are unforgettable.

## 6. C.R.A.P. + the grid (Robin Williams)

Robin Williams' *The Non-Designer's Design Book* gives the four levers Reynolds calls the "Big Four":

- **Contrast** — if two things differ, make them differ a lot. Timid contrast reads as a mistake.
- **Repetition** — repeat a few visual decisions (the type scale, the accent meaning, the margin) so
  the deck feels like one object and the audience learns the code.
- **Alignment** — nothing arbitrary. Every element lines up with something. The theme is
  left-aligned with a 4% margin; respect that edge.
- **Proximity** — group related things; separate unrelated ones. Whitespace does the grouping.

Asymmetry reads dynamic and confident; dead-center symmetry reads inert (and is a named AI-deck
tell — see Gate C). Put the focal element on a rule-of-thirds intersection rather than the middle.

## The three test gates (run every slide through all three)

Every slide in the spec must pass all three. These are the operational core of the design pass.

1. **3-second glance test (Duarte).** Project it (or imagine the back row). In ~3 seconds, can a
   viewer get the gist? If they have to read to understand, it's a document, not a slide.
2. **Squint / back-row test (Reynolds).** Squint at the slide (or shrink it to a thumbnail). The
   focal element and the headline must still be legible and obviously the focus. Catches text
   that's too small (the common error — text is rarely too big) and weak hierarchy.
3. **Assertion-integrity test (Alley).** Read the sentence-assertion headline, then look at the
   visual. Does the visual *prove* the headline? If not, the slide is lying or the headline is
   filler. Fix the mismatch.

Failing any gate is a redesign of that slide, not a tweak. And the whole deck gets one pass of
**Gate C** from [`../../talk/references/keep-it-human.md`](../../talk/references/keep-it-human.md):
4+ visual tells = redesign, not patch.
