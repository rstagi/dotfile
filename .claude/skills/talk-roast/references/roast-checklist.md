# The Roast Checklist — what to hunt, how to verify, what to ship

The deep checklist behind `talk-roast`. The SKILL.md drives the process; this file is the
ammunition: the seven finding categories, the fact-check procedure, a starter bank of
zombie stats and misattributed quotes that show up in tech and business talks, the
logical-fallacy and chart-deception lists, the severity scale, and the report template.

The bar for everything below: **a finding is only worth raising if you can show the evidence.**
Quote the offending line or name the slide, say why it's wrong, and — for any "this is false"
claim — cite the source that settles it. A roast that asserts without proof is itself slop.

---

## The roast stance — adopt all three before you read a word

1. **Every number is guilty until you've read its source.** Not "until it has a citation" — a
   citation to a blog that cites a slide that cites nothing is still unsourced. Read laterally to
   the primary source and confirm the value, the context, and the date. Manufactured precision
   ("improves retention by 47%") with no traceable origin is itself a finding.
2. **The world expert on this topic is in the front row.** Whatever the talk overclaims, assume
   one person in the room knows the real number, the failed replication, the counterexample, or the
   correct attribution — and will say so in Q&A or on social media afterward. Find it before they do.
3. **The audience has sat through a thousand AI-written decks.** They smell the cadence, the hollow
   conviction, the "it's not just X, it's Y," the stock-gradient hero. Every tell you leave in costs
   the speaker credibility they spent the whole talk earning.

Roast is adversarial and fair, and it works the whole list. **Do not manufacture findings.** A genuinely clean
talk gets a short report that says so. Inventing a problem to look thorough is the same failure as
missing a real one.

---

## The seven finding categories

### 1. Untrue or unverifiable (the headline)

- **A number that's wrong** — doesn't match its primary source, is stale, or was rounded into a
  different claim ("46.8%" voiced as "nearly half" is fine; "46.8%" shown as "over 50%" is a lie).
- **A number with no source at all** — the user's explicit ask. Flag every standalone statistic that
  can't be traced to a checkable primary source. "Studies show," "most companies," "experts agree,"
  "up to 10×" with no citation are all this.
- **A claim that's false** — verifiably contradicted by the record: a product that doesn't do what's
  claimed, a date that's wrong, a "first ever" that wasn't, a capability overstated.
- **A misattributed quote or a fabricated study** — see the bank below. Misattribution is the most
  common and most catchable error in keynote talks.
- **A myth taught as fact** — Mehrabian 7-38-55, Cuddy power-posing, "60,000× faster," learning
  styles, the goldfish attention span. Cross-check the bank and `../talk/references/sources.md`.
- **A projection or model output stated as an observed fact** — "AI will do X by 2027" presented in
  the same voice as a measured result. Flag the missing hedge.

### 2. Internal contradiction & drift

- **Number drift** — the same statistic appears as two values across slides/notes/brief.
- **Claim contradiction** — slide 3 says the thing slide 9 denies; the throughline argues one shift
  and a mid-section argues the opposite.
- **Terminology drift** — one concept wears three names; the audience can't tell if they're the same.
- **Throughline betrayal** — a beat that doesn't serve the ≤15-word throughline, or quietly
  contradicts it. (Coherence drift across artifacts is `talk-refine`'s job; here you flag drift that
  is *logically* wrong, not merely inconsistent labeling.)
- **Timeline / causality that doesn't hold** — "because of that" beats that don't actually follow.

### 3. AI slop & voice tells (run keep-it-human adversarially)

Run gates A–E from `../talk/references/keep-it-human.md`, but in roast mode: name **every** tell with
its evidence, don't stop at three. The accumulation rule still decides severity (a cluster is a
rewrite; one survivor in opinionated prose is a note).

- **Gate B lexical/structural** — `delve`, `leverage`, `seamless`, `robust`, `tapestry`, "in today's
  fast-paced world," "it's not just X, it's Y," "from X to Y" sweeps, forced rule-of-three with a
  vague third item, bookended self-summary. Grep the artifacts for the lexical list; read aloud for
  the structural ones.
- **Gate A hollow conviction / false balance** — universal claims with no concrete anchor; sections
  that refuse a stance; the competitor test (could any other speaker publish this verbatim?).
- **Gate C visual slop** — AI blue→purple gradients, equal-weight icon-card rows, emoji bullets,
  stock-photo clichés, AI-image artifacts, default component-kit look, charts that don't prove their
  title. Check the rendered deck, not just the spec.
- **Hollow specificity** — a number or name that looks concrete but means nothing ("we saw a 3×
  improvement" in what, measured how, against what baseline).

### 4. Easy-to-spot errors (low-hanging fruit a careful read catches)

- **Typos, misspellings, grammar** — especially in titles and on slides, where they're largest.
- **Broken math** — percentages that don't sum, a "3×" that's a 2×, a CAGR that doesn't compute, a
  unit error (ms vs s, MB vs GB), a chart whose numbers contradict its caption. Recompute by hand.
- **Numbers that don't agree with each other** — a pie that sums to 103%, a funnel whose stages don't
  reconcile, two slides whose totals can't both be true. Catchable with zero web access: check the
  deck's numbers against *each other*, not only against an outside source.
- **Wrong proper nouns** — misspelled product/person/company names, a framework credited to the
  wrong author, a version number that doesn't exist.
- **Placeholders that survived** — lorem ipsum, `TODO`, `[insert stat]`, "XX%", an empty chart, a
  slide whose visual is described but never made.
- **Formatting inconsistency** — a key term capitalized three ways, smart-quote/dumb-quote mixing,
  a date written `2024` and `2025` for the same event.

### 5. Logic holes & Q&A landmines (what gets challenged from the floor)

- **Fallacies** — see the list below; the common ones in talks are correlation→causation,
  survivorship bias, cherry-picking, and false dichotomy.
- **The unanswered obvious objection** — the "but what about X?" any practitioner will raise, left
  unaddressed. (The audience map in `01` named the resistance — check it's actually answered.)
- **The "so what" gap** — a claim or chart with no payoff; a fact that doesn't move the argument.
- **Overclaim the evidence doesn't carry** — "this proves" on top of one anecdote or an n=12 study.
- **Write the three hardest questions** the talk invites and whether it can answer them.

### 6. Risk & reputation landmines

- **Claims about named competitors or people** that are wrong, unfair, or defamatory — anything
  stated as fact about a third party that you can't source is a legal and reputational risk.
- **Confidential / NDA / unreleased specifics** — internal numbers, customer names, unannounced
  features that shouldn't be on a public slide.
- **Chart deception** — see the list below; a truncated y-axis or a cherry-picked window reads as
  dishonest the moment someone notices.
- **Uncredited borrowing** — a framework, diagram, or distinctive phrasing lifted without credit;
  a "borrowed" slide layout that's recognizably someone else's.
- **Ages badly** — a model name, price, benchmark, or "latest" claim that's already stale as of the
  talk date, or will be within weeks. Date-stamp the volatile claims.
- **Tone landmines** — a joke or analogy that punches down, a culturally unsafe reference, an
  example that alienates part of the room.
- **Unqualified legal / medical / financial claims** — advice-shaped statements that need a caveat.

### 7. Structural & format mismatch

- **Time budget vs content** — more words than the slot holds at ~130–150 wpm with 15–20% reserved
  for pauses. A 30-minute deck of 60 dense slides won't land; flag the overrun, don't trust the
  speaker to "talk faster."
- **Type mismatch** — a "workshop" that's a lecture with no practice; an online talk that never
  re-hooks; a lightning slot carrying three ideas.
- **Accessibility that undercuts credibility** — back-row-illegible type, contrast failures, a chart
  that dies in grayscale. (Detailed look-checks belong to `talk-slides-style`; here you flag the
  ones bad enough to lose the room.)

---

## The fact-check procedure (apply to every number and named claim)

1. **Find the claim's real origin, not its nearest citation.** Read laterally (SIFT): leave the talk,
   search the claim, find who actually produced the number. A stat "from a McKinsey report" that no
   one can locate in any McKinsey report is unsourced, however confidently it's cited. **Resolve any
   URL the talk itself cites** — a link that 404s or lands on a page that doesn't make the claim is its
   own finding (a broken or wrong citation), separate from "unsourced."
2. **Open the primary source and confirm three things:** the *value* (does the source say this
   number?), the *context* (does it say it about *this*? cherry-picking and quote-mining live here),
   and the *date* (is it current, or a 2019 figure presented as today's?).
3. **Recompute any derived number** — a percentage, a multiple, a growth rate, a "that means…"
   inference. Math errors are the easiest finding to prove and the most embarrassing to miss.
4. **Check attribution** — did this person actually say/write this? Misquotes and misattributions
   are rampant (the bank below). Confirm the speaker, the wording, and the occasion.
5. **State the uncertainty the talk omits** — sample size, "self-reported," "one study, not
   replicated," "projection." A real number with hidden fragility is a Q&A landmine.
6. **Record a verdict per claim:** `verified` · `wrong (→ correct value)` · `unsourced` ·
   `broken citation (→ dead/wrong link)` · `misattributed (→ real source)` · `myth` ·
   `needs human check` (you couldn't reach a primary source and won't guess). Never upgrade a guess
   to "verified."

Prefer the **Perplexity MCP tools** when available (`perplexity_search` for the source URL,
`perplexity_research` for contested claims, `perplexity_ask` for a quick cited check); degrade to
`WebSearch` / `WebFetch`. When you can't reach a primary source, the verdict is `needs human check`,
not silence — list it as a thing the speaker must verify before standing up.

---

## Zombie-stat & misattribution bank (starter set — verify, don't trust this list blindly)

High-frequency offenders in tech/business/keynote talks. **Re-verify each against a live source when
you cite it** — treat this bank as the place to look first, then confirm through the fact-check
procedure. Every entry states the *correction*, because the roast must survive its own roast. Quote
Investigator (quoteinvestigator.com) is the canonical first stop for the misattributions; the stats
cite their primary papers.

**Misattributed quotes**

- *"The definition of insanity is doing the same thing over and over and expecting different
  results."* — **not Einstein.** No record in his papers or letters; the Einstein attribution
  surfaced only around 1990. The earliest documented appearances are in 1981 twelve-step recovery
  circles (an Al-Anon meeting report, then a Narcotics Anonymous pamphlet).
- *"If I had asked people what they wanted, they would have said faster horses."* — **no evidence
  Henry Ford said it.** No contemporaneous source (he died in 1947); first pinned on him around 1999.
- *"It is not the strongest of the species that survives… but the one most adaptable to change."* —
  **not Darwin.** A 1963 paraphrase of Darwin by management professor Leon C. Megginson, later
  stripped of its framing and reassigned to Darwin himself.
- *"Be the change you wish to see in the world."* — **not a documented Gandhi quote** (it appears
  nowhere in his collected works); the earliest documented form is credited to educator Arleen
  Lorrance, 1974. The longer 1913 passage it supposedly paraphrases means something different.
- *"Good artists copy; great artists steal."* — popularized by Steve Jobs; **the Picasso attribution
  is unverified**, and the sentiment predates both (T.S. Eliot, 1920). Credit Jobs's usage, not Picasso.
- *"Culture eats strategy for breakfast."* — widely pinned on Peter Drucker; **no primary source in
  his work** (the Drucker Institute confirms he never said it; earliest documented use is a 2000
  headline). Treat it as apocryphal.

**Zombie stats**

- **"Communication is 7% words / 38% tone / 55% body language" (Mehrabian).** Applies only to
  messages about *feelings and attitudes* — the original experiments used words that conflicted with
  tone and face. Mehrabian's own site says the equations don't apply otherwise. Not a law of all
  communication.
- **"You have 8 seconds of attention — less than a goldfish."** The 2015 Microsoft-cited figure
  traces only to a commercial SEO infographic (Statistic Brain), not to any study; the BBC found no
  research behind it, and the goldfish comparison is invented. Drop it.
- **"The brain processes images 60,000× faster than text."** No primary source; fabricated marketing
  lore (it traces to a 1982 *Business Week* ad). For the real memory advantage of images, cite Paivio
  dual-coding / the picture-superiority effect — and note that's about retention, not speed.
- **"We use only 10% of our brains."** Neuroscience myth; the whole brain is active (imaging shows
  activity everywhere, even in sleep).
- **"Learning styles (visual/auditory/kinesthetic) improve outcomes when matched."** The meshing
  hypothesis isn't supported by controlled evidence (Pashler et al., 2008); widely taught, repeatedly
  failed to replicate.
- **"It takes 21 days to form a habit."** A misreading of Maxwell Maltz's 1960 *Psycho-Cybernetics*
  (about self-image after surgery, not habits); Lally et al. (2010) found a *median* of ~66 days,
  range 18–254.
- **"Power posing for two minutes changes your hormones / success."** The physiological and
  behavioral effects failed to replicate (Ranehill 2015; Carney disavowed it); only the self-reported
  "felt powerful" survived.
- **"The Chinese word for 'crisis' is 'danger' + 'opportunity.'"** A popular linguistic myth (the
  second character is closer to "incipient moment / crucial point," per sinologist Victor Mair).

**Pattern flags (no list can be complete — catch these by shape)**

- **Suspiciously round numbers** — "90% of startups fail," "80% of success is showing up." Round,
  memorable, and usually unsourced or laundered.
- **Citation laundering** — claim → blog → slide → nothing. The number gets more confident at each
  hop and never gains a source.
- **The undated "recent study"** — no author, no year, no link. Treat as unsourced until proven.

---

## Logical-fallacy checklist (the ones that actually show up in talks)

Correlation presented as causation · survivorship bias (only the winners in the sample) ·
cherry-picking / Texas sharpshooter (the favorable window or subset) · false dichotomy (only two
options when there are more) · strawman (the weak version of the opposing view) · appeal to
authority or to novelty ("it's new, therefore better") · hasty generalization from one anecdote ·
base-rate neglect (a scary percentage with no denominator) · post hoc (it happened after, so it was
caused by) · nirvana fallacy (rejecting a real fix because it isn't perfect) · motte-and-bailey
(retreat to the defensible claim when challenged, advance the bold one otherwise).

## Chart-deception checklist (look at the rendered chart, not the spec)

Truncated / non-zero baseline on a bar chart (exaggerates difference) · dual y-axes manufacturing a
correlation · inconsistent or reversed scales · a cherry-picked time window that hides the trend ·
switching between % and absolute counts to flatter · missing denominator / baseline · 3-D or
exploded pie distortion · area or bubble sized by the wrong dimension · a correlation chart captioned
as causation · an unlabeled axis or a missing unit.

---

## Severity scale

- 🔴 **Blocker** — false, fabricated, misattributed, defamatory, or flatly embarrassing. Will get the
  speaker caught on stage or online. Fix before presenting, full stop.
- 🟠 **Weak spot** — unsourced, overclaimed, a logic hole, or a question the talk can't answer. Won't
  end the talk, but a sharp audience member exposes it. Fix or arm the speaker with an answer.
- 🟡 **Slop / polish** — AI-voice tells, clutter, a soft assertion title, a typo. Costs credibility by
  accumulation, not individually. Clean up.
- 🟢 **Note** — a judgment call, a stylistic risk, or something only the speaker can decide. Flag and
  move on; not a defect.

An unsourced number the argument leans on is 🔴; an unsourced throwaway aside is 🟠 — flag every one of
them regardless. And anything that would embarrass the speaker but fits none of the seven categories
still earns at least a 🟢 Note: the catch-all is real, not a formality.

---

## Report template — `06-roast-report.md`

```markdown
# Roast Report — <talk title>

- Working dir: <abs path>
- Roasted: <which artifacts existed and were checked> · Could NOT check: <what was missing>
- Roast date: <date>  (claims verified as current as of this date)

## Verdict
**<Ship it / Fix the blockers first / Not ready>** — <one honest sentence>.
Blockers: <n> · Weak spots: <n> · Slop: <n> · Notes: <n>
> Gate: any 🔴 caps the verdict at "Fix the blockers first." "Ship it" needs zero 🔴 and no unresolved
> ⏳ load-bearing claims.

## Fact-check ledger (every number & named claim)
| Claim (as stated) | Where | Verdict | Primary source / correction |
|---|---|---|---|
| "improves retention 47%" | slide 6 | 🔴 unsourced | no traceable origin — cut or source |
| "Einstein: insanity is…" | slide 12 | 🔴 misattributed | not Einstein; drop the attribution |
| "46.8% of teams…" | slide 4 | ✅ verified | <primary URL>, figure 2, 2025 |
| "AI replaces X by 2027" | slide 9 | 🟠 projection-as-fact | model output; add the hedge |
…one row per number and named claim. Verdict ∈ {✅ verified | 🔴 wrong | 🔴 unsourced |
🔴 misattributed | 🔴 myth | 🟠 projection-as-fact | ⏳ needs human check}

## Findings, by severity
| Sev | Category | Where | What's wrong | Recommended fix |
|---|---|---|---|---|
| 🔴 | untrue | slide 12 | misattributed quote | drop attribution or cite real source |
| 🟠 | logic hole | arc beat 3 | correlation shown as causation | add the confound or soften the claim |
| 🟡 | slop | slide 2 title | "it's not just X, it's Y" | rewrite as a plain assertion |
…one row per finding, ordered 🔴 → 🟠 → 🟡 → 🟢.

## AI-slop tells caught (keep-it-human, adversarial)
| Gate | Tell | Where | Count |
|---|---|---|---|
| B — lexical | "leverage" ×3, "seamless" | notes, slide 5 | 4 |
| B — structural | bookended self-summary | open + close | 1 |
| A — hollow conviction | "transforms everything" no anchor | slide 8 | 1 |
| C — visual | chart accent not on takeaway | deck slide 7 | 1 |
Per-gate flag count → accumulation rule verdict: <clean / cluster = rewrite>.

## What a hostile audience member asks you
1. <the hardest question the talk invites> — can the talk answer it? <yes / no — arm the speaker>
2. …
3. …

## Route the structural fixes back (roast flags; it does not fix)
| Finding | Owner sub-skill |
|---|---|
| throughline / argument / evidence | talk-content |
| assertion title / slide count / chart message | talk-slides-content |
| palette / type / accessibility | talk-slides-style |
| deck render / speaker notes / build | talk-slides-build |
| cue cards / opening-closing line | talk-delivery |
| cross-artifact label drift | talk-refine |

## Human must verify before stage
- [ ] <each ⏳ needs-human-check claim — the stat you couldn't trace to a primary source>
- [ ] <any third-party/competitor claim that needs legal or factual sign-off>
- [ ] <any confidential/unreleased detail to confirm is cleared for a public room>
```
