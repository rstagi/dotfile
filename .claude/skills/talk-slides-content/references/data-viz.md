# Data viz — declutter every chart to one message

Charts are where decks go to die. A chart copied from a spreadsheet (or a dashboard, or a paper) is
built for *exploration* by an analyst with time; a slide chart must make *one* point to a tired room
in a few seconds. This file is the declutter procedure for Step 4 of the skill. Sources in
[`../../talk/references/sources.md`](../../talk/references/sources.md).

## The two ideas underneath everything

- **Maximize the data-ink ratio; kill chartjunk (Tufte).** Every drop of ink that isn't encoding
  data is a candidate for deletion. 3-D bars, drop shadows, gradient fills, heavy gridlines, dark
  borders, decorative backgrounds, redundant legends — all chartjunk, all gone. *The Visual Display
  of Quantitative Information* is the canon; **small multiples** (a grid of the same small chart
  across categories) and **sparklines** (the latter from *Beautiful Evidence*, 2006) are the
  high-density, low-junk patterns to reach for when one chart can't hold it all.
- **One message per chart (Knaflic).** Before drawing anything, write the sentence the chart proves
  — that sentence becomes the slide's assertion headline. If the chart supports two messages, it's
  two charts (or two slides). *Storytelling with Data* is the operational companion to Tufte.

## The declutter pass (run on every chart, in order)

This is Knaflic's reduce-and-focus pass, as a checklist. Each step removes noise or adds focus:

1. **Delete the chart border / frame.** The slide is the frame.
2. **Delete gridlines** (or fade them to a barely-there hairline if a couple are truly load-bearing
   for reading values). Default gridlines are noise.
3. **Delete the data markers / redundant points** unless a specific point is the story.
4. **Lighten and thin the axes.** Axis lines and ticks recede; they orient, they don't star. Drop
   the axis *titles* when the assertion headline already says what's measured.
5. **Kill the legend; label series directly.** A legend forces the eye to ping-pong between key and
   line (a contiguity failure, Mayer). Put the series name at the end of its line/bar, color-matched
   to the series. The theme's direct-label color *is* the meaning-mapped accent.
6. **Round the numbers.** "1,840ms → 210ms" beats "1,839.7ms → 210.4ms". Precision the audience
   can't act on is noise (and manufactured precision is an AI-slop tell).
7. **Title it with the takeaway sentence**, not the metric name. This is the assertion. ("p99
   dropped from 1,840ms to 210ms the week we deleted the cleverness" — not "p99 latency".)

After the pass, the chart should look almost too plain. That's correct. Then add **one** spot of
focus.

## Point with one color (preattentive attributes)

The human visual system processes a few attributes *preattentively* — before conscious attention,
in a few hundred milliseconds: **color (hue/intensity), position, size, orientation, length**. Use
exactly one of them to mark the takeaway and let everything else stay neutral:

- Render the whole series in `--talk-muted` or `--talk-ink`, then paint **the one bar / point / line
  that is the story** in `--talk-accent`. The eye goes straight there. That's signaling (Mayer)
  done with ink instead of animation.
- For a before/after comparison, the two meaning-mapped accents carry it: `--talk-accent` = before,
  `--talk-accent-2` = after. Keep that mapping identical to the rest of the deck.
- Never give every series its own bright color. A rainbow chart has no focal point; the audience
  doesn't know where to look, so they read the whole thing (= they read nothing).

## Pick the right chart (and kill the pie)

- **Replace pie / donut charts with sorted horizontal bars.** Humans compare *length* far more
  accurately than *angle/area*. Sort the bars by value (unless the categories have a natural order)
  so rank is readable at a glance. This is Knaflic's "death to pie charts." 3-D and exploded pies
  are doubly banned.
- **Bar** for comparing categories; **line** for trend over time; **dot/dumbbell** for before-after
  per category; **slope** for two-time-point change across items; **small multiples** when you have
  many series or many categories and one big chart turns to spaghetti.
- **No dual axes.** Two y-axes invite a false correlation and force the audience to track which line
  reads which scale. Split into two charts or two small multiples.
- **Honest axes.** Bar charts start at zero (a truncated bar axis lies about magnitude). Line charts
  may use a non-zero baseline if the *change* is the message — but say so and don't manufacture a
  dramatic slope from a flat trend. Trace the data to its primary source (SIFT/CRAAP) and state
  genuine uncertainty rather than faking a clean line.

## Mapping to the `talk-data` layout

The build theme's `talk-data` slide is purpose-built for this: a body-font assertion headline on top
(`h2`, capped at 38ch) and the chart filling the body (`img`/`svg` capped at 60vh inside
`<div class="chart">`). When you spec a data slide, give the build skill:

- the **assertion headline** (the one-message sentence),
- the **chart type** after the pie→bars / no-dual-axis rules,
- the **declutter notes** (what to delete) and the **one focal element** + which accent marks it,
- the **direct labels** and rounded values,
- a note that charts ship as **clean SVG** (selectable, vector, survives the PDF export and scales on
  a big screen) — not a screenshot of a spreadsheet.

## Anti-patterns (the chartjunk blacklist)

3-D anything · exploded / donut pies · heavy or dark gridlines · chart borders · drop shadows ·
gradient-filled bars · a unique bright color per series (rainbow) · legends that force eye-travel ·
dual y-axes · truncated bar baselines · decorative background images behind data · spreadsheet
screenshots (raster, tiny fonts, gridlines, the cell selection still visible) · default Excel /
Sheets / matplotlib styling shipped unedited.

When a chart trips several of these, it's a redesign — and it counts toward Gate C's "4+ tells =
redesign" in [`../../talk/references/keep-it-human.md`](../../talk/references/keep-it-human.md).
