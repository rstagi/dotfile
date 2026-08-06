---
name: talk-roast
description: |
  Run the adversarial truth audit on a finished talk — read the whole set (content brief, slides-content, delivery plan, slides-style, the built deck), then hunt for everything that will embarrass the speaker on stage: claims that are untrue, numbers that are wrong or have no source, internal contradictions, AI slop and obvious errors, misattributed quotes and zombie stats, logic holes a hostile audience member will pounce on, and chart or reputation landmines. Fact-check every number and named claim against its primary source on the live web, flag every statistic with no source, and write a brutal, severity-ranked roast report. Use whenever a talk (or even just its content brief) is built and the user wants an adversarial truth pass — a no-mercy red-team that attacks the talk rather than reconciling it — and asks to "roast my talk", "tear this apart", "find what's wrong before the audience does", "fact-check my talk", "what will I get caught on", "find the AI slop", "poke holes in this", "red-team my talk", or invokes `/talk-roast`. Trigger on phrases like "be brutal", "what's untrue here", "which numbers have no source", "what would a skeptic attack", "find the slop and the errors", "is anything in here a lie", "stress-test my claims" — even if the user doesn't say "skill" or "talk-roast" by name. This skill writes the roast report (06-roast-report.md) and does NOT edit the talk: it finds and flags with evidence, then routes the serious fixes back to the owning sub-skill. It is the adversarial complement to talk-refine, which reconciles coherence and fixes trivial drift in place; if the user wants the talk reconciled and made consistent rather than attacked, route to talk-refine instead.
allowed-tools:
  - Read
  - Write
  - Bash
  - Glob
  - Grep
  - WebSearch
  - WebFetch
  - Skill
---

# talk-roast — the adversarial truth audit (the optional red-team after refine)

This is the hostile-skeptic pass. Every upstream stage built the talk in good faith; `talk-refine`
checked that the pieces still agree. Roast assumes none of that earned trust and goes looking for the
things that get a speaker caught: a number that's wrong, a stat with no source, a quote the famous
person never said, a slide that contradicts another. It hunts the logic hole the world expert in the
front row will name in Q&A, and the AI-voice tells a room full of people who've seen a thousand decks
can smell. It reads the whole talk, fact-checks every number and claim against its primary source on
the live web, and writes `06-roast-report.md` — a brutal, severity-ranked findings list with the
evidence for each.

**Roast finds and flags; it does not fix.** That is the line between this skill and `talk-refine`.
Refine reconciles a coherent talk and edits trivial drift in place; roast is read-only on the talk
artifacts and writes only its own report. Every finding carries the offending line or slide, why it's
wrong, and a recommended fix — and the structural ones get routed back to the sub-skill that owns
them. The human decides what to change. This keeps roast safe to run any number of times and keeps its
verdict honest: it has no incentive to quietly paper over what it found.

The roast stance, adopted before reading a word (full version in [`references/roast-checklist.md`](references/roast-checklist.md)):
**every number is guilty until you've read its source; the world expert on this topic is in the front
row; the audience has sat through a thousand AI-written decks.** Adversarial but fair — and thorough
enough it never needs to manufacture a finding. A genuinely clean talk gets a short report that says so.

## When to use / honest skip

Use this when a talk is built and the user wants a no-mercy check before they present or submit — the
pass that tries to break the talk so the audience can't. It's an **optional** add-on after the
six-stage pipeline, not a pipeline stage: it runs naturally **after** `talk-refine` (refine makes the
set coherent; roast attacks what's left), but it doesn't depend on refine having run. Roast checks
whether the talk is *true*, a different question from whether it's coherent, and it can run on any talk
that exists — even just a content brief.

**Roast whatever exists, and say what you couldn't check.** Unlike refine, roast doesn't need the full
set — it will audit a content brief alone, or a brief plus slides. But it must be honest about
coverage: if there's no built deck, it didn't check the rendered charts or the visual slop; if there's
no delivery plan, it didn't check the spoken claims. List what was present and what was missing at the
top of the report. Don't imply you checked something you couldn't see.

**Honest skips:**

- **There is no talk yet** — no `01-content-brief.md`, or only a one-line topic. There's nothing to
  roast; a talk with no claims has no claims to fact-check. Point to [`talk-content`](../talk-content/SKILL.md) and stop.
- **The user wants the talk changed, not just a list of what's wrong.** Roast deliberately doesn't edit the talk. If they want
  cross-artifact drift reconciled in place, that's [`talk-refine`](../talk-refine/SKILL.md). If they
  want a section rewritten, route to the owning sub-skill. Roast hands them the list; it doesn't hold
  the pen on the talk itself.
- **The talk is genuinely clean.** Don't invent problems to look thorough. Say it's clean, list the few
  things only a human can verify (a stat behind a paywall, an unreleased detail), and stop. A fabricated
  finding is the same failure as a missed real one.

## Inputs

Resolve the working directory the way the rest of the suite does, then load **everything that exists** —
roast is a whole-talk read.

- **Working dir.** Default `~/talks/<slug>/`; runtime-overridable. Auto-detect an existing talk project
  by the earlier artifacts:

```bash
TALK_DIR="${TALK_DIR:-}"
if [ -z "$TALK_DIR" ]; then
  # current dir + any ~/talks/* project (find avoids a zsh "no matches" abort when ~/talks is empty)
  for d in . $(find "$HOME/talks" -mindepth 1 -maxdepth 1 -type d 2>/dev/null); do
    [ -f "$d/01-content-brief.md" ] && echo "candidate talk dir: $d"
  done
fi
# once resolved, inventory what's actually there — you roast what's present, and say what's missing:
ls -la "$TALK_DIR"
for f in 01-content-brief.md 02-slides-content.md 03-delivery-plan.md 04-slides-style.md deck 05-refine-notes.md; do
  [ -e "$TALK_DIR/$f" ] && echo "present : $f" || echo "absent  : $f (note in report — not checked)"
done
```

- **`01-content-brief.md`** — the spine, the evidence list with its source URLs, the analogies. The
  richest source of checkable claims; the place most numbers live.
- **`02-slides-content.md`** — assertion titles (each is a *claim* to fact-check), chart messages, the
  per-slide numbers.
- **`03-delivery-plan.md`** — the spoken claims and the opening/closing lines (a claim voiced but not on
  a slide is still a claim).
- **`04-slides-style.md`** — read for chart-deception and accessibility-credibility flags, not for taste.
- **`deck/`** — the built deck (a custom React deck by default): the *rendered* titles, charts, and
  visuals. Slop and chart deception that the spec hid show up here. Skim the slide source and any chart
  data. (On the fallback path this is a reveal.js deck / `deck.pdf` — roast the rendered slides either way.)
- **`05-refine-notes.md`** (if present) — read it to see what refine already reconciled, so roast spends
  its effort on truth and slop, leaving the coherence drift refine already logged.

Load the two shared references every stage uses: [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md)
(the gates, run adversarially here) and [`../talk/references/sources.md`](../talk/references/sources.md)
(the vetted bibliography — a claim that contradicts a correction logged there is an automatic finding).

## Process

Work the seven finding categories from [`references/roast-checklist.md`](references/roast-checklist.md)
across the eight steps below — the steps are the order, the checklist is the depth.

### Step 1 — Load everything, adopt the stance, declare coverage

Read every artifact that exists and skim the built deck. Write down, up front, exactly **what you can
and cannot check** given what's present — this becomes the report header. Adopt the three roast
assumptions. Then build your work-list: pull every number, every named claim, every quote, and every
attribution into a flat list to fact-check in Step 2. Grep the artifacts for digits, `%`, `×`, `$`,
years, and proper nouns; also harvest superlative and absolute claims (first, only, never, always,
fastest, no other, unique, proves) and every assertion title, since those carry checkable claims with
no digits. **Harvest the built deck too, not just the specs** — scan the deck source and any chart-data
files, and put every rendered title and chart value on the list. A claim the audience can see but that
lives only in the deck (a number baked into a component, text inside an image) still gets fact-checked
or marked "not checked" — it never gets silently skipped while the header says the deck was roasted.

### Step 2 — Fact-check every number and named claim (the headline)

For each item on the work-list, run the fact-check procedure from the checklist: read laterally to the
**primary source** (not its nearest citation), confirm the value, the context, and the date; recompute
any derived number by hand; verify every quote and attribution; resolve every URL the talk itself cites
(a dead or wrong link is its own finding, distinct from unsourced); state the uncertainty the talk omits.
Record a per-claim verdict — `verified` / `wrong` / `unsourced` / `misattributed` / `myth` /
`needs human check`. Cross-check against the zombie-stat and misattribution bank in the checklist and
against `../talk/references/sources.md`. **Flag every statistic with no traceable source** — that is the
user's explicit ask, and "it has a citation" is not the same as "the source exists and says this."

Prefer the **Perplexity MCP tools** if available (`perplexity_search` for the source URL,
`perplexity_research` for contested claims, `perplexity_ask` for a quick cited check); degrade to
`WebSearch` / `WebFetch`. When you can't reach a primary source, the verdict is `needs human check` —
a TODO for the speaker, so you flag it rather than guessing or passing it over in silence.

### Step 3 — Contradiction & consistency sweep

Hunt internal contradictions and drift: the same number with two values; a slide that denies another; a
beat that contradicts the throughline; one concept under three names; a timeline or "because of that"
chain that doesn't actually follow. This is the *logical* contradiction pass — pure label drift across
artifacts is `talk-refine`'s job, so if `05-refine-notes.md` already logged it, don't re-litigate;
focus on contradictions that make the talk *wrong*, not merely inconsistent. When `05-refine-notes.md`
is **absent** (refine never ran), roast owns the whole inconsistency sweep — logical contradictions and
label drift alike — rather than deferring to a stage that didn't run.

### Step 4 — Roast the slop (keep-it-human, adversarially)

Run gates A–E from [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) in roast
mode: name **every** tell with its evidence, don't stop at three. Grep the artifacts for the lexical
list (`delve`, `leverage`, `seamless`, `robust`, `tapestry`, …); read the prose aloud for the
structural tells ("it's not just X, it's Y," the "from X to Y" sweep, forced rule-of-three, bookended
self-summary, hollow conviction); check the **rendered deck** for Gate C visual slop (AI gradients,
icon-card rows, stock clichés, AI-image artifacts, a chart whose accent doesn't point at its title's
takeaway). Count flags per gate; the accumulation rule (3+ per gate, 4+ for visual Gate C) decides
whether a cluster is a rewrite or a scatter of polish notes.

### Step 5 — Easy-error sweep (low-hanging fruit)

The errors a careful read catches and a sloppy one ships: typos and grammar (largest on titles and
slides), **broken math** (recompute every percentage, multiple, and growth rate; check units; confirm
the deck's numbers agree with *each other* — a pie that sums to 103%, a funnel whose stages don't
reconcile — which needs no web access at all), misspelled product/person/company names, a framework
credited to the wrong author, surviving
placeholders (`TODO`, lorem ipsum, `XX%`, an empty chart), and formatting inconsistency (a key term
capitalized three ways, `2024` vs `2025` for one event). Use Grep/Bash to scan; don't eyeball alone.

### Step 6 — Logic holes & Q&A landmines

Find the fallacies (the checklist's list — correlation→causation, survivorship bias, cherry-picking,
false dichotomy lead the pack), the **unanswered obvious objection** (the audience map in `01` named the
resistance — is it actually answered?), the "so what" gaps, and the overclaims the evidence can't carry.
Then **write the three hardest questions** the talk invites and judge whether it can answer each — this
is what arms the speaker for the floor.

### Step 7 — Risk & reputation, structure & format

Sweep the landmines: claims about named competitors or people that are wrong or defamatory;
confidential/NDA/unreleased specifics on a public slide; **chart deception** (truncated axes, dual
axes, cherry-picked windows — checklist has the list); uncredited borrowing; claims that are already
**stale as of the roast date** or will be within weeks (date-stamp the volatile ones); tone landmines;
unqualified legal/medical/financial claims. Then the structural mismatches: word count vs the time slot
at ~130–150 wpm, talk-type mismatch, and accessibility bad enough to lose the room.

### Step 8 — Severity-rank, write the report, route the fixes back

Assign each finding a severity (🔴 Blocker / 🟠 Weak spot / 🟡 Slop / 🟢 Note — definitions in the
checklist). Write `06-roast-report.md` (template below): the honest one-line **verdict**
(*ship it / fix the blockers first / not ready* — any 🔴 caps it at *fix the blockers first*, and
*ship it* needs zero 🔴 and no unresolved load-bearing ⏳), the fact-check ledger, the findings table ordered by
severity, the AI-slop tally, the hostile-Q&A list, and the route-back table. Roast does **not** apply
fixes — it routes the structural ones back to the owning sub-skill via the Skill tool *only if the user
asks you to kick off the fixes*; by default it hands over the report and lets the human drive. Apply
Gate B to your own report prose: it should read like someone who actually opened the sources.

## Output: `06-roast-report.md`

The skill always writes this file in the working dir. It is a findings document, not an edit to the
talk. Full template (with example rows) lives in [`references/roast-checklist.md`](references/roast-checklist.md);
the shape:

```markdown
# Roast Report — <talk title>

- Working dir: <abs path>
- Roasted: <artifacts checked> · Could NOT check: <what was missing>
- Roast date: <date> (claims verified as current as of this date)

## Verdict
**<Ship it / Fix the blockers first / Not ready>** — <one honest sentence>.
Blockers: <n> · Weak spots: <n> · Slop: <n> · Notes: <n>

## Fact-check ledger (every number & named claim)
| Claim (as stated) | Where | Verdict | Primary source / correction |
| … | … | ✅/🔴/🟠/⏳ | … |

## Findings, by severity (🔴 → 🟠 → 🟡 → 🟢)
| Sev | Category | Where | What's wrong | Recommended fix |
| … | … | … | … | … |

## AI-slop tells caught (keep-it-human, adversarial)
| Gate | Tell | Where | Count |

## What a hostile audience member asks you
1. <hardest question> — can the talk answer it? <yes / no — arm the speaker>

## Route the structural fixes back (roast flags; it does not fix)
| Finding | Owner sub-skill |

## Human must verify before stage
- [ ] <each ⏳ needs-human-check claim>
- [ ] <third-party / confidential claims needing sign-off>
```

## Keep it human

Roast is the one stage that runs the gates as a *weapon*, but it must survive its own roast. Apply
**Gate B** to `06-roast-report.md` itself — no slop in the document that hunts slop. Two hard rules
that make the roast trustworthy:

- **Every "this is false" claim must itself be sourced.** A roast that misattributes a misattribution,
  or calls a real stat fake, is worse than the error it's hunting. If you can't source the correction,
  the verdict is `needs human check`, not a confident 🔴.
- **Don't manufacture findings.** The accumulation rule still applies (3+ flags in a gate = a rewrite,
  4+ for visual Gate C), but a clean talk gets a clean report. Thoroughness is reading every claim, not
  inventing problems for every slide.

Every number you touch traces back to a primary source or to [`../talk/references/sources.md`](../talk/references/sources.md);
the corrections logged there (Mehrabian, Cuddy, the 60,000× myth, the fabricated slop percentages) are
automatic findings if the talk repeats them.

## Reference files

- [`references/roast-checklist.md`](references/roast-checklist.md) — the seven finding categories, the
  fact-check procedure, the zombie-stat & misattribution bank, the logical-fallacy and chart-deception
  lists, the severity scale, and the full `06-roast-report.md` template.

Shared, don't duplicate: [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md)
(the gates A–E, run adversarially here) · [`../talk/references/sources.md`](../talk/references/sources.md)
(the vetted bibliography every correction traces to).

## Handoff

Runs after the pipeline, once the talk is built and (usually) refined. It reads the whole set, attacks
it, and writes the report — then routes structural fixes back to the owning sub-skill. It writes nothing
into the talk itself.

| Direction | Skill | What flows |
| --- | --- | --- |
| Reads ← | [`talk-content`](../talk-content/SKILL.md) | `01-content-brief.md` — claims, evidence + source URLs, analogies, the spine |
| Reads ← | [`talk-slides-content`](../talk-slides-content/SKILL.md) | `02-slides-content.md` — assertion titles (each a claim), chart messages, numbers |
| Reads ← | [`talk-delivery`](../talk-delivery/SKILL.md) | `03-delivery-plan.md` — spoken claims, opening/closing lines |
| Reads ← | [`talk-slides-style`](../talk-slides-style/SKILL.md) | `04-slides-style.md` — read for chart-deception + accessibility-credibility flags |
| Reads ← | [`talk-slides-build`](../talk-slides-build/SKILL.md) | the built deck — rendered titles, charts, visual slop, chart deception |
| Reads ← | [`talk-refine`](../talk-refine/SKILL.md) | `05-refine-notes.md` (if present) — what coherence drift was already reconciled |
| Writes → | _(the speaker)_ | `06-roast-report.md` — severity-ranked findings + fact-check ledger; routes structural fixes back, edits nothing in the talk |
