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
| `sponsor_url` | no | Sponsor website. If missing, find the official site in Step 0 |
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
{ "date": "...", "sponsor": "...", "sponsor_url": "...", "venue": "...", "logo_path": "...",
  "luma_manage_url": "...", "luma_public_url": "...",
  "doc_url": "...", "deck_url": "...",
  "done": ["research", "luma_duplicate", "doc", "deck", "luma_fill"] }
```

At start, read the state file if it exists and skip every step already in `done`. Write the file after **every** step. If the file is missing but artifacts might exist (e.g. a crashed earlier run), check before creating anything new. Never create duplicates:
- Luma: in the AI Socratic Milan calendar's manage view, look for an event titled `AI Aperitivo <MONTH YEAR>`
- Doc: search Drive for `AI Aperitivo Milan Presentations — <DATE_LONG>`
- Deck: look on https://aisocratic.org/presentations for `AI Socratic Milan <MONTH YEAR>`

If one exists, adopt it, record its URL and verify its state rather than recreating it.

## Step 0 — research (sponsor site, logo, venue)

- **Site** (only if `sponsor_url` is missing): find the sponsor's official website and record `sponsor_url`. The sponsor name is always linked to it in Luma.

- **Logo** (only if `sponsor_logo` is missing): find the official logo on the sponsor's site, press kit or brand page. Use Google search in BrowserOS to locate it. Prefer a wide (horizontal) PNG or SVG, transparent or light background, readable on a light Luma theme. Convert SVG to PNG (e.g. `rsvg-convert` or `sips`). Save it under the scratchpad and record `logo_path`.
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

Create the deck from scratch with "Create new deck". **Never** use "Duplicate presentation" or "Use as template": they give a `-copy` slug and a `(copy)` name.

1. https://aisocratic.org/presentations → "Create new deck" → **Socratic event (full scaffold)**. This opens an unsaved deck at `/slides/1`.
2. **Before touching any slide**, open Menu (top-right hamburger) → Deck setup and set:
   - Event = **Generic deck (no event)** (the default). The Luma event is private, so it can't be selected.
   - Blog post left at its default.
   - Deck name `AI Socratic Milan <MONTH YEAR>`.
   - Sharing **Link**.

   Then click **Save deck**. The URL slug comes from the name at this first save and can't be changed afterwards. If you save slides first, the slug ends up as `deck`. You should land on `/slides/ai-socratic-milan-<month>-<year>/1`.
3. Close Deck setup. Click the "Socratic Dialogues" slide, then Menu → **Edit slides** → **Add slide**. The new slide is added after the current one; it's a title slide named "Slide".
4. Select that new slide and set its title (the `h2[contenteditable]`) to **Topics Coming Soon**. This matches the template's slide. Click **Save**. The final order should be:
   - Cover, Agenda, Our Mission, Guidelines, Selfie, Thanks, Intro
   - Socratic Dialogues, **Topics Coming Soon**
   - StackOverflow Live, Thank You

   The user fills in the topics later.
5. Record `deck_url` = `https://aisocratic.org/slides/<slug>`. Check on /presentations that exactly one deck has this name and that it shows "Link only".

## Step 4 — fill the Luma event

Everything is in the "Modifica Evento" form on the manage page. The description is a tiptap editor. Edit it through `document.querySelector('.ProseMirror').editor`: find each placeholder's position with `state.doc.descendants`, then use `chain().setTextSelection(...).insertContent([...text nodes with bold/italic/link marks])`. This is more reliable than typing.
- **Sponsor logo:** select and delete the placeholder paragraph's text. The `.add-block-menu` widget then moves next to that empty paragraph. Set its hidden image `input[type=file]` with CDP `DOM.setFileInputFiles` (get the objectId via `Runtime.evaluate`). The image is inserted at that spot.
- **Location:** type the address into "Luogo dell'evento" and pick the first Google suggestion.
- **Cover:** "Cambia Foto" opens a dialog. Use the `upload` tool on its "file upload" button. The upload takes a few seconds and applies immediately.
- **Clone dialog:** the date field is `dd/mm/yyyy`. Fill it, then press Tab so it gets parsed (it should show e.g. "mar 20 ott"). If the confirm button is covered by an overlay, click it via DOM.

Edit the new event's description, modeled on the filled example and replacing each template placeholder:

| Template placeholder | Replace with |
|---|---|
| `👉 This event has been sponsored by **<Sponsor(s) Here>**` | sponsor name, bold, **always** linked to `sponsor_url` |
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
