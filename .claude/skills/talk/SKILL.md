---
name: talk
description: |
  Build a conference talk end to end — gather inputs once, then run the six-stage pipeline (content → slides-content → delivery → slides-style → built deck → refine) in order, running the right Keep-It-Human gate between every stage. Use whenever the user wants to make a talk from scratch and doesn't want to drive each stage by hand, asks "help me build a talk", "I'm speaking at <conference>", "turn this idea into a talk", "take this from idea to slides", or invokes `/talk`. Trigger on phrases like "I have a talk to give", "make me a conference talk", "run the whole talk pipeline", "I need a keynote / lightning talk / workshop", "build my deck and notes" — even if the user does not say "skill" or "talk" by name. This skill is the front door and orchestrator; it gathers inputs, creates the working dir, and invokes the six sub-skills via the Skill tool. It does not itself write the brief, structure slides, coach delivery, style the deck, build it, or refine it — the sub-skills own that. You can also run any single stage standalone.
allowed-tools:
  - Read
  - Write
  - Bash
  - Skill
---

# /talk — idea to delivery, one pipeline

This is the front door for the talk suite. It collects the talk's facts once (topic, type, time, audience, venue, point of view), creates a shared working directory, and runs six sub-skills in order — each reads the previous ones' artifacts and writes its own. Between stages it runs the matching gate from [`references/keep-it-human.md`](references/keep-it-human.md) and refuses to advance a stage that trips 3+ flags (4+ for the visual slice of Gate C).

The spine that survives every handoff: the throughline (<=15 words), the one designed memorable moment, and the named specifics (names/dates/numbers/places) decided in Stage 1. Downstream stages dress them up; they never replace them. A stage that wants to change the argument loops back to `talk-content` — it does not edit the spine itself.

## When to use / honest skip

Use this when someone wants the whole talk built and would rather answer a few questions than babysit six skills. If they only want one piece, skip the orchestrator and invoke that sub-skill directly — say so out loud:

- **"Just figure out what to say" / no throughline yet** → [`talk-content`](../talk-content/SKILL.md).
- **"Structure my slides" / "what's the assertion title for each"** → [`talk-slides-content`](../talk-slides-content/SKILL.md).
- **"Write my speaker notes / cue cards" / "how do I deliver this"** → [`talk-delivery`](../talk-delivery/SKILL.md).
- **"Pick fonts and colors" / "make my deck not look AI-generated"** → [`talk-slides-style`](../talk-slides-style/SKILL.md).
- **"Build the deck (custom React site by default; reveal.js/PDF on request)"** → [`talk-slides-build`](../talk-slides-build/SKILL.md).
- **"Check the whole thing's consistent before I ship"** → [`talk-refine`](../talk-refine/SKILL.md).
- **"Fact-check it / find what's untrue / roast it before I ship"** → [`talk-roast`](../talk-roast/SKILL.md) — the **optional** adversarial truth audit, run after refine.

Honest skips:

- **No topic or inputs yet.** Don't invent a talk. Ask the six input questions below and wait. A talk with no point of view is the thing the gates exist to catch — starting one blind wastes the whole pipeline.
- **A working dir already has artifacts.** If `~/talks/<slug>/` (or the dir the user names) already contains `01-content-brief.md`, `02-slides-content.md`, `03-delivery-plan.md`, `04-slides-style.md`, `deck/`, or `05-refine-notes.md`, do NOT clobber them. List what exists, infer the furthest completed stage, and offer to resume from the next one (or re-run a specific stage). Resuming is the default; full rebuild only on explicit ask.

## Inputs

Gather these **once**, up front, so no sub-skill re-asks:

- **Topic** — the subject, in the user's words (not yet a throughline; Stage 1 forges that).
- **Type** — keynote / conference session / lightning or Ignite / workshop / online talk. Drives time-vs-content budget and structure.
- **Time** — minutes on stage (and whether Q&A is inside or outside it).
- **Audience** — who's in the room, what they already know, what they resist.
- **Venue** — conference name, in-person vs online vs hybrid, slot context.
- **POV** — the one opinion the user is willing to defend. If they don't have one yet, that's fine; flag it so Stage 1 forces a choice (a talk with no stance fails Gate A).

**Resolve the working dir:** default `~/talks/<slug>/` where `<slug>` is a kebab-case of the topic. Let the user override the path. Create it with `mkdir -p`. Auto-detect an existing project: if the resolved dir (or a dir the user points at) already holds earlier artifacts, switch to the resume flow above.

## Process

### Step 1 — Gather inputs once, resolve the dir, print the plan

Ask the six inputs (batch them; don't interrogate one at a time). Resolve and create the working dir. Then print the six-stage plan and the working-dir tree (below) so the user knows what's coming and where files land. Confirm before kicking off Stage 1.

### Step 2 — Stage 1: content → `01-content-brief.md`

Invoke skill `talk-content`. It writes the talk-brief and locks the spine: throughline, the named specifics, audience map, type+time budget, the chosen story skeleton, hook + close, validated analogies, evidence with primary-source URLs, and the designed memorable moment.

Then run **Gate A + Gate B** on `01-content-brief.md` (thesis/content + voice/prose, per [`references/keep-it-human.md`](references/keep-it-human.md)). Count flags. **3+ in either gate → do not advance.** Tell the user exactly what tripped (quote the offending line), fix it (rewrite, not patch), re-run the gate, then continue.

### Step 3 — Stage 2: slides-content → `02-slides-content.md`

Invoke skill `talk-slides-content`. It reads `01` and owns slide STRUCTURE: slide count (don't pad it to a number), one idea per slide, the complete-sentence assertion title per slide (Alley), which layout class each slide uses, the proving-visual concept per slide (real photography or hand-built diagram only — no AI images), the per-slide word budget (Mayer: don't put the spoken sentence on screen), each chart's message + type + declutter spec + takeaway element, the motion/fragment intent, and which slide carries the memorable moment. It does NOT choose fonts or colors.

Then run **Gate C — structure slice** (the bullets `talk-slides-content` owns): one idea per slide, sentence-assertion titles, the 3-second glance + assertion-integrity tests. **4+ tells = rewrite the structure**, not 3. Report flags, fix, re-run, continue.

### Step 4 — Stage 3: delivery → `03-delivery-plan.md`

Invoke skill `talk-delivery`. It reads `01` + `02` and writes the delivery plan: trigger-word cue cards written for the ear and keyed to `02` slide order (the deck does not exist yet), a hand-written opening and closing line, pace/pause/emphasis marks, a pre-talk countdown + warm-ups, a nerve reframe, a humor risk-check, and the context variant (in-person / online / workshop).

Writing cue cards for the ear forces voicing the talk, which surfaces slide-content problems. `talk-delivery` **MAY propose specific edits to `02-slides-content.md`** — a slide redundant with speech (Mayer), a beat with no slide, a missing transition, a slide that says what the speaker would not. It PROPOSES, you surface those edits to the user, and on approval it APPLIES them (Write/Edit) and logs them in the delivery plan. It must NEVER touch the spine (throughline / memorable moment / named specifics) — that's a loop back to `talk-content`.

Then run **Gate D + Gate B** (delivery + voice). Confirm no myths taught as fact (no Mehrabian 7-38-55, no Cuddy power-posing), opening and closing lines hand-written, every number verified. Report, fix, continue.

### Step 5 — Stage 4: slides-style → `04-slides-style.md`

Invoke skill `talk-slides-style`. It reads `01` + `02` + `03` and owns the LOOK only: one distinctive typeface (+ at most one pairing) mapped to `--talk-font-display/body/mono`; the 60-30-10 meaning-mapped palette mapped to the six neutral tokens (`--talk-bg/ink/ink-bright/muted/rule/surface`) plus two meaning-mapped accents (`--talk-accent`, `--talk-accent-2`); the accessibility hard checks (WCAG >=4.5:1 text / >=3:1 large, Okabe-Ito colorblind-safe pair, greyscale survival, no blue→purple AI gradient, never `#1e1e2e`); the grid/alignment rule; the per-layout-class token-role mapping; the accent that marks each chart takeaway. It ends with the paste-ready `accents.css` block + the Google Fonts `<link>`. It does NOT author assertion titles, choose layout classes, or write chart declutter logic (those are `talk-slides-content`).

Then run **Gate C — look slice** (the bullets `talk-slides-style` owns): no AI gradient, no default/AI-era fonts, palette meaning-mapped + accessible, no AI-image artifacts, charts get one accent on the takeaway, not the default theme, plus the squint/back-row legibility test. **4+ tells = restyle.** Report, fix, continue.

### Step 6 — Stage 5: build → `deck/` (built site) [+ optional `deck.pdf`]

Invoke skill `talk-slides-build`. It reads `02` + `04` + `03` and builds a custom React deck — translating assertion titles + layout classes into components, mapping the `04` style tokens (the `accents.css` block) into the deck's CSS, and placing the cue cards into the deck's per-slide speaker notes. The reveal.js Markdown + `Note:` blocks + the `deck.pdf` export are the documented fallback path, invoked only when the user asks for a PDF.

`talk-slides-build` already verifies the build at its own last step; this is the orchestrator's independent re-check. Run **Gate C (rendered)** — did the built deck honor the structure and the style, or drift to placeholder defaults? — plus the React-deck checks below. If the user took the reveal.js/PDF fallback path, also run the conditional PDF checks. Substitute the real working dir for `<slug>` (there's no persistent `$TALK_DIR` across sub-skill invocations):

```bash
TALK_DIR="$HOME/talks/<slug>"            # the working dir you resolved in Step 1

# DEFAULT (custom React deck):
( cd "$TALK_DIR/deck" && npm run build )            # build succeeds
#   slide count rendered == the 02 spec
#   the 04 tokens/accents render (not placeholder defaults)
#   deck is keyboard-navigable (arrow keys advance staged reveals + slides)

# CONDITIONAL — only if a deck.pdf was produced (reveal/PDF fallback path):
if [ -f "$TALK_DIR/deck.pdf" ]; then
  if command -v pdffonts >/dev/null 2>&1; then
    pdfinfo  "$TALK_DIR/deck.pdf" | grep -i pages   # page count == slide count
    pdffonts "$TALK_DIR/deck.pdf"                    # every font row should read emb=yes
    pdftotext "$TALK_DIR/deck.pdf" - | sed '/^$/d' | head -20   # real text ⇒ vector/selectable
  else
    echo "poppler not installed (brew install poppler) — open $TALK_DIR/deck.pdf, page through it, and select text by hand"
  fi
fi
```

For the default React deck, a failed `npm run build`, a slide count that disagrees with `02`, or placeholder defaults rendering instead of the `04` tokens/accents — fix the build before continuing. Absence of a `deck.pdf` is NOT a failure; the PDF only exists on the fallback path. On that path, if poppler is present and `pdftotext` yields no text, the export rasterized — fix the build; if poppler is absent, confirm by opening the PDF and don't read the empty output as a failure.

### Step 7 — Stage 6: refine → `05-refine-notes.md`

Invoke skill `talk-refine`. It reads ALL of the above and runs last, after the deck exists. It verifies the spine survived unchanged end-to-end (throughline identical in `01`/`02`/`03`/`04` and on the title + statement slides; memorable moment lands the same in the deck slide and the delivery beat; numbers consistent and still traced to sources), checks cross-artifact consistency (each assertion title in `02` == the rendered deck title == referenced by its cue card in `03`; cue cards present as the deck's speaker notes; slide count consistent; the rendered deck honors `04`'s tokens/accents; rendered slide count == `02`; and, if a PDF deliverable was produced, PDF page count == slide count, vector/selectable), and re-runs **Gates A–E holistically** across the finished set.

It FIXES trivial incoherences in place (Edit/Write), ASKS approval for non-trivial ones (including any deck change), and routes structural problems back to the owning sub-skill via the Skill tool — surface those routes to the user. It writes `05-refine-notes.md` = a reconciliation changelog + the residual HUMAN-ONLY TODOs.

### Step 8 — Final summary

List the seven artifacts with absolute paths. Give the deck preview command (`npm run dev` inside `deck/`) and, if a PDF was exported, the `deck.pdf` path. Read `05-refine-notes.md` back to the user as the ship-readiness summary, and name anything the human must still do by hand (record a rehearsal, commission the hero image, verify a stat you couldn't).

Then offer the **optional** adversarial pass: [`talk-roast`](../talk-roast/SKILL.md) fact-checks every number and claim against primary sources, hunts AI slop, contradictions, and on-stage landmines, and writes `06-roast-report.md`. It is not one of the six pipeline stages — it's the red-team you run when you want the talk attacked before the audience does.

## Working-dir tree

```
~/talks/<slug>/
├── 01-content-brief.md      ← talk-content        (the talk-brief / spine)
├── 02-slides-content.md     ← talk-slides-content (slide structure: assertion titles, layout classes, visuals)
├── 03-delivery-plan.md      ← talk-delivery       (cue cards for the ear; may amend 02)
├── 04-slides-style.md       ← talk-slides-style   (the look: type, palette, tokens, accents.css)
├── deck/                     ← talk-slides-build   (custom React deck source; `npm run dev`)
│   └── ...
├── deck.pdf                  ← talk-slides-build   (optional; vector, selectable export — reveal.js/PDF fallback)
├── 05-refine-notes.md       ← talk-refine         (reconciliation changelog + human-only TODOs)
└── 06-roast-report.md       ← talk-roast          (optional adversarial truth audit: fact-check + slop + landmines)
```

## The six sub-skills

| Sub-skill | Consumes | Produces | Run standalone when |
| --- | --- | --- | --- |
| [`talk-content`](../talk-content/SKILL.md) | the six inputs | `01-content-brief.md` | you only need the argument, throughline, and structure |
| [`talk-slides-content`](../talk-slides-content/SKILL.md) | `01-content-brief.md` | `02-slides-content.md` | you have a brief and want the slide structure: assertion titles, layout classes, proving visuals |
| [`talk-delivery`](../talk-delivery/SKILL.md) | `01` + `02-slides-content.md` | `03-delivery-plan.md` (+ approved edits to `02`) | you have a brief and slide structure and want notes + cue cards |
| [`talk-slides-style`](../talk-slides-style/SKILL.md) | `01` + `02` + `03-delivery-plan.md` | `04-slides-style.md` | you have the slide structure and want the look: type, palette, accents.css |
| [`talk-slides-build`](../talk-slides-build/SKILL.md) | `02` + `04` + `03-delivery-plan.md` | `deck/` (custom React deck) [+ optional `deck.pdf`] | you have structure + style + cues and want the built deck — React by default; reveal.js/PDF on request |
| [`talk-refine`](../talk-refine/SKILL.md) | ALL of the above | `05-refine-notes.md` | you have a finished set and want it reconciled and gate-checked before shipping |

## Keep it human

Run the gate that matches each handoff, between stages, from [`references/keep-it-human.md`](references/keep-it-human.md): **Gate A + B** after Stage 1 (content); **Gate C — structure slice** after Stage 2 (slides-content); **Gate D + B** after Stage 3 (delivery); **Gate C — look slice** after Stage 4 (slides-style); **Gate C — rendered after build; the PDF check only when a PDF was produced** after Stage 5 (build); **all of Gates A–E holistically** in Stage 6 (refine, the final pass). Apply the accumulation rule — **3+ flags (4+ for the visual Gate C) means rewrite, not patch** — and don't advance a tripped stage. Apply Gate B to this orchestrator's own prose too. Verify every number against [`references/sources.md`](references/sources.md) as it flows through.

## Reference files

- [`references/keep-it-human.md`](references/keep-it-human.md) — the shared anti-AI-slop gates (A–E); each sub-skill runs its slice, this orchestrator runs the gate between stages, `talk-refine` re-runs them all.
- [`references/sources.md`](references/sources.md) — the fact-checked bibliography every claim and statistic traces back to.

## Handoff

This skill is the front door; the chain it drives:

| Stage | Skill | Hands to |
| --- | --- | --- |
| 1 | [`talk-content`](../talk-content/SKILL.md) | → `talk-slides-content` |
| 2 | [`talk-slides-content`](../talk-slides-content/SKILL.md) | → `talk-delivery` |
| 3 | [`talk-delivery`](../talk-delivery/SKILL.md) | → `talk-slides-style` |
| 4 | [`talk-slides-style`](../talk-slides-style/SKILL.md) | → `talk-slides-build` |
| 5 | [`talk-slides-build`](../talk-slides-build/SKILL.md) | → `talk-refine` |
| 6 | [`talk-refine`](../talk-refine/SKILL.md) | → ship it |
