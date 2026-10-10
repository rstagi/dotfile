---
name: ai-aperitivo-prep
description: |
  Prepare the monthly AI Aperitivo (AI Socratic Milan) event end to end via BrowserOS: duplicate the private Luma template, create the Demos & Presentations Google Doc, create the "AI Socratic Milan <Month> <Year>" deck on aisocratic.org, then fill the Luma event with sponsor, venue, links and cover. Use when the user invokes `/ai-aperitivo-prep`, or asks to "prepare / set up the next AI Aperitivo", "prep the monthly aperitivo", "create the aperitivo event for <month>".
---

# /ai-aperitivo-prep — monthly AI Aperitivo setup

Run end to end without asking for confirmation. Leave the Luma event **private**: the user publishes it manually.

## Inputs

| Input | Required | Notes |
|---|---|---|
| `date` | yes | Event date (e.g. `2026-11-18`). Month and year come from it. Time stays at the template's 18:30–21:30 unless told otherwise |
| `sponsor` | yes | Sponsor name |
| `sponsor_logo` | no | Local path or URL. If missing, find it yourself (see Step 0) |
| `cover_image` | yes | Local path. Upload as given, no crop or resize |
| `venue` | no | Name + address. Defaults to the sponsor's Milan office (see Step 0) |

If a required input is missing, ask for it once, then proceed.

Derived values (English month names):
- `MONTH YEAR` → e.g. `November 2026`
- `DATE_LONG` → e.g. `November 18, 2026`; `DATE_ORDINAL` → e.g. `November 18th, 2026` (use correct st/nd/rd/th)

Only change capacity, tickets, registration questions, hosts or other settings when the user explicitly asks.

## References

| What | URL |
|---|---|
| Luma template (private) | https://luma.com/event/manage/evt-lWQYqS75P0QOAwv |
| Luma filled example | https://luma.com/yq0fwx95 |
| Doc template | https://docs.google.com/document/d/1H2czS2tQDR25PkIrlStUxMm29dsOxSKjfChmw9bW1mg/edit |
| Doc filled example | https://docs.google.com/document/d/1BvnqxRnuHWdcNYoBUUTI9k52bzLlcJWIcVNU1ENVCFE/edit |
| Decks list | https://aisocratic.org/presentations |
| Deck template (has the "Topics Coming Soon" slide) | https://aisocratic.org/slides/template-ai-socratic-milan/1 |

## Browser

Use BrowserOS neo (load the `browseros-neo` skill), not other browser tools. Call `name_session` first (e.g. `aperitivo <month>`). It's already logged into Luma, Google and aisocratic.org. If a login wall shows up, use `request_human_help`. The Luma and Google UIs are in **Italian** (e.g. "Altro" = More, "Duplica" = Duplicate, "Condividi" = Share, "File > Crea una copia" = Make a copy).

To read a Google Doc's text, the page body is a canvas, so `read` won't work. Fetch `/export?format=txt` from inside the page with `page.evaluate`.

## Resume (state file)

State lives in `~/.ai-aperitivo/<YYYY-MM>.json`:

```json
{ "date": "...", "sponsor": "...", "venue": "...", "logo_path": "...",
  "luma_manage_url": "...", "luma_public_url": "...",
  "doc_url": "...", "deck_url": "...",
  "done": ["research", "luma_duplicate", "doc", "deck", "luma_fill"] }
```

At start, read the state file if it exists and skip every step already in `done`. Write the file after **every** step. If the file is missing but artifacts might exist (e.g. a crashed earlier run), check before creating anything new. Never create duplicates:
- Luma: in the AI Socratic Milan calendar's manage view, look for an event titled `AI Aperitivo <MONTH YEAR>`
- Doc: search Drive for `AI Aperitivo Milan Presentations — <DATE_LONG>`
- Deck: look on https://aisocratic.org/presentations for `AI Socratic Milan <MONTH YEAR>`

If one exists, adopt it, record its URL and verify its state rather than recreating it.

## Step 0 — research (sponsor logo + venue)

- **Logo** (only if `sponsor_logo` is missing): find the official logo on the sponsor's site, press kit or brand page. Use perplexity or WebSearch to locate it. Prefer a wide (horizontal) PNG or SVG, transparent or light background, readable on a light Luma theme. Convert SVG to PNG (e.g. `rsvg-convert` or `sips`). Save it under the scratchpad and record `logo_path`.
- **Venue** (only if `venue` is missing): find the sponsor's main office **in Milan** (website contact page, Google Maps, LinkedIn). If they have no Milan office, stop and ask the user for the venue. That's the one allowed mid-run question.

## Step 1 — duplicate the Luma template (private)

1. Open the template manage page → "Altro" (More) → duplicate the event.
2. Set the new date (`date`, 18:30–21:30 CET/CEST) and keep visibility **Private**.
3. Rename it to `AI Aperitivo <MONTH YEAR>`, following the example's pattern (template title is `TEMPLATE - AI Aperitivo <month> <year>`).
4. Record `luma_manage_url` (`luma.com/event/manage/evt-…`) and `luma_public_url` (the short `luma.com/xxxxxxx` link shown as "Pagina dell'evento").

## Step 2 — Demos & Presentations Google Doc

1. Make a copy of the doc template (File → Make a copy, or `…/copy` URL) in the **same Drive folder** as the template.
2. Title: `AI Aperitivo Milan Presentations — <DATE_LONG>`.
3. In the body, replace `<date>` with `<DATE_ORDINAL>` and `<luma link>` with `luma_public_url`. Leave everything else (moderators, rules, table) untouched. Compare with the filled example.
4. Share → General access: **Anyone with the link → Editor**.
5. Record `doc_url` as the `…/edit?usp=sharing` link.

## Step 3 — topics deck on aisocratic.org

1. https://aisocratic.org/presentations → "Create new deck" → **Socratic event (full scaffold)**.
2. Open the deck menu (top-right hamburger) and add a **"Topics Coming Soon"** slide right after **"Socratic Dialogues"**. Copy it as-is from the deck template, whose slide order is Cover, Agenda, Our Mission, Guidelines, Selfie, Thanks, Intro, Socratic Dialogues, **Topics Coming Soon**, StackOverflow Live, Thank You. The user fills in topics later.
3. In deck settings:
   - Event = **Generic deck (no event)**. The Luma event is private, so it can't be selected.
   - Leave Blog post at its default.
   - Saved deck name `AI Socratic Milan <MONTH YEAR>`.
   - Sharing **Link**.
   - Click **Save changes**.
4. Record `deck_url` with "Copy link", or use `https://aisocratic.org/slides/<slug>`.

## Step 4 — fill the Luma event

Edit the new event's description, modeled on the filled example and replacing each template placeholder:

| Template placeholder | Replace with |
|---|---|
| `👉 This event has been sponsored by **<Sponsor(s) Here>**` | sponsor name, bold |
| `Topics of this month: <Coming soon or blog post link…>` | link text `AI Socratic Milan <MONTH YEAR>` → `deck_url`, followed by 🔥 |
| `🎤 Demos & Presentations: <Google Doc link…>` | link text `Google Doc` → `doc_url`, followed by 🔥 |
| Socratic Conversations `<Coming soon or blog post link…>` | `coming soon 🔥` (blog post isn't out yet) |
| Presentations `<Google Doc link as above>` | link text `Demos & Presentations Google Doc` → `doc_url`, followed by 🔥 |
| Sponsors `<SPONSOR LOGO HERE>` | upload `logo_path` as an image in place of the placeholder, directly above "If you want to sponsor this event…" |

The template's Sponsors section already has the Ratel logo (permanent sponsor). Keep it and only replace the `<SPONSOR LOGO HERE>` placeholder.

Then:
- **Location**: set to `venue` (pick the matching Google Maps place suggestion).
- **Cover image**: upload `cover_image` as given.
- **Save**. Confirm that visibility is still **Private**.

## Verify, then report

Re-open `luma_manage_url` and check:
- the title and date are correct
- no `<…>` placeholders are left in the description
- all three links resolve
- the logo and cover are present
- the location is set
- the event is Private

Also check that `doc_url` opens with the new title and the right sharing, and that `deck_url` shows the Topics Coming Soon slide.

Final reply (concise):
- Luma (manage + public), Doc and Deck links
- venue used, and its source if it was researched
- logo source if found by you
- anything skipped or left for the user (publishing the event)

Leave the BrowserOS tabs open for the user to inspect.
