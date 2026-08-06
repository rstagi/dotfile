---
name: daily-takeaway
description: |
  Scan today's Claude conversations and memory updates, and distill 1–2 shareable insights the user could post on X / LinkedIn / a blog. Use whenever the user asks for an end-of-day takeaway, wants to know what's worth posting today, asks for social-media-ready insights from recent work, or invokes `/daily-takeaway`. Trigger on phrases like "what should I post today", "give me an EOD takeaway", "daily insights for social", "shareable insight from today", "anything worth tweeting from this week's work" — even if the user doesn't say "skill" or "daily-takeaway" by name. Be honest when nothing is worth posting.
allowed-tools:
  - Bash
  - Read
  - Write
---

# /daily-takeaway — distill today's conversations into shareable insights

End of a working day. Read today's Claude conversations and any memory files updated today, then surface 1–2 candidate insights worth posting on X / LinkedIn / a blog.

The user runs this around 6pm CET, manually. The goal isn't to produce content every day — it's to *notice* the days when something genuinely worth saying happened, and to skip the days when nothing did.

## Why this is structured the way it is

Most days, nothing surprising happens — and saying so is more valuable than fabricating an insight. Forced posting corrodes a reputation faster than silence does, so the honest-skip path is load-bearing, not a fallback.

Citing source conversations matters because the user's name is on the post, not yours. They need to verify the framing in <30 seconds before publishing.

## Step 1 — Gather today's signal

Today = the user's local date. Compute once:

```bash
TODAY=$(date +%Y-%m-%d)
```

Two sources to scan:

**Memory files modified today:**

```bash
find /Users/rstagi/.claude/projects/-Users-rstagi-dotfile/memory -name "*.md" -newermt "today 00:00" -type f
```

Read each one (most are short, <100 lines). These represent things the user *chose to remember* — a strong signal that something happened worth keeping.

**Conversation transcripts from today:**

Transcripts live at `/Users/rstagi/.claude/projects/-Users-rstagi-dotfile/*.jsonl`. Each line is a JSON object with `type` (`user` / `assistant` / `system`), a `message` object containing a `content` array, and a `timestamp` (ISO 8601, UTC — note timezone gotcha below).

Extract today's user + assistant **text** content only — skip tool calls, tool results, and system messages. They're noisy and not the part that turns into an insight.

```bash
# Note: timestamps in transcripts are UTC; "today" in CET/CEST may cross UTC date boundaries.
# Pull the last 30 hours of UTC timestamps and let the model filter from there.
CUTOFF_UTC=$(date -u -v-30H +%Y-%m-%dT%H:%M:%S)

for f in /Users/rstagi/.claude/projects/-Users-rstagi-dotfile/*.jsonl; do
  jq -r --arg cutoff "$CUTOFF_UTC" '
    select(.timestamp >= $cutoff) |
    select(.type == "user" or .type == "assistant") |
    (.message.content // [] |
      if type == "string" then .
      else (.[] | select(.type == "text") | .text)
      end)
  ' "$f" 2>/dev/null
done
```

If the total volume is large, sample: read the longest few files fully and skim the rest. You don't need every word — you need the moments worth talking about.

If `jq` isn't available, fall back to `python3` with `json.loads` line by line. Don't load files into context via `Read` — they're too big and 95% noise.

## Step 2 — Apply the distillation rubric

For each candidate insight, ask:

- **Is it surprising or contrarian?** Generic productivity tips ("write tests!", "use git!") are noise — skip them. Look for moments where a default assumption got pushed back on, where a debugging path took an unexpected turn, or where the user said something genuinely opinionated.
- **Is it concrete?** A specific debugging session, a specific architectural decision and its reasoning, a specific bug and what made it gnarly — these travel further than generalized lessons.
- **Does it have a hook?** A post worth scrolling past needs a first line that earns the rest. If you can't write a hook for it, it's probably not the insight.

**Discard:**

- Routine work ("set up the linter", "renamed a function")
- Anything requiring deep project context the audience won't have
- Hot takes the user might regret tomorrow — if uncertain, flag as "Possible take — verify framing"

## Step 3 — Format candidates

For each insight (**1–2 max** — quality over quantity), produce all three platform variants so the user can pick the right channel:

```markdown
## Insight: <one-line working title>

**Source:** <transcript file basename + rough UTC timestamp, or memory file path>

**X (≤ 280 chars, punchy hook):**
<draft post>

**LinkedIn (3–5 sentences, narrative with a takeaway):**
<draft post>

**Blog / thread (paragraph-length thought worth expanding into a longer piece):**
<draft thought>
```

## Step 4 — Write output and print inline

Output file: `~/dotfile/insights/$(date +%Y-%m-%d).md`

- Create `~/dotfile/insights/` if it doesn't exist.
- If the file already exists for today, **append** a new section headed `## Run: $(date +%H:%M %Z)` rather than overwriting. The user might re-run later in the evening after more conversations.

Then **also print the candidates inline in the chat** — don't just say "wrote to X." The user wants to read them immediately, not open a file.

## Honest skip

If nothing met the rubric, append exactly this to the file and stop:

```markdown
## Run: <HH:MM TZ>

# Low-signal day — nothing worth posting.

<one short sentence on what today actually was, e.g.: "mostly config tweaks and dependency bumps — no surprising moments to draw on">
```

Tell the user the same thing inline. Do not invent insights to fill the slot.
