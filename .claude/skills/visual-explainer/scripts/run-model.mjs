// Run graph model: one Loop milestone as it ran -> an explore graph plus its timeline.
// Pure: no I/O. run-facts.mjs reads the facts of either engine into the form below.
//
//   buildRun(facts, notes) -> { graph, timeline }   timeline is null when no fact has a time
//
// The plan lanes are the base: one group for each lane, one node for each work phase, the
// dependency edges and the gate node at the end. Each phase node tells what happened to it.
// The gate node holds the review, the converge pass, the hand test and the rollback.
//
// facts (times are ISO strings or null):
//   { milestone, title, repo,                     repo: "owner/name" for commit links, or null
//     phases:  [{ n, title, lane, kind, dependsOn, doneWhen }],   kind "pr-review" = the gate
//     start, end,
//     events:  [{ at, phase, status, kind, text, short?, sha? }], kind: run | setback | merge
//     attempts: { <n>: [{ attempt, model?, minutes, verify }] },
//     review:  { phase, startAt, endAt, finalVerdict, rows: [{ stage, role, provider, verdict }],
//                converge: { status, reason } | null, convergeRows: [{ phase, state, criterion }] },
//     gate:    { prUrl, reviewUrl, shipLink, summary, deviations: [text], handTest: [{ title, command }], rollback: [text] },
//     checkpoints: [{ k, at, href, done, total }] }
//
// notes (all optional, written by hand or by the gate runner). `summary` is the page subtitle.
// `story` tells the milestone in two or three sentences:
//   { title, summary, story, lanes: { <lane>: { what, why } }, joins: { <n>: text }, findings: { <n>: text }, gate: text }

// Same second: the failure comes before the retry that it caused.
const ORDER = { failed: 0, review: 1, active: 2, done: 3 };
const GATE = "gate";
const MAX_STEPS = 30;

export function buildRun(facts, notes = {}) {
  const m = facts.milestone;
  const work = facts.phases.filter((p) => p.kind !== "pr-review");
  const gatePhase = facts.phases.find((p) => p.kind === "pr-review");
  const lanes = [...new Set(work.map((p) => p.lane ?? "main"))];
  const idOf = (p) => (p.kind === "pr-review" ? GATE : `p:${p.n}`);
  const byNum = new Map(facts.phases.map((p) => [p.n, p]));
  const story = storyByPhase(facts.events);
  const attempts = (n) => facts.attempts?.[n] ?? [];
  const review = facts.review ?? {};
  const gate = facts.gate ?? {};
  const findings = notes.findings ?? {};
  const nodes = [];
  const edges = [];

  nodes.push({ id: "m", type: "group", label: `Milestone ${m}`, sublabel: clip(facts.title ?? "", 70), details: milestoneDetails(facts, notes, work) });
  for (const lane of lanes) {
    const mine = work.filter((p) => (p.lane ?? "main") === lane);
    const ln = notes.lanes?.[lane] ?? {};
    const count = sum(mine.map((p) => attempts(p.n).length));
    const min = Math.round(sum(mine.flatMap((p) => attempts(p.n).map((a) => a.minutes ?? 0))));
    nodes.push({ id: `lane:${lane}`, type: "group", group: "m", label: `Lane ${lane}${ln.what ? ` · ${ln.what}` : ""}`,
      details: [ln.why && `**Why a lane:** ${ln.why}`, `**As it ran:** ${plural(mine.length, "phase")}, ${plural(count, "attempt")}, ${min} runner minutes.`].filter(Boolean).join("\n\n") });
  }
  for (const p of work) nodes.push(phaseNode(p, story.get(p.n) ?? [], attempts(p.n), review, findings[p.n], notes.joins?.[p.n], facts.repo));
  if (gatePhase) {
    nodes.push({ id: GATE, type: "milestone", group: "m", label: `Gate · Phase ${gatePhase.n}`, sublabel: "merge gate · AR review", status: review.endAt ? "done" : review.startAt ? "review" : "todo", emphasis: true,
      details: gateDetails(gatePhase, review, gate),
      links: [
        gate.prUrl && { label: "Pull request", href: gate.prUrl },
        gate.reviewUrl && { label: "Final review", href: gate.reviewUrl },
        gate.shipLink && { label: "Ship report page", href: gate.shipLink },
        ...facts.checkpoints.map((c) => c.href && { label: `Checkpoint ${c.k} page`, href: c.href }),
      ].filter(Boolean).slice(0, 8) });
  }
  for (const p of facts.phases) {
    for (const d of p.dependsOn ?? []) {
      const from = byNum.get(d);
      if (!from) continue;
      const cross = p.kind !== "pr-review" && from.kind !== "pr-review" && from.lane !== p.lane;
      edges.push({ from: idOf(from), to: idOf(p), kind: cross ? "depends" : "flow", emphasis: cross || undefined, label: cross ? "waits on" : undefined });
    }
  }

  const timeline = buildTimeline(facts, work, story, idOf);
  const steps = walkthrough(facts, notes, work, lanes, story, timeline, idOf);
  if (timeline) timeline.stepsAt = steps.map((s) => s.at ?? null);
  return {
    graph: {
      title: notes.title ?? `Milestone ${m}, as it ran`,
      summary: notes.summary ?? `The plan lanes, with what happened to each phase.${timeline ? " Move the slider to see any moment." : ""} Click a node for details.`,
      direction: "right",
      nodes: nodes.map(clean),
      edges: edges.map(clean),
      steps: steps.map(({ at: _at, ...s }) => s),
    },
    timeline,
  };
}

// ---- the parts ----

function phaseNode(p, lines, attempts, review, finding, join, repo) {
  const setbacks = lines.filter((l) => l.kind === "setback").length;
  const merged = lines.find((l) => l.kind === "merge" && l.sha);
  const mins = Math.round(sum(attempts.map((a) => a.minutes ?? 0)));
  const rows = (review.convergeRows ?? []).filter((x) => x.phase === p.n);
  return {
    id: `p:${p.n}`, type: "task", group: `lane:${p.lane ?? "main"}`, label: clip(p.title, 46),
    tag: `#${p.n}${setbacks ? ` ⚠${setbacks}` : ""}${finding ? " ◆" : ""}`,
    sublabel: `${plural(attempts.length, "attempt")} · ${mins} min`, status: lines.at(-1)?.status ?? "todo", emphasis: finding ? true : undefined,
    details: [
      `**Phase ${p.n} — ${p.title}**`,
      p.doneWhen && `**Done when:** ${clip(oneLine(p.doneWhen), 500)}`,
      join && `**Why it waits:** ${join}`,
      lines.length && `**What happened**`,
      lines.length && lines.map((l) => `- ${clock(l.at)} ${l.kind === "setback" ? "⚠ " : ""}${l.text}`).join("\n"),
      attempts.length && `**Attempts:** ${attempts.map((a) => `a${a.attempt}${a.model ? ` ${a.model}` : ""} ${a.minutes == null ? "?" : Math.round(a.minutes)} min (${a.verify ?? "verify unknown"})`).join(" · ")}`,
      rows.length && `**Converge:** ${rows.map((x) => `${x.state} — ${x.criterion}`).join(" · ")}`,
      finding && `**Review finding:** ${finding}`,
    ].filter(Boolean).join("\n\n"),
    links: merged && repo ? [{ label: `Commit ${merged.sha}`, href: `https://github.com/${repo}/commit/${merged.sha}` }] : undefined,
  };
}

function milestoneDetails(facts, notes, work) {
  const attempts = sum(work.map((p) => facts.attempts?.[p.n]?.length ?? 0));
  const setbacks = facts.events.filter((e) => e.kind === "setback").length;
  const rounds = new Set((facts.review?.rows ?? []).map((r) => r.stage).filter((s) => /^round/.test(s))).size;
  const wall = facts.start && facts.end ? duration(Math.round((Date.parse(facts.end) - Date.parse(facts.start)) / 60_000)) : "unknown";
  return [notes.story,
    `**Run:** ${wall} wall time, ${plural(work.length, "work phase")}, ${plural(attempts, "attempt")}, ${plural(setbacks, "setback")}, ${plural(rounds, "review round")}.`]
    .filter(Boolean).join("\n\n");
}

function gateDetails(phase, review, gate) {
  const conv = review.converge;
  return [
    `**${phase.title}**`,
    gate.summary && `**Verdict:** ${gate.summary.replace(/^Verdict:\s*/, "")}`,
    !gate.summary && review.finalVerdict && `**Verdict:** ${review.finalVerdict}`,
    review.rows?.length && `**Review rounds:** ${review.rows.map((r) => `${r.stage} ${r.role} (${r.provider ?? "?"}): ${r.verdict}`).join(" · ")}`,
    conv && `**Converge:** ${conv.status ?? "?"}${conv.reason ? `. ${conv.reason}` : ""}`,
    gate.deviations?.length && `**Deviations**\n\n${gate.deviations.map((d) => `- ${clip(oneLine(d), 220)}`).join("\n")}`,
    gate.handTest?.length && `**Hand test**\n\n${gate.handTest.map((h, i) => `- ${i + 1}. ${h.title}${h.command ? `: ${code(clip(oneLine(h.command), 140))}` : ""}`).join("\n")}`,
    gate.rollback?.length && `**Rollback**\n\n${gate.rollback.map((r) => `- ${oneLine(r)}`).join("\n")}`,
  ].filter(Boolean).join("\n\n");
}

// Each node's status over time, plus the marks: the start, each checkpoint, the review, the ship.
function buildTimeline(facts, work, story, idOf) {
  const timed = facts.events.filter((e) => e.at);
  const times = [facts.start, ...timed.map((e) => e.at), facts.review?.startAt, facts.review?.endAt, ...facts.checkpoints.map((c) => c.at)].filter(Boolean).sort();
  if (!times.length) return null;
  const start = facts.start ?? times[0];
  const end = [facts.end, times.at(-1)].filter(Boolean).sort().at(-1);
  const nodes = {};
  for (const p of work) {
    nodes[idOf(p)] = (story.get(p.n) ?? []).filter((l) => l.at).map((l) => ({ at: l.at, status: l.status }))
      .sort((a, b) => a.at.localeCompare(b.at) || ORDER[a.status] - ORDER[b.status]);
  }
  const gate = facts.phases.find((p) => p.kind === "pr-review");
  const review = facts.review ?? {};
  if (gate) nodes[GATE] = [review.startAt && { at: review.startAt, status: "review" }, review.endAt && { at: review.endAt, status: "done" }].filter(Boolean);
  const merges = work.map((p) => nodes[idOf(p)].find((x) => x.status === "done")?.at).filter(Boolean).sort();
  const doneAt = (t) => merges.filter((x) => x <= t).length;
  const marks = [{ at: start, label: "Start", text: `Milestone ${facts.milestone} starts. ${plural(work.length, "work phase")} in ${plural(new Set(work.map((p) => p.lane)).size, "lane")}.` }];
  for (const c of facts.checkpoints.filter((x) => x.at)) {
    marks.push(clean({ at: c.at, label: `CP${c.k}`, href: c.href, text: `Checkpoint ${c.k}: ${doneAt(c.at)} of ${work.length} phases merged.` }));
  }
  if (review.startAt) marks.push({ at: review.startAt, label: "Review", text: "All phases merged. Converge pass and AR review of the PR." });
  if (review.endAt) {
    marks.push(clean({ at: review.endAt, label: "Shipped", href: facts.gate?.shipLink,
      text: `Final review: ${verdictOf(facts)}. Parked for the human merge.` }));
  }
  marks.sort((a, b) => a.at.localeCompare(b.at));
  return { start, end, nodes, marks, stepsAt: [], merges, total: work.length };
}

// The walkthrough: the lanes first, then the story in time, then the findings and the gate.
// Each step carries `at`: the time that the page shows for it (the end for the overview steps).
function walkthrough(facts, notes, work, lanes, story, timeline, idOf) {
  const end = timeline?.end ?? null;
  const steps = [{ title: `Milestone ${facts.milestone}, as it ran`, at: end, focus: ["m"],
    text: `${notes.story ?? ""}${timeline ? " The slider at the top moves through time. Each checkpoint is a mark on it." : ""}`.trim() || `${work.length} work phases and the gate.` }];
  for (const lane of lanes) {
    const mine = work.filter((p) => (p.lane ?? "main") === lane);
    const sb = sum(mine.map((p) => (story.get(p.n) ?? []).filter((l) => l.kind === "setback").length));
    const ln = notes.lanes?.[lane] ?? {};
    steps.push({ title: `Lane ${lane}${ln.what ? `: ${ln.what}` : ""}`, at: end, focus: [`lane:${lane}`, ...mine.map(idOf)],
      text: `${ln.why ? `Why a lane: ${ln.why} ` : ""}As it ran: ${plural(mine.length, "phase")}, ${plural(sb, "setback")}.` });
  }
  const marks = timeline?.marks ?? [];
  for (let i = 1; i < marks.length; i += 1) {
    const [from, to] = [marks[i - 1].at, marks[i].at];
    const inside = (t) => t && t > from && t <= to;
    const changed = work.filter((p) => (story.get(p.n) ?? []).some((l) => inside(l.at)));
    const merged = work.filter((p) => inside(timeline.nodes[idOf(p)].find((x) => x.status === "done")?.at));
    const bad = work.flatMap((p) => (story.get(p.n) ?? []).filter((l) => l.kind === "setback" && inside(l.at)).map((l) => `#${p.n} ${l.short ?? l.text}`));
    const label = marks[i].label;
    const text = [
      `${clock(from)}–${clock(to)} UTC. ${marks[i].text ?? ""}`,
      merged.length && `Merged: ${merged.map((p) => `#${p.n}`).join(", ")}.`,
      bad.length && `Setbacks: ${bad.slice(0, 4).join("; ")}${bad.length > 4 ? `; and ${bad.length - 4} more` : ""}.`,
    ].filter(Boolean).join(" ");
    const title = label === "Shipped" ? "Ship" : label === "Review" ? "To the review" : /^CP\d+$/.test(label) ? `Up to checkpoint ${label.slice(2)}` : label;
    const focus = label === "Shipped" ? [GATE] : [...changed.map(idOf), ...(label === "Review" ? [GATE] : [])];
    steps.push({ title, text: clip(text, 600), at: to, focus });
  }
  for (const [n, text] of Object.entries(notes.findings ?? {})) {
    steps.push({ title: `Finding on Phase ${n}`, text: clip(text, 600), at: end, focus: [`p:${n}`, GATE] });
  }
  if (facts.phases.some((p) => p.kind === "pr-review")) {
    steps.push({ title: "Your turn", text: notes.gate ?? "Install, run the hand test in the gate node and merge.", at: end, focus: [GATE] });
  }
  // Keep the last step (the gate) when the walkthrough is too long.
  return steps.length > MAX_STEPS ? [...steps.slice(0, MAX_STEPS - 1), steps.at(-1)] : steps;
}

// ---- helpers ----

// On a PR of the authenticated user, GitHub gets COMMENT: the ship summary has the real verdict.
function verdictOf(facts) {
  return facts.gate?.summary?.match(/Verdict:\s*([A-Z_]+)/)?.[1] ?? facts.review?.finalVerdict ?? "done";
}

function storyByPhase(events) {
  const story = new Map();
  for (const e of events) {
    if (!story.has(e.phase)) story.set(e.phase, []);
    story.get(e.phase).push(e);
  }
  for (const lines of story.values()) lines.sort((a, b) => (a.at ?? "").localeCompare(b.at ?? "") || ORDER[a.status] - ORDER[b.status]);
  return story;
}

// Inline code, unless the text has a backtick of its own: then plain text.
function code(text) { return text.includes("`") ? text : `\`${text}\``; }
function clock(iso) { return String(iso ?? "").slice(11, 16) || "--:--"; }
function sum(xs) { return xs.reduce((a, b) => a + b, 0); }
function plural(n, word) { return `${n} ${word}${n === 1 ? "" : "s"}`; }
function oneLine(s) { return String(s ?? "").replace(/\s+/g, " ").trim(); }
function clip(s, n) { s = String(s ?? ""); return s.length <= n ? s : `${s.slice(0, n - 1).trimEnd()}…`; }
function clean(o) { return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined && v !== "")); }
function duration(minutes) {
  const hours = Math.floor(minutes / 60);
  return hours ? `${hours}h ${minutes % 60}m` : `${minutes}m`;
}
