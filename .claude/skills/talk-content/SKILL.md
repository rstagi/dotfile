---
name: talk-content
description: |
  Decide and write the content of a conference talk — the throughline, audience map, narrative skeleton, evidence, analogies, hook, arc, and close — and produce a talk-brief that the rest of the suite builds on. Use whenever the user is planning a talk and asks "help me figure out what my talk should say", "what's the throughline", "shape my conference talk", "draft my talk content", "outline my keynote / webinar / workshop / lightning talk", or invokes `/talk-content`. Trigger on phrases like "I'm giving a talk and don't know where to start", "what's the one idea", "structure my talk", "what story should I tell", "help me find my angle for this conference" — even if the user doesn't say "skill" or "talk-content" by name. This skill writes the content brief (01-content-brief.md); it does not design slides, build a deck, or coach delivery.
allowed-tools:
  - Read
  - Write
  - WebSearch
  - WebFetch
  - Bash
---

# talk-content — decide what the talk actually says

This is step 1 of a six-skill talk suite. It takes a vague topic ("I want to talk about agent evals") and turns it into a content brief with a single throughline, a chosen narrative skeleton filled with beats, traced evidence, validated analogies, a hook, and one memorable moment with one call to action. It reads nothing upstream and writes `01-content-brief.md` into the talk's working directory; `talk-slides-content` reads that file next.

The hard rule of the whole suite: the throughline, the memorable moment, and the named specifics (names/dates/numbers/places) decided here flow downstream **unchanged**. They are the human spine of the talk. No stage after this one rewrites them — a later stage that wants to change the argument loops back here; it does not edit the spine itself. Everything you do in this skill is in service of getting them right before a single slide exists.

## When to use / honest skip

Use this when someone has a talk to give and hasn't yet locked what it argues — no throughline, no chosen structure, no decided close. That's most talks at the start.

**Honest skip path.** Skip — or stop early and hand off — if any of these is true:
- The user already has a finished outline or brief and only wants slides → go straight to [`talk-slides-content`](../talk-slides-content/SKILL.md).
- The "talk" is a 3-minute internal status update or a demo with no argument → it doesn't need a throughline; tell them so and stop.
- The user wants delivery coaching for content that's already set → [`talk-delivery`](../talk-delivery/SKILL.md).
- There is genuinely no point of view yet and the user won't commit to one. A talk with no defensible stance is the thing this suite refuses to dress up. Push once (Step 2); if there's still no there there, say so plainly rather than manufacturing a thesis.

## Inputs

Gather these before doing anything else. Ask in one batch; don't interrogate one question at a time.

1. **Topic** — the rough subject, in the user's own words.
2. **Talk TYPE** — one of: online webinar · in-person stage keynote · hands-on workshop · lightning/Ignite. This reshapes structure more than anything else (see [`references/talk-type-and-time.md`](references/talk-type-and-time.md)).
3. **TIME budget** — minutes on stage, plus whether Q&A is inside or outside that.
4. **Audience** — who's in the room, their level, why they showed up.
5. **Venue / conference** — name, track, what else is on the program (so the talk doesn't duplicate the slot next door).
6. **Speaker's point of view** — what does *this* speaker believe about the topic that not everyone does? If they can't answer, that's the first real work (Step 2).

**Resolve the working directory.** Default `~/talks/<slug>/` where `<slug>` is a kebab-case short title. Auto-detect an existing project: if `~/talks/<slug>/01-content-brief.md` or any `0N-*.md` already exists, you're editing an existing talk — read what's there first. The user can override the directory at runtime. Create it if absent:

```bash
TALK_DIR="${TALK_DIR:-$HOME/talks/<slug>}"
mkdir -p "$TALK_DIR"
ls -la "$TALK_DIR"   # detect prior artifacts
```

## Process

### Step 1 — Gather inputs and resolve the working dir

Collect the six inputs above. If the talk type, time, or audience is missing, ask — they change everything downstream and guessing wastes the user's time. Create or detect `$TALK_DIR`. Note any prior artifacts you found so you don't overwrite a brief the user has already edited.

### Step 2 — THROUGHLINE GATE (hard)

A talk carries **one** idea. State it as a single sentence, ≤15 words, framed as a **shift in understanding** — not a topic label. "Agent evals" is a topic. "You're measuring the wrong thing: judge the trajectory, not the final answer" is a throughline. (Anderson, *TED Talks* — sources.md.)

Generate **5–10 candidate throughlines**, each a real shift the speaker could defend. Vary the enemy each one names. Then pick **one** with the user (or pick the strongest and say why, if the user defers). Write it at the top of the brief.

This is a **gate**: do not proceed to structure until the throughline is written and the user has agreed it's the shift they want to cause. Everything in the talk hangs on it; in Step 6 and Step 8, anything that doesn't serve it gets cut. If you can't generate even three candidates that say something, return to the honest-skip path — there may be no talk here yet.

### Step 3 — Audience map

Three columns, filled with specifics, not adjectives:
- **Already know** — what you can assume and therefore skip.
- **Will resist** — the objection, prior belief, or vested interest the throughline bumps into. Name it; you'll answer it in the arc.
- **Must trade** — what adopting the new idea costs them (effort, status, a comfortable habit). No idea is free; if you can't name the cost, you don't understand the audience yet.

Beware the **Curse of Knowledge** (Heath, *Made to Stick* — sources.md): the speaker has forgotten what it's like not to know this. Test the map against a named non-expert if you can.

### Step 4 — Research + creative connections

Build the evidence and the analogies. Prefer **Perplexity MCP tools** if available (`perplexity_search` for facts/URLs, `perplexity_research` for multi-source depth, `perplexity_ask` for quick cited answers) — they return citations directly. **Degrade gracefully to `WebSearch` / `WebFetch`** if Perplexity isn't loaded.

**Evidence.** For every statistic or claim you'll put on stage:
- Trace it to the **primary source** using SIFT + CRAAP (lateral reading; read past the blog to the study). See [`references/research-and-evidence.md`](references/research-and-evidence.md).
- Record the **primary-source URL**, not the secondary write-up.
- State the **uncertainty** honestly (sample size, "reported, not independently verified", "this is a projection, not an observation"). **No manufactured precision** — "improves retention by 47%" with no source is itself a slop tell. Human-scale big numbers (Rosling/Knaflic): a billion is hard; "one per person in this room" is felt.

**Analogies (the analogy subroutine).** For each thing the audience finds abstract:
1. Abstract it to its core *relation* (what depends on what), stripped of surface features.
2. Brute-force **distant domains** for a matching relation (James Webb Young's combination move; Koestler bisociation; SCAMPER; biomimicry) — see [`references/research-and-evidence.md`](references/research-and-evidence.md).
3. **Validate** with Gentner structural alignment (the deep relations map, not just the surface), then a **boundary test** (where does it break? say so before the audience finds it).
4. **Never mix two metaphor families** in one talk. Pick the one that maps deepest and live in it.

### Step 5 — Pick ONE narrative skeleton and fill its beats

Choose **one** skeleton by the talk's goal, from [`references/narrative-frameworks.md`](references/narrative-frameworks.md):
- Inspire / change a belief → **Duarte sparkline**.
- Personal transformation / a journey → **Story Spine** or **Harmon Story Circle**.
- Persuade / pitch / rally → **Raskin strategic narrative + Name-the-Enemy**.
- Analytical / decision-oriented → **SCQA / Minto**.
- Make it stick (overlay, not a standalone) → **Heath SUCCESs** as a checklist over whichever skeleton you chose.

Fill the chosen skeleton's beats with *this talk's* content. Don't blend two skeletons — a half-sparkline, half-Story-Circle talk has no shape.

### Step 6 — Design the hook, the arc, and the close

- **Hook** (~first 30–60s): a curiosity gap tied to the throughline — a question, a startling traced stat, a story in medias res, a bold claim, or a physical demo. **Never** open with your bio or the agenda. Pick from the hook menu in [`references/research-and-evidence.md`](references/research-and-evidence.md).
- **Arc**: the filled beats from Step 5, sequenced so each one earns the next, with the resistance from Step 3 answered along the way.
- **The ONE memorable moment**: the single thing they'll repeat at lunch — a demo, a prop, a reveal, a reframe, a number made physical (Bill Gates's 2009 TED mosquito jar — sources.md, cite the **2009** talk). Design it deliberately. This is what flows downstream to the slide and the delivery beat unchanged.
- **Close**: a callback to the hook + the memorable moment landing + **one** specific, scoped call to action (one thing, doable Monday — not "go change the industry"). Write the closing line by hand (keep-it-human Gate E).

### Step 7 — Budget time → content

Convert minutes to a word and idea budget at **~130–150 wpm**, reserving **15–20% for pauses, demos, and breathing**. Then apply the **type-specific** tactics from [`references/talk-type-and-time.md`](references/talk-type-and-time.md): a 5-min slot carries one idea and zero tangents; online talks re-hook every 5–8 minutes; a workshop is *mostly practice*, designed backward from a "you-do" exercise with Bloom ABCD objectives. Sanity-check the arc against the budget — if it doesn't fit, cut a beat, don't talk faster.

### Step 8 — Write the brief, then run the human gates

Write `01-content-brief.md` (template below). Then run, against your own output:
- **keep-it-human Gate A (thesis):** throughline is a ≤15-word shift; a real point of view is present; ≥1 concrete particular per section; every stat traced to a primary source; analogies map deep relations and don't mix families; type+time matched.
- **keep-it-human Gate B (voice):** read it aloud; lexical + structural linter clean; cadence varies; de-hedged; the closing beat is something only this speaker could write.

See [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md). Gate A is owned here; Gate B is shared with `talk-delivery`, and every stage also applies it to its own prose. **Accumulation rule: 3+ flags in a gate = rewrite that section, don't patch it.**

## Output: `01-content-brief.md`

The skill always produces this file. Template:

```markdown
# Talk brief — <Title>

- **Type:** <webinar | keynote | workshop | lightning>   **Time:** <N min> (Q&A <in/out>)
- **Venue/conference:** <name, track>
- **Audience:** <one line>
- **Speaker POV:** <what this speaker believes that not everyone does>

## Throughline
> <≤15 words — the shift in understanding, not the topic>

(Rejected candidates, for the record: <2–3 of the 5–10 you didn't pick>.)

## Audience map
| Already know | Will resist | Must trade |
| --- | --- | --- |
| … | … | … |

## Type & time budget
- Words ≈ <wpm × min × 0.8> · Ideas: <1 core + N supports per the type table>
- Type-specific tactics applied: <re-hook cadence / practice ratio / one-beat-per-slide …>

## Chosen skeleton: <name> — why: <one line tied to goal>
<each beat of that skeleton, filled with this talk's content>

## Hook
<the curiosity-gap opening, written out — opening line by hand>

## Outline / arc
1. … (resistance answered: <which>)
2. …

## Evidence list
| Claim | Primary source URL | Uncertainty |
| --- | --- | --- |
| … | https://… | <sample size / projection / reported-not-verified> |

## Analogies (validated)
- **<source domain → target>** — deep relation mapped: <…>; breaks down at: <boundary>.

## The memorable moment
<the one thing they repeat at lunch — demo / prop / reveal / reframe>

## Close + single CTA
<callback to hook + moment landing>
**CTA:** <one specific, scoped action, doable Monday>  — closing line written by hand.
```

## Keep it human

Before finishing, run **Gate A (thesis/content)** and **Gate B (voice/prose)** from [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) against the brief. Gate A is owned by this skill; Gate B is shared with `talk-delivery`. Apply the accumulation rule (3+ flags → rewrite). Apply the linter to *your own* prose in the brief too, not just the talk — no `delve`/`leverage`/`tapestry`, no "it's not just X, it's Y", no forced rule-of-three, no bookended summary. The full vetted bibliography lives in [`../talk/references/sources.md`](../talk/references/sources.md); cite from it, don't re-paste it.

## Reference files

- [`references/narrative-frameworks.md`](references/narrative-frameworks.md) — fill-in-the-blank beat templates + when-to-use for the six skeletons (Duarte sparkline, Story Spine, Harmon Circle, Raskin + Name-the-Enemy, SCQA/Minto, Heath SUCCESs).
- [`references/talk-type-and-time.md`](references/talk-type-and-time.md) — how type × time reshape structure: 5/15/30/45-min word & idea budgets, online re-hook cadence, lightning/Ignite, workshop instructional design.
- [`references/research-and-evidence.md`](references/research-and-evidence.md) — SIFT + CRAAP, primary-source tracing, human-scaling numbers, the analogy subroutine, hook menu, close menu, idea generation.

Shared, don't duplicate: [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) · [`../talk/references/sources.md`](../talk/references/sources.md).

## Handoff

This skill writes `01-content-brief.md`. The next skill reads it and never edits the spine — the throughline, the memorable moment, and the named specifics are locked here. A downstream stage that wants to change the argument loops back to this skill; it does not edit the spine in place.

| Direction | Skill | What flows |
| --- | --- | --- |
| → | [`talk-slides-content`](../talk-slides-content/SKILL.md) | reads `01-content-brief.md`; writes `02-slides-content.md` (slide structure: count, assertion titles, layout classes, proving visuals, word budgets, chart specs) |
| → | [`talk-delivery`](../talk-delivery/SKILL.md) | reads `01-content-brief.md` + `02-slides-content.md`; writes `03-delivery-plan.md` (may propose+apply approved edits to `02-slides-content.md`) |
| → | [`talk-slides-style`](../talk-slides-style/SKILL.md) | reads `01` + `02-slides-content.md` + `03-delivery-plan.md`; writes `04-slides-style.md` (typeface, palette, tokens, accents.css) |
| → | [`talk-slides-build`](../talk-slides-build/SKILL.md) | reads `02-slides-content.md` + `04-slides-style.md` + `03-delivery-plan.md`; writes `deck/` (custom React deck by default) [+ optional `deck.pdf` on the reveal.js/PDF fallback path] |
| → | [`talk-refine`](../talk-refine/SKILL.md) | reads all of the above; writes `05-refine-notes.md` and reconciles artifacts in place |
