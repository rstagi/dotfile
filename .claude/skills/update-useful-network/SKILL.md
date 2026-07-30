---
name: update-useful-network
description: >-
  Update the Useful Network in Notion — add an interesting/useful contact
  or log a conversation with an existing one. Use when the user says "add this contact",
  "add X to my network", "add this interesting person", "I just met <person>",
  "log my chat with <person>", "update the useful network", or pastes an intro/
  email/Granola meeting for someone worth keeping in the network (founders,
  operators, potential hires, advisors, partners — NOT investors; investors go to
  update-vc-network). Pulls context from Gmail and Granola, dedupes, and only
  writes after confirmation.
---

# Update the Useful Network (Notion contacts DB)

This skill maintains the "✅ Useful Network" — the CRM for interesting,
useful people who aren't investors (founders, operators, prospective hires,
advisors, partners, community contacts). It handles two jobs:
1. Add a new contact that isn't in the DB yet.
2. Log a contact — update an existing person after you talk to them.

Everything is written to Notion. Never create local markdown for this.

Not for investors. VCs/investors belong in the VC network — use the
update-vc-network skill instead. If someone is clearly an investor, say so and
point to that skill.

## The data model (READ THIS FIRST)

One single database — no linked/fund DB, no Status field. One row per person.

"✅ Useful Network"
- data source: collection://3598636c-9870-4bb4-adf1-8ad6d2ff3ec4
- parent page "Network": 309f341af2a38082a657cb865e888f93
- default view: https://app.notion.com/p/c5716314c4024ef48e7332db3d881ede?v=5634b9a13c984eb38bfeb05c4210e7e9
- Person pages have no body content — everything lives in properties.

Always fetch the data source before writing to reconfirm the schema (options can change).

### Properties (exact names — copy them verbatim)

| Property | Type | Notes |
|---|---|---|
| Name | title | Person's full name. |
| Email | text | |
| Company | text | Free-text company/org name, e.g. Insight Partners. |
| Role | text | Job title / what they do. |
| Location | multi-select (cities) | Where the person is based. Controlled options — see below. |
| Intro Source | text | How the contact came to be — who intro'd them, or the event/context where you met them (e.g. Avatar AI Summit (Milan, Jul 2026)). |
| LinkedIn | url | |
| Phone Number | text | |
| Last Contact | date | Use expanded keys when writing (see "Writing" below). |
| Update & Next Steps | text | Running dated log: what happened + next step, usually in Italian. |

Intro Source is free text. Common values: a person who made the intro
(Giacomo, Roberto, Edo, Rebecca, IFF, Hampus (point blue vc), Alecla7, Sprints),
or the event/context where you met them (e.g. Avatar AI Summit (Milan, Jul 2026)).

Location (city) options: London, Rome, Ghent, Milan, Paris, Berlin,
Amsterdam, Madrid, New York, Copenhagen, Munich, Zurich, Antwerp, Brussels,
Riyadh, Turin, Athens, Bonn, Malmo, Los Angeles, Vienna, Luxemburg. If a needed
city isn't listed, ask before inventing one.

## Conventions

- One row per person. If several people from the same org are worth adding,
  create a separate row for each; put the org in Company on each.
- Update & Next Steps = prepend a dated log. Never overwrite. Add the new
  entry on top, keep history below, separated by ---. Format (newest on top):
      2026-07-13 — <short update in your usual style/Italian>
      → next: <next step>
      📝 Note meeting: <link, if there's a Granola/Notion meeting>
      ---
      <existing content, unchanged>
  Match the language of the existing entries (usually Italian). Omit the
  "Note meeting" line if there's no meeting to link.
- Linking meeting notes. When there's a Granola meeting, link it in the log
  entry. Prefer the Notion Granola-archive page if it exists; the archive sync is
  manual and often lags, so if today's meeting isn't in Notion yet, use the
  Granola app deep link https://notes.granola.ai/d/<meeting_id> and note it
  can be swapped for the Notion page once it syncs.
- Enrichment from Gmail/Granola only. Pull email / company / role / intro
  source / phone (from signatures) from Gmail and the meeting. Do not
  web-search for missing LinkedIn/phone — just ask for what's missing.
- Always confirm before writing. Show the exact fields you'll set/change and
  wait for a yes.

## Writing to Notion — format notes

- Create a person: notion-create-pages with parent
  { "type": "data_source_id", "data_source_id": "3598636c-9870-4bb4-adf1-8ad6d2ff3ec4" }.
  Leave content empty (person pages are blank).
- Company, Role, Email, Phone Number, Intro Source: plain text strings.
- Multi-select value (Location): JSON-array string, e.g. "[\"Milan\"]" or "[\"Milan\",\"London\"]".
- LinkedIn: plain URL string.
- Last Contact (date) — use expanded keys, not Last Contact:
  "date:Last Contact:start": "2026-07-13", "date:Last Contact:is_datetime": 0.
- Update an existing person: notion-update-page with command: "update_properties".
  For the log field, read the current value first, then write the full prepended string back.

## Workflow 1 — Add a new contact

1. Dedup first. Query the DB by the person's name and by email domain. If a
   plausible match exists, surface it and ask whether to update that row
   (Workflow 2) instead of creating a duplicate.
2. Enrich from Gmail + Granola. Search Gmail and Granola for the person /
   company / email domain. Extract: full name, email, company, role, who intro'd
   them (Intro Source), phone from signature if present, and any base city (Location).
3. Ask for the gaps. Show what you gathered and ask only for the still-missing
   fields (LinkedIn, phone, Location, Role, Intro Source if unknown).
4. Confirm, then create the person row with all fields. Set Last Contact if
   there's already been contact, and link any meeting note in Update & Next Steps.
5. Return the new page URL.

## Workflow 2 — Log a contact with an existing person

1. Find the person in the DB (by name / company / email). If they're not there,
   switch to Workflow 1 first.
2. Pull the meeting from Granola. Use the meeting date as Last Contact. If
   there's no Granola meeting, ask for the gist and date.
3. Draft the log entry. Write a concise dated update + next step (Italian unless
   the row is in English). Prepend it to Update & Next Steps (keep existing
   history), and link the meeting note.
4. Confirm, then update the person: Last Contact (meeting date) and the prepended
   Update & Next Steps. Return the page URL.

## Guardrails

- Never write without an explicit confirmation of the exact changes.
- Never overwrite Update & Next Steps — always prepend and preserve history.
- Don't invent Location city options; ask if a needed city is missing.
  (Intro Source is free text — no constraint.)
- Keep investors out — route them to update-vc-network.
- If dedup is ambiguous, ask rather than risk a duplicate.
