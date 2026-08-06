---
name: talk-delivery
description: |
  Draft the speaker notes and a full delivery plan for a conference talk — trigger-word cue cards written for the ear, a hand-written opening and closing line, pace/pause/emphasis marks on the key lines, a pre-talk countdown, a nerve-reframe plan, a humor check, and the context-specific tactics for an in-person stage, an online talk, or a workshop — and, because voicing the talk surfaces slide problems, propose specific edits to the slide-content spec for your approval. Use whenever the user is past content and slide structure and asks to "write my speaker notes", "help me rehearse", "how do I deliver this", "coach my delivery", "calm my nerves before the talk", "make cue cards", "I keep saying um", "prep me for the stage", or invokes `/talk-delivery`. Trigger on phrases like "I have the slide outline, now what do I say", "turn this into a talk I can actually give", "how do I not read my slides", "get me ready for the room", "online talk delivery tips", "facilitate this as a workshop" — even if the user does not say "skill" or "talk-delivery" by name. This skill writes the delivery plan (03-delivery-plan.md) and may propose approved edits to 02-slides-content.md; it does not choose fonts/colors (talk-slides-style), build the deck (talk-slides-build), or re-argue the talk (talk-content).
allowed-tools:
  - Read
  - Write
  - Edit
---

# talk-delivery — write the speech you can actually give, then coach the delivery

Stage 3 of six. You have a content brief (the throughline, the memorable moment, the named specifics) and a slide-content spec (the slide order, the assertion titles, the one idea per slide). The deck does not exist yet — slides-style and slides-build come after you. This turns those two artifacts into **speaker notes for the ear** — trigger-word cue cards, not a word-perfect script — plus a delivery plan: a hand-written opening and closing line, pace/pause/emphasis marks, a pre-talk countdown, a nerve plan, a humor check, and tactics tuned to where you are presenting. It reads `01-content-brief.md` and `02-slides-content.md` from the working dir and writes `03-delivery-plan.md`.

Writing cue cards for the ear forces you to actually voice the talk, and voicing it surfaces slide-content problems no silent read catches — a slide that just repeats your spoken sentence, a beat you say out loud with no slide behind it, a jump with no transition. So you also get a second power: **propose specific edits to `02-slides-content.md`, show the user, and apply the approved ones.** You propose, you never patch silently.

The content brief's throughline, memorable moment, and named specifics are the **spine** — they carry through here unchanged, and your proposed slide edits must never touch them. Your job is to make a human say them well, not to re-argue them. If a delivery problem can only be fixed by changing the *argument* (a different claim, a new statistic, a reordered story), that is a loop back to `talk-content`, not an edit you make.

## When to use / honest skip

Use this when the content brief and the slide-content spec both exist and the speaker now needs notes to rehearse from. Also use it standalone when someone has settled content and structure from elsewhere and just wants cue cards, a nerve plan, or delivery coaching.

**Honest skip path — do not run this skill if:**

1. **There is no content yet.** No `01-content-brief.md` and no settled argument means there is nothing to draft notes *for*. Coaching delivery on an unfinished talk produces confident delivery of a muddy idea. Route to [`../talk-content/SKILL.md`](../talk-content/SKILL.md) first.
2. **The ask is "fix my slides," not "fix my delivery."** Proposing slide-content fixes that surface while voicing the talk is in scope — but a wholesale restructure of the slides belongs to [`../talk-slides-content/SKILL.md`](../talk-slides-content/SKILL.md), and fonts/colors/look belong to [`../talk-slides-style/SKILL.md`](../talk-slides-style/SKILL.md). Route there.
3. **The user wants a verbatim teleprompter script.** This skill deliberately refuses to write one — a word-perfect script is the single most reliable path to brittle, recited, uncanny delivery (see [`references/myths-and-pitfalls.md`](references/myths-and-pitfalls.md)). If they insist after you have explained why, write it but flag the risk in the plan. Do not pretend it is good practice.

## Inputs

Gather, in this order:

- **Working dir.** Default `~/talks/<slug>/`; runtime-overridable. Auto-detect an existing talk project by the presence of earlier artifacts — if `01-content-brief.md` or `02-slides-content.md` is in the current dir or `~/talks/<slug>/`, use that dir. If you find none, ask the user for the slug / path before proceeding.
- **`01-content-brief.md`** (required) — pull the throughline, the structure/beats, the hook and close menu choices, the named specifics, the validated analogies, the evidence list, and the designed memorable moment. These are the spine; they do not change.
- **`02-slides-content.md`** (required) — pull the slide order, the assertion title per slide, the one idea per slide, the proving-visual concept, the per-slide word budget, and which slide carries the memorable moment. Cue cards align to **this slide order**, slide by slide. This is also the file you may propose edits to in Step 4.
- **From the user, if not inferable:** the venue context (in-person stage / online / workshop), the time slot, and the talk date (drives the rehearsal schedule). Ask if ambiguous; do not guess the context — it changes the whole plan.

## Process

### Step 1 — Consume the brief and the slide-content spec, resolve the working dir

Read `01-content-brief.md` and `02-slides-content.md` end to end. Write down the throughline verbatim, the beat list, the memorable moment, and the slide order with each slide's assertion title — you will align every cue card to a slide. Confirm the venue context and time slot. If you cannot find either artifact, stop and ask for the path.

### Step 2 — Draft speaker notes for the ear (cue cards, not a script)

Write the notes the way the talk will be *spoken*, not the way an essay reads — speak-then-transcribe (Paul Graham, *Write Like You Talk*; Gate B). For each slide, produce a **TRIGGER-WORD cue card**: 3–7 words that fire the memory of the point, plus the one named specific or number that anchors it. Not full sentences. The speaker owns the idea and finds the words live; the card just keeps the order. Key every card to its slide number from `02-slides-content.md` so it lands on the right slide later.

**Two lines are hand-written verbatim — the opening line and the closing line.** These are the highest-signal human moments and the hardest to wing. Write the opening so it lands the hook in the first ~15 seconds (curiosity gap, not bio/agenda) and the closing so it delivers the callback + memorable moment + the one call to action. Write them in the speaker's voice, plain, sayable. Everything between them stays as cue words.

See [`references/delivery-craft.md`](references/delivery-craft.md) for the rehearse-to-own modes and why the middle "memorized-but-brittle" zone is the trap.

### Step 3 — Mark pace, pause, and emphasis on the key lines

On the cards that carry the weight — the throughline, the memorable moment, the close, the turn of each story — add delivery marks (legend in the output template):

- `[PAUSE]` before or after the line that needs to land. The strategic pause is what frames an idea and replaces a filler word; it is the cheapest tool and the most underused.
- `(slow)` on the one sentence that matters most (drop to ~110–130 wpm against a ~190 wpm conversational baseline); `(up)` where energy rises.
- **bold** the single word in a line that takes the stress. One per line, not three.
- `[BREATHE]` where a filler would otherwise sneak in. Target ≈1 filler per minute — substitute a silent diaphragmatic breath for the "um" (Zandan / Quantified Communications).

Do not over-mark. A card dense with symbols is as un-sayable as a full script.

### Step 4 — Propose slide-content fixes (propose → approve → apply)

Voicing the talk for the ear is the first time anyone has actually *said* the slides out loud, and that surfaces structure problems a silent read misses. As you draft, flag each slide where one of these is true:

- **Redundant with speech (Mayer).** The slide puts on screen the exact sentence you will say. The eye reads it, the ear hears it, and the two compete — cut the on-screen text to the assertion title plus the proving visual.
- **A beat with no slide.** You say something out loud — a turn in the story, a transition, the setup for the memorable moment — that has no slide behind it. Propose the slide.
- **A missing transition.** Two adjacent slides jump with nothing to carry the audience across. Propose a transition slide or a bridging line.
- **A slide that says what the speaker would not say.** The assertion title or on-screen text is phrased in a register the speaker would never use aloud — propose rewording it toward the spoken voice (without touching the spine).

For each flag, write a **specific** proposed edit (which slide, current → proposed, one line of why), **show the user the full list**, and **on approval apply it with Write/Edit** to `02-slides-content.md`. Log every applied change in the plan under **"Proposed slide-content changes (applied with approval)."** If the user rejects one, drop it — do not relitigate.

**Hard boundary:** never touch the **spine** — the throughline, the memorable moment, or any named specific (names/dates/numbers/places). If the only honest fix changes the argument, do not edit `02`; write it up as a routed note ("→ loop back to talk-content") and keep your proposals inside the brief's existing argument.

### Step 5 — Leave the cue cards keyed by slide for the build

The deck does not exist yet, so there is nothing to paste notes into. Leave the cue cards in `03-delivery-plan.md` **keyed by slide number**, in slide order, so `talk-slides-build` can lift each card straight into the matching slide's note block. The deck builder lifts each card into that slide's per-slide speaker notes — in a React deck, the notes field shown by the presenter overlay (toggle `N`/`S`); on the reveal/PDF fallback, a `Note:` block. Keep the pace/pause/emphasis marks inside the card text so they survive the move. State this as the handoff; do not try to render or build anything here.

### Step 6 — Delivery coaching

Pull the substance from [`references/delivery-craft.md`](references/delivery-craft.md) and write a short, specific coaching section the speaker can act on:

- **Voice** — vocal variety as the instrument (register / prosody / pace / pitch / volume; monotone is the #1 robot tell); the strategic pause; filler substitution (sense it coming → silent breath). HAIL + the six warm-ups (Treasure); Vinh Giang's over-articulation / soft-palate drills.
- **Body** — gestures illustrative, *from thought*, inside the "power sphere" (eyes-to-navel); adaptors (face-touching, fidgeting, sway) filmed and eliminated. Plant and hold on the key line.
- **Rehearsal** — rehearse to **own it**, not recite it (Anderson's four modes; avoid the brittle middle zone). Record and watch back.
- **Nerves** — REFRAME arousal as excitement ("I am excited" beats "calm down" — Alison Wood Brooks, 2014) + box breathing (4-4-4-4) or 4-7-8; burn adrenaline physically; the body settles in ~90 seconds.

### Step 7 — Choose the context variant

Pick exactly one primary context and write its tactics from [`references/format-delivery.md`](references/format-delivery.md):

- **In-person stage** — claim the stage, purposeful movement, stillness for emphasis, find 3–4 friendly faces and rotate.
- **Online** — camera at/above eye level, dial energy up ~⅓ (video flattens it), look at the lens at the opening/key point/close, self-view OFF, stand up, 10-minute attention resets.
- **Workshop facilitation** — be the guide not the hero; time-box every activity with explicit instructions and warnings; Note & Vote / silent-then-share; count-to-7 silence after a question; ~10% buffer, finish early.

### Step 8 — Write `03-delivery-plan.md`, then run the gates

Assemble the output (template below). Before finishing, run **Gate D (Delivery)** and **Gate E (Process)** from [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md): prosody varies, gestures in the power sphere, the recording self-audit is specified, humor (if any) is found-in-material, opening + closing lines hand-written, every number verified. Apply **Gate B** to your own prose. **Do not teach the Mehrabian 7-38-55 or Cuddy power-posing claims as fact** — see [`references/myths-and-pitfalls.md`](references/myths-and-pitfalls.md). 3+ flags in a gate → rewrite, do not patch.

## Output: `03-delivery-plan.md`

The skill always produces this file in the working dir. Template:

```markdown
# Delivery Plan — <talk title>

- Throughline (from 01, unchanged): <≤15 words>
- Context: <in-person stage | online | workshop> · Slot: <NN min> · Date: <YYYY-MM-DD>
- Memorable moment (from 01): <one line — which slide it lands on and how it's delivered>

## Mark legend
[PAUSE] hold the silence · [BREATHE] silent breath instead of a filler · (slow) drop to ~110–130 wpm ·
(up) lift energy/pace · **word** stress this one word

## Opening line (verbatim — hand-written)
"<the first ~2 sentences, exactly as spoken — lands the hook in ~15s, no bio/agenda>"

## Per-slide speaker notes (cue cards, keyed by slide for the build)
### Slide 1 — <assertion title from 02>
- <trigger words> · <the named specific / number that anchors it>
- [PAUSE] before "**<key word>**"   (slow)
### Slide 2 — <assertion title from 02>
- <trigger words> · <anchor>
…one block per slide, in slide order. NOT full sentences.
(talk-slides-build lifts each block into that slide's per-slide speaker notes — a React deck by default, a reveal/PDF `Note:` block on the fallback path.)

## Closing line (verbatim — hand-written)
"<the last ~2 sentences, exactly as spoken — callback + memorable moment + the one CTA>"

## Proposed slide-content changes (applied with approval)
<one row per change actually applied to 02-slides-content.md, or "none">
- Slide <N>: <current> → <applied edit> — <why (Mayer redundancy / orphan beat / missing transition / off-voice)>
- Routed back (NOT applied here — changes the argument): <slide / beat> → loop back to talk-content because <reason>

## Pre-talk countdown
- T-24h: <tech / room check>
- T-30min: <walk the room or set up the camera frame>
- T-7min: <box or 4-7-8 breathing · burn adrenaline · hydrate>
- T-1min: <"I'm excited" reframe · rehearse the opening line word-for-word>
- T-0: <walk on · plant · [PAUSE] · deliver the opening line>

## Vocal warm-up (~3 min, run at T-30 to T-7)
<sigh "aaah" · lip trill "brrr" · pitch siren · over-articulate one tongue-twister · 3 diaphragmatic breaths>

## Nerve plan
<reframe line · breathing pattern · the 90-second fact · the 3–4 friendly faces · the recovery line if something breaks>

## Humor check
- Found-in-material moment(s): <where, or "none — and that's fine">
- Verdict per moment: keep / cut · self-deprecating not audience-mocking · relevant · travels culturally · ride the laugh, don't step on it

## Context-specific tactics
<the chosen variant's checklist from references/format-delivery.md>

## Rehearsal schedule (counts back from the talk date)
- 3–4 weeks out: full run-through to a wall, timed; record it
- 2 weeks out: cue-card pass — can you fire each slide from trigger words alone?
- 1 week out: run to one friendly human; watch the recording sound-off then audio-only
- Day before: opening + closing line only, out loud, x3; light tech check; sleep
- Day of: the countdown above

## Recording self-audit checklist
- [ ] Watched sound-OFF: no sway, no adaptors, gestures in the power sphere, stillness on the key line
- [ ] Listened AUDIO-ONLY: not monotone; pace varies; pauses present; ≈1 filler/min; breathing audible; the occasional self-correction; emotion genuinely shifts beat to beat
- [ ] Opening and closing lines land as written
- [ ] Came in under the time slot with reserve for pauses
```

## Keep it human

Run **Gate D (Delivery)** and **Gate E (Process)** from [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) before finishing. Apply **Gate B**'s read-aloud and lexical/structural checks to your *own* prose in the plan too — the coaching should sound like a person who has actually stood on a stage, not a press release. The myth guardrails (no Mehrabian, no power-posing as fact) are non-negotiable; see [`references/myths-and-pitfalls.md`](references/myths-and-pitfalls.md). Every number you cite (gesture counts, the Brooks study, filler rates) must trace to [`../talk/references/sources.md`](../talk/references/sources.md). The same restraint governs your proposed slide edits: propose, never patch silently, and never touch the spine.

## Reference files

- [`references/delivery-craft.md`](references/delivery-craft.md) — the coaching canon: Anderson (gift framing, connection, rehearsal modes), Treasure (HAIL + warm-ups), Gallo, the strategic pause, gesture research (power sphere; illustrators vs adaptors), the nerve reframe (Brooks), filler substitution (Zandan), Rodenburg's Second Circle, Vinh Giang drills.
- [`references/format-delivery.md`](references/format-delivery.md) — in-person stage vs online vs workshop, each with concrete tactics.
- [`references/myths-and-pitfalls.md`](references/myths-and-pitfalls.md) — debunks Mehrabian 7-38-55 and Cuddy power-posing (cited to `sources.md`); the uncanny-valley / canned-delivery tells; the "watch these" example library.

Shared, do not duplicate:

- [`../talk/references/keep-it-human.md`](../talk/references/keep-it-human.md) — the anti-slop gates (run D and E).
- [`../talk/references/sources.md`](../talk/references/sources.md) — the fact-checked bibliography behind every claim above.

## Handoff

Stage 3 of six. It reads the brief and the slide-content spec, writes the delivery plan, and may apply approved edits to the slide-content spec on the way through.

| Direction | Skill | What flows |
| --- | --- | --- |
| Reads ← | [`talk-content`](../talk-content/SKILL.md) | `01-content-brief.md` — throughline, beats, memorable moment, named specifics, hook/close, evidence (the spine; unchanged) |
| Reads ← | [`talk-slides-content`](../talk-slides-content/SKILL.md) | `02-slides-content.md` — slide order, assertion titles, one-idea-per-slide, word budgets; cue cards align to this order |
| Writes → (with approval) | [`talk-slides-content`](../talk-slides-content/SKILL.md) | edits to `02-slides-content.md` — Mayer-redundant slides, orphan beats, missing transitions, off-voice phrasing (never the spine) |
| Writes → | [`talk-slides-style`](../talk-slides-style/SKILL.md) | `03-delivery-plan.md` — read for context (which slide carries the moment, where the weight lands) |
| Writes → | [`talk-slides-build`](../talk-slides-build/SKILL.md) | `03-delivery-plan.md` — lifts each slide-keyed cue card into the deck's per-slide speaker notes (a React deck by default) |
| Writes → | [`talk-refine`](../talk-refine/SKILL.md) | `03-delivery-plan.md` — cross-artifact spine + cue-card-presence checks |
