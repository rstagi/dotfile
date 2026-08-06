---
name: talk-refine
description: |
  Run the final cross-artifact coherence pass on a finished talk — load the whole set (content brief, slides-content, delivery plan, slides-style, the built deck — a custom React deck by default, plus a PDF if one was exported via the fallback path), verify the spine survived end to end unchanged, check that every artifact agrees with the rendered deck, re-run the anti-slop gates holistically, reconcile the trivial incoherences in place, and write a reconciliation changelog plus the residual human-only TODOs. Use whenever the talk is fully built and the user asks to "do a final pass", "make everything consistent", "check the deck matches the notes", "reconcile the talk", "is this ready to ship", "final QA on my talk", or invokes `/talk-refine`. Trigger on phrases like "tie it all together", "last look before I present", "do the titles in the deck match the spec", "did the throughline survive", "final coherence check", "ship-readiness pass" — even if the user doesn't say "skill" or "talk-refine" by name. This skill writes the reconciliation notes (05-refine-notes.md) and fixes trivial incoherences in place; it does not re-argue the content, redesign the slides, restyle the look, rebuild the deck, or rewrite the delivery — it routes those back to the owning sub-skill. It reconciles coherence and fixes trivial drift in place; it does not fact-check claims, hunt untrue numbers, or attack the talk adversarially — that is talk-roast.
allowed-tools:
  - Read
  - Write
  - Edit
  - Bash
  - Glob
  - Skill
---

# talk-refine — the final coherence pass, after the deck exists

Last stop in the talk pipeline. Every other stage built one artifact and handed it forward; you are the only stage that reads **all of them at once** and checks they still tell the same story. You load `01-content-brief.md`, `02-slides-content.md`, `03-delivery-plan.md`, `04-slides-style.md`, and the built deck (a custom React deck by default; `deck.pdf` only if a PDF export was produced via the fallback path), then verify three things: the spine survived end to end, every artifact agrees with the rendered deck, and the whole set still clears the Keep-It-Human gates. You fix the trivial drift in place, ask before anything non-trivial, route structural problems back to the sub-skill that owns them, and write `05-refine-notes.md` — a reconciliation changelog plus the TODOs only a human can close.

The spine — the throughline (≤15 words), the one memorable moment, the named specifics — was locked in `talk-content` and was supposed to flow downstream untouched. You may fix wording, labels, and consistency. You may **not** silently rewrite the spine. If the throughline reads differently in `01` than on the title slide, that is not a refine fix — surface it and route it back to `talk-content`. Refine reconciles a finished talk; it does not finish the argument.

## When to use / honest skip

Use this when the talk is genuinely built — content, slides-content, delivery, style, and the built deck (a custom React deck by default; a `deck.pdf` too if a PDF export was produced via the fallback path) all exist — and the user wants a final, holistic ship-readiness pass before they walk on stage or submit the deck.

**Honest skip — do NOT "refine" a half-built talk.** Refine reconciles a complete set; it cannot finish the work, and pretending to polish a missing stage just hides the gap. Stop and name the stage to run first if:

- **The brief is missing or thin** — no `01-content-brief.md`, or no settled throughline → run [`talk-content`](../talk-content/SKILL.md) first. There is no spine to verify yet.
- **No slides-content** — no `02-slides-content.md` → run [`talk-slides-content`](../talk-slides-content/SKILL.md). There are no assertion titles or slide count to reconcile against.
- **No delivery plan** — no `03-delivery-plan.md` → run [`talk-delivery`](../talk-delivery/SKILL.md). There are no cue cards to match to slides.
- **No style spec** — no `04-slides-style.md` → run [`talk-slides-style`](../talk-slides-style/SKILL.md). There is no palette/type spec to compare the deck's `accents.css` against.
- **No built deck** — no built deck (a custom React deck by default) in `deck/` → run [`talk-slides-build`](../talk-slides-build/SKILL.md). Refine runs *after* the deck exists; it checks the rendered thing, it does not render it. (`deck.pdf` is optional — only present if a PDF export was produced via the fallback path.)
- **The user wants the talk attacked, fact-checked, or red-teamed — not reconciled.** That's the adversarial truth audit, [`talk-roast`](../talk-roast/SKILL.md), not refine. Refine assumes good faith and checks that the pieces agree; roast assumes nothing and hunts what's false, unsourced, or embarrassing.

List exactly what is present, name the first missing stage, and stop. Don't refine around a hole.

## Inputs

Resolve the working directory the same way the rest of the suite does, then load **every** artifact — refine is a whole-set read, not a single handoff.

- **Working dir.** Default `~/talks/<slug>/`; runtime-overridable. Auto-detect an existing talk project by the presence of the earlier artifacts. From the current dir and `~/talks/*`, the marker to find is the full set:

```bash
TALK_DIR="${TALK_DIR:-}"
if [ -z "$TALK_DIR" ]; then
  for d in . ~/talks/*; do
    [ -f "$d/01-content-brief.md" ] && [ -f "$d/02-slides-content.md" ] && echo "candidate talk dir: $d"
  done
fi
# once resolved, list what's actually there before judging readiness:
ls -la "$TALK_DIR"
for f in 01-content-brief.md 02-slides-content.md 03-delivery-plan.md 04-slides-style.md deck; do
  [ -e "$TALK_DIR/$f" ] && echo "present : $f" || echo "MISSING : $f"
done
# deck.pdf is OPTIONAL — only present if a PDF export was produced via the fallback path:
[ -e "$TALK_DIR/deck.pdf" ] && echo "present (optional): deck.pdf" || echo "absent (optional): deck.pdf — no PDF export"
```

- **`01-content-brief.md`** (required) — the spine source of truth: throughline verbatim, the memorable moment, the named specifics (names / dates / numbers / places), evidence with primary-source URLs.
- **`02-slides-content.md`** (required) — slide count, the per-slide assertion titles, layout class per slide, the proving-visual concept, word budgets, the chart messages, which slide carries the memorable moment.
- **`03-delivery-plan.md`** (required) — cue cards keyed by slide number, the hand-written opening and closing lines, the memorable-moment beat, any edits delivery applied to slides-content.
- **`04-slides-style.md`** (required) — the typeface mapping, the 60-30-10 meaning-mapped palette, the six neutral tokens + two accents, and the paste-ready `accents.css` block.
- **`deck/`** (required) — the built deck (a custom React deck by default): the actual slide titles, the per-slide speaker notes, and the resolved accent/token values that actually shipped. (On the fallback path this is a reveal.js deck; either way you reconcile against its slide titles, speaker notes, and resolved tokens — don't assume a `slides.md` / `theme/accents.css` layout.)
- **`deck.pdf`** (optional) — present only if a PDF export was produced via the fallback path: page count, embedded fonts, selectable text.

If any required artifact is missing, switch to the honest-skip path above and stop.

## Process

### Step 1 — Load everything and restate the spine

Read all five markdown artifacts and skim the built deck (a custom React deck by default) — its slide titles, per-slide speaker notes, and resolved accent/token values. Then write down, from `01-content-brief.md` **verbatim**, the three spine elements you will trace through everything else:

- **Throughline** — the ≤15-word shift in understanding.
- **Memorable moment** — the one designed beat the audience leaves with.
- **Named specifics** — the names, dates, numbers, and places the talk stakes its credibility on.

These are your reference. Every later check asks one question: *did this survive unchanged?* You are not allowed to "improve" them here — you are checking fidelity to what `talk-content` decided.

### Step 2 — Spine-survival check, end to end

Trace the spine through every downstream artifact and the rendered deck. Flag any drift; for each, decide later (Step 5) whether it is a trivial label fix or a real spine change that routes back to `talk-content`.

- **Throughline identical** across `01` / `02` / `03` / `04`, and on the title slide + the statement slide in the deck. Word-for-word, not "close enough." A throughline that mutated by a stage is the single most expensive incoherence — the talk now argues two slightly different things. If it differs, this is **not** a refine fix; surface it and route to `talk-content`.
- **Memorable moment lands the same** in two places: the deck slide that carries it (the one `02` marked) and the delivery beat in `03`. Same moment, same payload. If the deck builds to one climax and the notes build to another, the audience gets neither.
- **Every number consistent and still traced.** Each named statistic appears with the same value in `01`, in any slide/chart that shows it, and in the cue card that voices it — and each still traces to a primary source in [`../talk/references/sources.md`](../talk/references/sources.md). A number that drifted between the brief and a chart (a rounded "47%" that started life as "46.8%") is a credibility hole; a number with no source is worse.

### Step 3 — Cross-artifact consistency

Now check the artifacts agree with the **rendered deck**, not just with each other. The deck is what the audience sees; where a spec and the deck disagree, the deck is the fact and the spec is the bug (or vice versa — name which).

- **Assertion title triple-match.** For each slide: the assertion title in `02-slides-content.md` == the rendered title in the built deck == the title referenced by that slide's cue card in `03-delivery-plan.md`. A title the build softened into a topic label, or a cue card that points at a headline the deck no longer shows, breaks the chain.
- **Cue cards present as per-slide speaker notes.** Each delivery cue card from `03` should exist as the speaker note for its slide in the built deck, keyed to the right slide. If `talk-delivery` wrote the cards but `talk-slides-build` never placed them, the speaker view is empty — flag it.
- **Slide count consistent.** The slide count in `02` == the number of rendered slides in the built deck (count the deck's slide entries/components). A padded or dropped slide shows up here. (If a PDF was exported, also reconcile against its page count under the conditional PDF check below.)
- **Deck accents match the style spec.** The deck's resolved accent/token values (read the deck's resolved CSS variables) == the paste-ready block in `04-slides-style.md`: the same hex values on the same six neutral tokens + two accents, the same font families, the accents still meaning-mapped to the same two ideas. A deck whose shipped token values drifted from the approved palette is shipping an unapproved look.
- **If a PDF was exported (fallback path), the PDF is real.** Only when a `deck.pdf` was produced: page count == slide count, fonts embedded, text selectable (vector, not rastered). Run the check:

```bash
cd "$TALK_DIR"
if [ -e deck.pdf ]; then
  if command -v pdfinfo >/dev/null 2>&1; then
    pdfinfo  deck.pdf | grep -i Pages                  # Pages == slide count from 02
    pdffonts deck.pdf                                   # every font row reads emb=yes
    pdftotext deck.pdf - | sed '/^$/d' | head -20       # real words ⇒ vector / selectable, not an image dump
  else
    echo "poppler not installed (brew install poppler) — open deck.pdf, count pages by hand, and try to select text with the cursor"
  fi
else
  echo "no deck.pdf — no PDF export was produced (custom React deck is the default); skip the PDF checks"
fi
```

If a PDF was exported and its page count exceeds the slide count, a staged reveal or a stepped code highlight exported as extra pages — that's a build fix, route to `talk-slides-build`. If a font reads `emb=no` or `pdftotext` returns nothing, the export is broken — same route. Don't paper over a rasterized PDF in the notes.

### Step 4 — Re-run gates A–E holistically

Run **all** of [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md), gates A through E, across the **finished set** — not slice by slice the way each upstream stage ran its own. You are looking for slop that only shows up at the seams: a Gate A throughline that lost its point of view by the deck, a Gate B closing line that reads like a press release once you hear it next to the opener, a Gate C chart whose one accent no longer points at the takeaway in the rendered deck, a Gate D cue card that teaches Mehrabian 7-38-55 as fact, a Gate E number nobody verified.

Apply the accumulation rule per gate: **3+ flags in a gate = rewrite, not patch** (the whole of Gate C — structure + look — is the exception at **4+ tells**). A gate that trips the rule is not a refine reconciliation — it is a structural problem that routes back to the owning stage (Step 5). Record the per-gate flag count in the notes.

### Step 5 — Reconcile: fix in place, ask, or route back

For every incoherence found in Steps 2–4, take exactly one action and log which:

- **Fix trivial incoherences in place** (Edit / Write). A title in `02` whose punctuation drifted from the deck; a cue card keyed to slide 7 when the deck moved it to slide 8; a hex typo in one artifact that disagrees with the approved `accents.css`; a date written `2024` in one place and `2025` in another where the source settles it. Make the small edit so the set agrees, and write down what you changed and why.
- **ASK the user before anything non-trivial — including any deck change.** Rewording an assertion title, re-keying a whole block of cue cards, changing a chart's takeaway, editing the built deck's slide source or its resolved accent/token values: propose the exact change, show the before/after, and wait for approval before applying. The deck is the deliverable; don't edit it silently.
- **Route a structural problem back to the owning sub-skill** via the Skill tool. A gate that tripped its accumulation rule, a spine element that genuinely diverged, a missing slide, or (if a PDF was exported) a rasterized PDF — these are not reconciliations, they are re-work:
  - Spine / throughline / argument diverged → [`talk-content`](../talk-content/SKILL.md).
  - Assertion titles, layout class, chart message, slide count → [`talk-slides-content`](../talk-slides-content/SKILL.md).
  - Palette / typeface / accessibility / `accents.css` drift → [`talk-slides-style`](../talk-slides-style/SKILL.md).
  - Deck rendered wrong / speaker notes not placed / (if exported) PDF broken → [`talk-slides-build`](../talk-slides-build/SKILL.md).
  - Cue cards / opening-closing line / delivery marks → [`talk-delivery`](../talk-delivery/SKILL.md).

The spine rule binds here: refine may fix wording, labels, and consistency, but a throughline / memorable-moment / named-specific that is *inconsistent across stages* gets surfaced and routed to `talk-content` — never quietly rewritten by you.

### Step 6 — Write `05-refine-notes.md`

Assemble the reconciliation changelog (template below): what you checked, the table of incoherences and how each was reconciled, the deck re-verification result (and the PDF re-verification result if a PDF was exported), the gates-re-run result, and the residual human-only TODOs. This file is the record that the talk was tied together and the list of the few things a machine cannot close.

## Output: `05-refine-notes.md`

The skill always produces this file in the working dir. Template:

```markdown
# Refine Notes — <talk title>

- Working dir: <abs path>
- Spine restated (from 01, verbatim):
  - Throughline (≤15 words): <…>
  - Memorable moment: <…>
  - Named specifics: <names / dates / numbers / places>
- Artifacts checked: 01-content-brief.md · 02-slides-content.md · 03-delivery-plan.md · 04-slides-style.md · deck/ (the built deck — a custom React deck by default) · deck.pdf (only if a PDF export was produced)

## Spine survival (Step 2)
- Throughline identical across 01/02/03/04 + title & statement slides: <yes / DRIFT — routed to talk-content>
- Memorable moment lands the same in deck slide + delivery beat: <yes / fixed / asked>
- Numbers consistent and traced to sources: <yes / list any that drifted>

## Incoherences found → how reconciled
| # | Where | Incoherence | Reconciliation |
|---|-------|-------------|----------------|
| 1 | 02 vs deck slide 4 | title punctuation differs | fixed in place (Edit 02) |
| 2 | 03 cue card slide 7 | keyed to wrong slide | fixed in place (Edit 03) |
| 3 | deck resolved tokens vs 04 | accent-2 hex differs | ASKED — awaiting approval |
| 4 | Gate C, rendered deck | chart accent not on takeaway | ROUTED → talk-slides-content |
…one row per incoherence; Reconciliation ∈ {fixed in place | asked | routed → <skill>}

## Deck re-verification (Step 3)
- Slide count (02) == rendered slides in the built deck: <N == N / mismatch>
- Speaker notes present per slide: <yes / which slides are missing notes>
- Shipped token values == 04 (resolved CSS variables vs the approved accents.css): <match / which token differs>

### PDF export (if produced)
- Page count == slide count: <N == N / mismatch / no PDF export>
- pdffonts: <all emb=yes / which font isn't / no PDF export>
- pdftotext: <selectable text confirmed / rasterized — routed to talk-slides-build / no PDF export>

## Gates re-run holistically (Step 4)
| Gate | Flags | Verdict |
|------|-------|---------|
| A — thesis/content | <n> | <clean / fixed / ROUTED → talk-content> |
| B — voice/prose | <n> | <clean / fixed> |
| C — slide visuals (4+ = redesign) | <n> | <clean / ROUTED → talk-slides-content|style|build> |
| D — delivery | <n> | <clean / ROUTED → talk-delivery> |
| E — process | <n> | <clean / fixed> |

## Residual human-only TODOs
- [ ] Record a rehearsal and run the self-audit (sound-off + audio-only) — a machine can't watch you present.
- [ ] Commission / shoot the hero image the spec calls for (no AI-generated fill).
- [ ] Verify <the one stat I could not trace to a primary source> before you stand up.
- [ ] <any change I ASKED about and you have not yet approved>
```

## Keep it human

Re-run **all of gates A–E** from [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) across the finished set (Step 4) — you are the holistic pass the per-stage gates can't be, because slop hides at the seams between artifacts. Apply the accumulation rule (3+ flags = rewrite, 4+ for the visual Gate C), and treat a tripped gate as a route-back, not a patch. Apply Gate B to your **own** prose in `05-refine-notes.md` too: the changelog should read like a person who actually checked, not a status template. Every number you touch while reconciling traces back to [`../talk/references/sources.md`](../talk/references/sources.md) — if you can't trace it, it becomes a human-only TODO, not a silent edit.

## Reference files

Refine has no domain references of its own; it uses the shared pair every stage links:

- [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) — the anti-slop gates A–E; here you run **all** of them holistically as the last stage.
- [`../talk/references/sources.md`](../talk/references/sources.md) — the fact-checked bibliography every number must still trace to.

## Handoff

Last skill in the chain. It reads the whole set, reconciles it, and writes the notes that say the talk is tied together.

| Direction | Skill | What flows |
| --- | --- | --- |
| Reads ← | [`talk-content`](../talk-content/SKILL.md) | `01-content-brief.md` — the spine: throughline, memorable moment, named specifics, evidence + sources |
| Reads ← | [`talk-slides-content`](../talk-slides-content/SKILL.md) | `02-slides-content.md` — slide count, assertion titles, layout classes, chart messages, memorable-moment slide |
| Reads ← | [`talk-delivery`](../talk-delivery/SKILL.md) | `03-delivery-plan.md` — cue cards keyed by slide, opening/closing lines, the memorable-moment beat |
| Reads ← | [`talk-slides-style`](../talk-slides-style/SKILL.md) | `04-slides-style.md` — palette, typeface, the paste-ready `accents.css` block |
| Reads ← | [`talk-slides-build`](../talk-slides-build/SKILL.md) | the built deck (a custom React deck by default) in `deck/` — the rendered titles, per-slide speaker notes, shipped accent/token values; plus `deck.pdf` if a PDF was exported via the fallback path |
| Writes → | _(ship it)_ | `05-refine-notes.md` + trivial reconciliations applied in place; routes structural problems back to the owning sub-skill |
