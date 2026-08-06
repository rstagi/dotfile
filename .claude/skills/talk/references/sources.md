# Sources — the fact-checked bibliography

The canon behind this suite. Every claim in the skills should be traceable here. These were
adversarially verified; **corrections and de-listings are marked** — do not silently re-introduce
the dropped attributions.

## Content & structure

- **Chris Anderson — *TED Talks: The Official TED Guide to Public Speaking*** (ISBN 9780544634497 /
  9781328710284). Throughline (≤15 words = a shift, not a topic), idea worth spreading, build from
  where the audience already is, the ~18-min rationale, four rehearsal modes, five connection tools.
  https://www.ted.com/read/ted-talks-the-official-ted-guide-to-public-speaking
  - Free condensed: *How to Give a Killer Presentation*, HBR June 2013 (R1306K).
    https://hbr.org/2013/06/how-to-give-a-killer-presentation
- **Nancy Duarte — *Resonate* / *slide:ology*** + TED "The secret structure of great talks." The
  **sparkline** (oscillate what-is ↔ what-could-be, end on new bliss + CTA, a memorable moment),
  audience-as-hero / speaker-as-mentor. https://www.duarte.com/resonate/ ·
  https://www.ted.com/talks/nancy_duarte_the_secret_structure_of_great_talks
- **Chip & Dan Heath — *Made to Stick*.** SUCCESs (Simple, Unexpected, Concrete, Credible,
  Emotional, Stories); the Curse of Knowledge. https://heathbrothers.com/made-to-stick-introduction/
- **Andy Raskin — *The Greatest Sales Deck I've Ever Seen*** (5-step strategic narrative) + companion
  "Name the Enemy." https://medium.com/the-mission/the-greatest-sales-deck-ive-ever-seen-4f4ef3391ba0
- **Kenn Adams — the Story Spine** ("Once upon a time / every day / one day / because of that /
  until finally"), popularized via Emma Coats' Pixar 22 Rules.
  https://www.aerogrammestudio.com/2013/03/22/the-story-spine-pixars-4th-rule-of-storytelling/
- **Dan Harmon — Story Circle** (You-Need-Go-Search-Find-Take-Return-Change).
  https://reedsy.com/blog/guide/story-structure/dan-harmon-story-circle/
- **Barbara Minto — SCQA / the Pyramid Principle** (analytical/structured talks).
- **Carmine Gallo — *Talk Like TED*** (emotion, novelty, memorable moments, hooks). Cite the
  book/Inc. column, **not** mirror PDFs. https://www.carminegallo.com/books/talk-like-ted/
- **Lakoff & Johnson — *Metaphors We Live By*** (concrete embodied source domains);
  **Dedre Gentner — "Structure-Mapping"**, *Cognitive Science* 7(2):155–170, 1983 (map deep
  relations, not surface features); **Hofstadter & Sander — *Surfaces and Essences***.
- **Mike Caulfield — SIFT (The Four Moves)** + **CRAAP test** (Blakeslee, CSU Chico 2004) for
  evidence vetting. https://hapgood.us/2019/06/19/sift-the-four-moves/
- **Cole Nussbaumer Knaflic — *Storytelling with Data*** (one message per chart, declutter, direct
  attention). https://www.storytellingwithdata.com/
- **Hans Rosling / Gapminder / *Factfulness*** (vivid, honest data storytelling).
  https://www.gapminder.org/
- **James Webb Young — *A Technique for Producing Ideas*** (5-step idea generation); pair with
  Koestler bisociation, de Bono, SCAMPER, biomimicry for distant-domain analogy search.
- **Workshops:** Bloom's taxonomy + ABCD objectives; Cathy Moore — *Map It* (action mapping);
  Julie Dirksen — *Design for How People Learn*; gradual release (I-do / we-do / you-do).
- **Example talks:** Jill Bolte Taylor, "My stroke of insight" (in-medias-res open + brain prop);
  Steve Jobs, Stanford 2005 (three labeled stories, rule of three, no slides).
- ⚠️ **CORRECTION:** the **mosquito-jar release demo is Bill Gates's 2009 TED talk "Mosquitos,
  malaria and education"** (https://www.ted.com/talks/bill_gates_mosquitos_malaria_and_education),
  **not** the 2015 "The next outbreak? We're not ready" talk (which has no such demo). Cite 2009.

## Slide design

- **Garr Reynolds — *Presentation Zen***. Signal-to-noise, restraint, *ma* (negative space),
  picture-superiority, the slide as visual aid not document. https://www.presentationzen.com/ ·
  https://www.garrreynolds.com/design-tips
- **Michael Alley — the Assertion-Evidence model** (*The Craft of Scientific Presentations*).
  Sentence-assertion headline + visual evidence; the one bullet-alternative with controlled-study
  support (Penn State — state as "statistically significant in controlled studies," not a bare
  p-value). https://www.assertion-evidence.com
- **Richard E. Mayer — *Multimedia Learning*** (12 principles: coherence, signaling, redundancy,
  modality, contiguity, segmenting); **John Sweller — Cognitive Load Theory**. "Death by
  PowerPoint" coined by **Angela Garber**, *Small Business Computing*, April 2001.
- **Matthew Butterick — *Practical Typography*** (presentations chapter): start type ~50% gray on
  black, one base size for 12–15 lines, avoid centered text, drop sub-line bullets.
  https://practicaltypography.com/presentations.html · depth: Ellen Lupton *Thinking with Type*,
  Robert Bringhurst *Elements of Typographic Style*.
- **Color/accessibility:** WCAG contrast (≥4.5:1 text, ≥3:1 large/graphical); **Okabe-Ito** 8-hue
  CUD palette (popularized by Bang Wong, *Nature Methods* 2011, https://jfly.uni-koeln.de/color/);
  ColorBrewer; viridis/cividis. Tools: WebAIM Contrast Checker, Viz Palette, Coblis / Color Oracle,
  + grayscale check.
- **Robin Williams — *The Non-Designer's Design Book*** — **C.R.A.P.** (Contrast, Repetition,
  Alignment, Proximity); Reynolds' "Big Four."
- **Edward Tufte — *The Visual Display of Quantitative Information*** (data-ink ratio, chartjunk,
  small multiples; sparklines are from *Beautiful Evidence*, 2006). + Knaflic declutter pass.
- ⚠️ **CORRECTIONS:** Developers Digest piece is **"15 Patterns of AI Design Slop"** (21% heavy /
  46% mild / 33% clean over 500 Show HN pages; "4+ tells = heavy slop"); the per-pattern percentages
  some sources quote are **fabricated — drop them.** Do **not** attribute the "AI slop aesthetic"
  framing to Anthropic. AI-image tells → **Kamali et al., arXiv:2406.08651** (five categories).
  Never parrot folklore stats ("65% recall with images", "brain processes images 60,000× faster",
  "processes visuals 60k× faster") — cite the *effect* (Paivio dual-coding / picture-superiority).

## Building the deck — React default, reveal.js/PDF fallback

- **Custom React deck (default build path: React + Vite + Framer Motion).** The deck is a hand-built
  React + Vite app scaffolded from a bundled, verified engine; Framer Motion drives the staged
  reveals and slide-to-slide motion. Per-slide speaker notes live alongside each slide. This is the
  default deliverable — a world-built deck, not a template — and the fallback path below is taken
  only when a vector PDF is the required artifact.
- **reveal.js (fallback, documented):** (Hakim El Hattab). Markdown plugin (`---` horizontal,
  `--` vertical, `Note:` notes); highlight.js with `data-line-numbers` stepped highlighting;
  Sass themes / full CSS replacement; Mermaid via plugin/iframe. https://revealjs.com/
- **PDF export (fallback path):** open `?print-pdf` → Chrome/Chromium Print → Save as PDF
  (Landscape, Margins None, Background graphics ON). Config `pdfMaxPagesPerSlide:1`,
  `pdfSeparateFragments:false`, `showNotes: true | 'separate-page'`. **Chrome/Chromium only.**
  CI path = **decktape** or a Playwright script
  (`page.goto(url+'?print-pdf'); page.pdf({preferCSSPageSize:true})`).
  https://revealjs.com/pdf-export/ · **Vector + selectable text.**
- **Fallback-path gotchas (when a PDF was produced):** install fonts in CI/containers (else
  box-glyphs); embed images locally so headless export works offline; don't expect `?print-pdf`
  in Firefox/Safari.
- **Other documented fallbacks (not default):** Slidev (sli.dev — Vue/markdown, Shiki, `slidev
  export` via playwright-chromium); Quarto revealjs (.qmd, Print View `E` / decktape); Marp
  (`marp --pdf --allow-local-files`). **Blocklisted (dead/wrong tool):** mdx-deck (abandoned),
  remark/remark.js (stagnant), Eagle.js, WebSlides, impress.js.

## Delivery

- **Chris Anderson** — talk-as-gift framing (also the best nerve cure); the danger of the middle
  "memorized-but-brittle" rehearsal zone.
- **Julian Treasure — "How to speak so that people want to listen"** (TEDGlobal 2013): HAIL,
  the seven deadly sins, vocal toolbox (register/timbre/prosody/pace/pitch/volume), six warm-ups.
  https://www.ted.com/talks/julian_treasure_how_to_speak_so_that_people_want_to_listen
- **Alison Wood Brooks** — "Get Excited: Reappraising Pre-Performance Anxiety as Excitement,"
  *J. Experimental Psychology: General* 2014, 143(3):1144–1158, DOI 10.1037/a0035325. Saying
  "I am excited" beats "calm down."
- **Noah Zandan / Quantified Communications** — filler-word pause-substitution, HBR 2018.
  https://hbr.org/2018/08/how-to-stop-saying-um-ah-and-you-know
- **Toastmasters / Vanessa Van Edwards** — gesture research (top TED talks ~465 gestures/18 min vs
  272 for lowest), the "power sphere," illustrators help / adaptors hurt.
  https://www.toastmasters.org/magazine/magazine-issues/2020/sept/the-power-of-body-language
- **Patsy Rodenburg — the Second Circle** (*The Second Circle: How to Use Positive Energy for
  Success in Every Situation*, W.W. Norton). ⚠️ **CORRECTION:** the title *Presence: Bring Your
  Boldest Self…* is **Amy Cuddy's** book — do not attribute it to Rodenburg.
- **Shoda & Yamanaka** — instructional humor, *Behavioral Sciences* (MDPI) 2022, PMC8772906
  (popular TED talks used humor ~12.9× vs ~3.9× in unpopular ones). Cite the primary, not wrappers.
- **Bryan Stevenson**, TED2012 "We need to talk about an injustice" (story-driven, ~190 wpm, no
  slides; longest TED ovation) as the conversational-authenticity exemplar.
- ⚠️ **MYTHS — do not teach as fact:** **Mehrabian 7-38-55** applies only when verbal & nonverbal
  signals about *feelings/attitudes* conflict (Mehrabian himself says so) — not to idea/data talks.
  **Cuddy power posing** failed to replicate (Ranehill n=200; Carney disavowed; TED appended a
  disclaimer) — only the self-reported "felt powerful" effect survived, fine as a private ritual.

## Keep-it-human / anti-slop (cross-cutting)

- **Shaib, Chakrabarty, Garcia-Olano, Wallace — *Measuring AI Slop in Text*** (arXiv:2509.19163):
  density/relevance/tone are the strongest predictors of "slop."
- **Charlie Guo — *The Field Guide to AI Slop*** (Oct 2025).
  https://www.ignorance.ai/p/the-field-guide-to-ai-slop
- **Kobak, González-Márquez, Horvát, Lause** — arXiv:2406.07016; published as *"Delving into
  LLM-assisted writing in biomedical publications through excess vocabulary"* (Science Advances
  2025). The authoritative excess-word list ("delve" ≈ standout, report as approximate).
- **Juzek & Ward** — arXiv:2412.11385 (COLING 2025): 21 LLM-overrepresented focal words.
- **Kamali, Groh, et al.** — *How to Distinguish AI-Generated Images from Authentic Photographs*,
  arXiv:2406.08651 (five artifact categories; humans ~31% accurate).
- **Wikipedia: Signs of AI writing** — https://en.wikipedia.org/wiki/Wikipedia:Signs_of_AI_writing
  (cited by NPR Sept 2025 & TechCrunch Nov 2025); operationalized by the `humanizer` skill.
- **Patrick Winston — *How to Speak*** (MIT OCW **RES.TLL-005**, Jan IAP 2018); **Paul Graham —
  *Write Like You Talk*** (https://www.paulgraham.com/talk.html).
- **Corporate Memphis / Alegria** as a named visual anti-pattern —
  https://en.wikipedia.org/wiki/Corporate_Memphis
- ⚠️ **De-listed as unverifiable:** the "It's Just a Word, Until It Isn't" essay
  (writerwhocodes.com — URL 500s). The point (a tell is a signal not proof) is real; re-source via
  the Kobak paper or TechnoLlama's "To delve or not to delve."
