# Keep It Human — the anti-AI-slop spine

This is the shared quality bar for the whole talk suite. Every skill runs the slice that applies
to it; the orchestrator runs the relevant gate **between** stages and refuses to advance a stage
that trips 3+ flags. As the final stage, `talk-refine` re-runs all gates A–E holistically across
the finished set. Citations are short here — full bibliography in [`sources.md`](sources.md).

## The three laws

1. **Specificity is the strongest human signal.** Low information density — the absence of
   concrete names, dates, numbers, places, and lived detail — is the single most reliable "slop"
   tell measured in the literature (Shaib et al., *Measuring AI Slop in Text*, arXiv:2509.19163;
   Charlie Guo, *The Field Guide to AI Slop*). The fix is never "write better prose"; it's "add a
   real, checkable, particular thing that could only have come from you."
2. **Tells are a stack of signals, not one smoking gun.** Weight *durable* signals (cadence
   uniformity, low density, hollow conviction) over *volatile* lexical ones ("delve" is fading as a
   meme; newer models suppress em-dashes, so their **absence no longer proves human authorship**).
   No single word condemns a sentence; a pile-up does.
3. **AI prunes and critiques; the human writes the messy first draft and supplies the lived
   specifics.** Write the opening line and the closing line by hand. Verify every number against a
   real source. **Accumulation rule: 3+ flags in any gate below = rewrite, not patch.**

---

## Gate A — Thesis / content  *(owned by `talk-content`)*

- [ ] Throughline stated in ≤15 words as a **shift in understanding**, not a topic. Everything
      in the talk hangs on it; anything that doesn't is cut.
- [ ] A real, defensible **point of view** is present — the talk argues something, it doesn't just
      validate the audience. (AI defaults to false balance / "the wallpaper of consensus.")
- [ ] **≥1 concrete particular per section** (name / date / number / place / first-person scar).
      *Competitor test:* could any other speaker publish this section verbatim? If yes, it's generic.
- [ ] Every statistic **traced to a primary source** (SIFT + CRAAP); no manufactured precision
      ("improves retention by 47%"); genuine uncertainty stated plainly.
- [ ] Each analogy maps a **deep relation** (Gentner structure-mapping), sits between too-obvious
      and too-distant, and never mixes two metaphor families.
- [ ] Talk **type + time** matched: no keynote crammed into a lightning slot; a "workshop" is
      majority practice, not a lecture; an online talk re-hooks every 5–8 minutes.

## Gate B — Voice / prose  *(shared by `talk-content` and `talk-delivery`)*

- [ ] **Read it aloud.** It sounds like you talking to a respected friend — not a keynote
      performance, not a press release (Paul Graham, *Write Like You Talk*).
- [ ] **Lexical linter clean** (see list below): no delve/underscore/showcase/tapestry/realm/
      leverage/elevate/unlock/seamless/meticulous/intricate/"in today's fast-paced world"/"when it
      comes to" surviving unchallenged.
- [ ] **Structural linter clean:** no "it's not just X, it's Y"; no "from X to Y" sweep; no forced
      rule-of-three whose third item is a vague intensifier ("fast, efficient, and transformative");
      no bookended self-summary (opener restates the headline, closer restates the opener — delete
      both).
- [ ] **Cadence varies.** Read any three consecutive sentences aloud — if they breathe identically,
      break one. Em-dashes ≤1–2 (but absence ≠ human).
- [ ] **De-hedged.** Committed claims, not a wall of may/could/might/"it's worth noting".
      Productive uncertainty ("I'm not sure, but X suggests…") reads human; both false confidence
      and false hedging read AI.
- [ ] **Closing beat** is something only you could write — an unresolved question, an admission you
      were wrong, a conviction. (Highest-signal human marker, cheapest to enforce.)

## Gate C — Slide visuals  *(shared by `talk-slides-content` [structure bullets], `talk-slides-style` [look bullets], and `talk-slides-build` [rendered deck]; `talk-refine` re-runs it whole)*  — **4+ tells = redesign**

- [ ] No **AI-blue → purple/indigo/lavender diagonal gradients**; no centered-everything; no row of
      three equal-width icon cards; no emoji-as-bullets; no identical card grid with colored
      left/top borders; no single flat thin-line icon set repeated everywhere; no over-symmetry.  *[look → `talk-slides-style`]*
- [ ] No **default/system fonts** (Calibri, Arial, Times) and no AI-era defaults (Inter/Roboto)
      chosen unconsciously. One distinctive typeface with character + ≤1 pairing.  *[look → `talk-slides-style`]*
- [ ] **60-30-10 palette**, ≤3 colors + one accent, **meaning-mapped** (e.g. one colour = the old
      world, one = the promised land). WCAG ≥4.5:1 (text) / ≥3:1 (large/graphical); colourblind-safe
      (Okabe-Ito / ColorBrewer / viridis); survives a grayscale check.  *[look → `talk-slides-style`]*
- [ ] **One idea per slide**; sentence-assertion titles (not topic labels); passes the 3-second
      glance test, the squint / back-row test, and the assertion-integrity test (does the visual
      *prove* the headline sentence?).  *[structure → `talk-slides-content`]*
- [ ] **Real, full-bleed photography**; **no AI-image artifacts** (warped hands/fingers, garbled
      text-in-image, waxy skin, impossible shadows, uncanny symmetry — Kamali et al.,
      arXiv:2406.08651); no **Corporate Memphis / Alegria** flat illustration; no cheap stock
      (handshakes, lightbulb=idea, diverse-team-laughing-at-salad).  *[look → `talk-slides-style`]*
- [ ] **Charts decluttered** (no 3-D, no exploded pies, no chartjunk; pies → sorted horizontal
      bars; direct labels; one accent colour points at the takeaway). Transitions = cut/fade only;
      animation justified only by signaling/segmenting (progressive disclosure).  *[look → `talk-slides-style`]*
- [ ] **Not a default look — whatever the medium.** For the fallback path: not a stock reveal.js
      theme shipped unedited. For the default built deck (a custom React deck by default): not
      generic component-library / shadcn-default UI, not an unedited AI-gradient hero, not a
      copy-paste card grid. No layout that was auto-generated (in a tool or by a default component
      kit) and shipped untouched.  *[look → `talk-slides-style`]*

## Gate D — Delivery  *(owned by `talk-delivery`)*

- [ ] Rehearsed to **own it**, not word-perfect recitation. Trigger cards, not a teleprompter.
- [ ] **Prosody varies** (monotone is the #1 robot tell); pace varies around a ~190 wpm
      conversational baseline; strategic pauses used; filler words ≈1/min.
- [ ] Gestures are **illustrative, from thought, inside the "power sphere"**; adaptors (fidgeting,
      face-touching) eliminated.
- [ ] **Recording self-audit passes:** watch it sound-off (no sway/adaptors) and audio-only (not
      monotone; breathing and the occasional self-correction present; emotion genuinely shifts).
      Human delivery has texture; the uncanny-valley tell is delivery that hits every beat a beat
      too perfectly.
- [ ] Humor (if any) is **found in the material**, self-deprecating, relevant, culturally safe;
      ride the laugh, never step on it. Manufactured jokes are worse than none.
- [ ] **No myths taught as fact** — not Mehrabian 7-38-55 (applies only to feeling/attitude
      conflicts), not Cuddy power-posing (failed to replicate).
- [ ] **Context-correct** for in-person stage vs online (camera eye-line, +⅓ energy, lens contact,
      self-view off, 10-min resets) vs workshop facilitation (time-boxing, guide-not-hero).

## Gate E — Process  *(all skills)*

- [ ] AI was used to **research, prune, critique, and draft mechanics** — not to generate the
      finished voice or auto-build the deck.
- [ ] **Opening line + closing line written by hand.**
- [ ] **Every number verified** against a real, checkable source. No fabricated studies or quotes.
- [ ] **Accumulation rule applied:** 3+ flags in any gate above → rewrite, don't patch.
- [ ] **`talk-refine` re-runs all gates A–E holistically** across the finished set as the final
      stage, before the talk is called done.

---

## Appendix 1 — Lexical linter (flag → replace with a plain verb / cut)

`delve` · `delving` · `underscore(s)` · `showcase` · `showcasing` · `intricate` · `tapestry` ·
`realm` · `testament (to)` · `leverage` · `unlock` · `elevate` · `harness` · `foster` ·
`meticulous` · `nuanced` · `robust` · `seamless` · `comprehensive` · `crucial` · `pivotal` ·
`boast(s)` · `notably` · `moreover` · `furthermore` · `navigate the landscape`.

Phrase tells: "in today's fast-paced world", "it's not just X, it's Y", "the power of",
"game-changer", "a testament to", "when it comes to", "at the end of the day", "in conclusion",
"without further ado", "let me be clear", "here's the thing".

> These are *signals*, not proof. One survivor in a paragraph of concrete, opinionated prose is
> fine. A cluster is a rewrite. The list will date — weight the structural tells below higher.

## Appendix 2 — Structural tells (durable; weight these highest)

- **Cadence uniformity** — 3+ consecutive sentences of near-identical length/shape.
- **Hollow conviction** — universal claims with no concrete anchor ("transforms how organizations
  operate"); false balance that refuses any stance; sycophantic framing.
- **The "from X to Y" sweep** and the **"it's not just X, it's Y"** reframe as default rhythm.
- **Forced rule-of-three** where the third item is a vague intensifier.
- **Bookended self-summary** — the intro and the conclusion say the same thing with no payoff.
- **False comprehensiveness** — covering everything evenly because there's no thesis to prioritize.

## Appendix 3 — AI-image artifact categories (Kamali et al., arXiv:2406.08651)

Anatomical (hands, fingers, teeth, ears, eyes) · stylistic (waxy/oversaturated skin, plastic
sheen) · functional (garbled text/logos/UI in the image) · physics (impossible shadows,
inconsistent reflections, melted edges) · sociocultural (uncanny symmetry, off cultural detail).
Humans only catch AI images ~31% of the time on a screen — **rely on source/process verification,
not "I can tell"**, and prefer commissioned/real photography or hand-built diagrams.

## Appendix 4 — Named visual anti-patterns

- **Corporate Memphis / Alegria** — faceless, bendy-limbed, uniformly-happy flat illustration; reads
  as soulless big-tech filler.
- **The auto-deck look** — the default-output tell, in either medium. Tool output (Gamma/Canva-Magic)
  *and* a stock reveal.js theme on the fallback path: diagonal gradient, one flat font with no real
  hierarchy, emoji bullets, equal-weight icon cards, perfect symmetry. Its React equivalent on the
  default built deck: generic component-library / shadcn-default UI, an unedited AI-gradient hero,
  copy-paste card grids. If a tool, a default theme, or a default component kit generated the layout,
  redesign the title, the data-viz, and the hero image by hand before shipping.
