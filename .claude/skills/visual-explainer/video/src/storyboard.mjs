// Storyboard: the props, the narration scenes, the screenshots and the voiceover -> the scenes of
// the video, each with its data, its timing and its captions. Zero dependencies, so node --test
// checks it without Remotion.
//
// The input is what render-video.mjs passes: { mode, props, scenes, shots }. `scenes` are the
// narration scenes in video order ([{ id, text }], scripts/narration.mjs). `shots` maps a
// screenshot id of the report page (architecture, sequence, media) to { src, width, height }.
// Each scene shows the data of its section of the report page, with the same fallbacks.

export const FPS = 30;
export const WIDTH = 1920;
export const HEIGHT = 1080;
// The content box of a scene: below the header and the scene title, above the captions.
export const CONTENT = { x: 120, y: 236, w: 1680, h: 600 };

// A narrated scene adds 0.3 s of silence before its audio and 0.3 s after it.
const AUDIO_LEAD_SECONDS = 0.3;
const AUDIO_PAD_SECONDS = 0.6;
// The minimum visual time of a scene: the time to read its data, within fixed bounds.
const MIN_SCENE_SECONDS = 4;
const MAX_SCENE_SECONDS = 20;
// The outro plays the snapcn status-cycle: at least 198 frames at 30 fps, its reference length.
const OUTRO_SECONDS = 6.6;

// What one scene can show at 1080p before it says "+N more".
const LIMITS = { table: 5, phases: 8, decisionsPerPage: 3, decisionPages: 3, rejected: 3, rabbitHoles: 7, changes: 14, featured: 4, files: 5, findings: 5, steps: 8, rollback: 4, facts: 6 };
// The hand-test terminal: characters in one row and rows in the box (the box of HandTest in scenes.jsx).
export const TERMINAL = { cols: 60, rows: 11, charsPerFrame: 1, chunkSize: 3, leadFrames: 6 };
const CHARS = { title: 90, label: 40, statusPrefix: 26 };
const ELLIPSIS = "…";

const SCENE_TITLES = {
  intro: "", tldr: "", "done-when": { plan: "Done when", ship: "Done when: planned vs delivered" }, phases: "Phases",
  architecture: "Architecture: before / after", decisions: "Decisions", "rabbit-holes": "Rabbit holes & no-gos",
  changes: "Changes, in reading order", findings: "Findings", "hand-test": "Hand test", rollback: "Rollback", outro: "",
};
// The header label of the scenes without a title.
const LABELS = { intro: "Intro", tldr: "TL;DR", outro: "Wrap-up" };
// The outro (status-cycle) opens for 24 frames, then holds each status in turn to the end.
const OUTRO_INTRO_FRAMES = 24;
const DELIVERED_STATUS = ["met", "partial", "missed", "pending"];
const SEVERITIES = ["blocker", "major", "minor", "nit", "style"];
const FINDING_STATUS = ["open", "deferred", "fixed"];
const IMPACT = ["high", "medium", "low"];
const NUMBERS = ["①", "②", "③"];

// storyboard(input, voiceover, { fps }) -> { fps, width, height, durationInFrames, scenes }.
// `voiceover` maps a scene id to its audio clip: { src, seconds }. Each board scene: { id, title,
// label, number, text, data, from, durationInFrames, audio, captions }. `label` names the scene in
// the header. `number` is the section number of
// the report page (null for intro, tldr and outro). `audio` is { src, from, durationInFrames } or null.
// `captions` are the words of the text, [{ text, startFrame, endFrame }] in frames of the scene.
export function storyboard(input, voiceover = {}, { fps = FPS } = {}) {
  const board = [];
  let from = 0;
  let number = 0;
  for (const { id, text } of input.scenes) {
    const data = sceneData(input, id);
    const audio = voiceover?.[id];
    const audioFrames = audio ? Math.ceil(audio.seconds * fps) : 0;
    const minimum = Math.round(minimumSeconds(id, data) * fps);
    const durationInFrames = Math.max(minimum, audio ? audioFrames + Math.round(AUDIO_PAD_SECONDS * fps) : 0);
    const lead = Math.round(AUDIO_LEAD_SECONDS * fps);
    // The report page numbers its sections from "Done when": the title, the TL;DR and the end have no number.
    const section = !["intro", "tldr", "outro"].includes(id);
    if (section) number += 1;
    board.push({
      id,
      title: titleOf(input.mode, id),
      label: titleOf(input.mode, id) || LABELS[id],
      number: section ? number : null,
      text,
      data: timed(id, data, durationInFrames, fps),
      from,
      durationInFrames,
      audio: audio ? { src: audio.src, from: from + lead, durationInFrames: audioFrames } : null,
      captions: captionWords(text, lead, audio ? audioFrames : durationInFrames - lead - Math.round(0.3 * fps)),
    });
    from += durationInFrames;
  }
  return { fps, width: WIDTH, height: HEIGHT, durationInFrames: from, scenes: board };
}

// sceneIndexAt(board, frame) -> the index of the scene that shows at `frame`.
export function sceneIndexAt(board, frame) {
  let index = 0;
  board.scenes.forEach((s, i) => { if (s.from <= frame) index = i; });
  return index;
}

// sceneData(input, id) -> the data that the scene shows. See each builder for its shape.
export function sceneData({ mode, props, shots = {} }, id) {
  const build = (mode === "plan" ? PLAN : SHIP)[id];
  if (!build) throw new Error(`no ${mode} scene "${id}"`);
  return build(props, shots);
}

// verdictOf(props) -> { tone: "ok" | "bad", label }: the verdict pill of a ship report. Changes
// are requested when a review asks for them or a blocker is open. Else the label names the most
// severe class of the findings that are not fixed.
export function verdictOf(props) {
  const findings = (props.reviews ?? []).flatMap((r) => r.findings ?? []);
  const blocking = findings.filter((f) => f.status === "open" && f.severity === "blocker").length;
  if (blocking || (props.reviews ?? []).some((r) => r.verdict === "REQUEST_CHANGES")) {
    return { tone: "bad", label: `Changes requested${blocking ? `, ${blocking} blocking` : ""}` };
  }
  const left = findings.filter((f) => f.status !== "fixed");
  const worst = SEVERITIES.find((s) => left.some((f) => f.severity === s));
  if (!worst) return { tone: "ok", label: "Ready to merge" };
  const n = left.filter((f) => f.severity === worst).length;
  return { tone: "ok", label: `Ready to merge, ${n} ${worst} finding${n === 1 ? "" : "s"}` };
}

// modeLabel(input) -> the mode label of the header. A ship report at a meta.checkpoint is a
// progress checkpoint.
export function modeLabel(input) {
  if (input.mode === "plan") return "Plan";
  return input.props.meta?.checkpoint ? "Progress checkpoint" : "PR review";
}

// checkpointOf(props) -> { number, done, total, since } for a ship report at a mid-milestone
// progress checkpoint (meta.checkpoint), else null. `since` is "" when absent.
export function checkpointOf(props) {
  const c = props.meta?.checkpoint;
  if (!c) return null;
  return { number: c.number, done: c.phasesDone, total: c.phasesTotal, since: c.since ?? "" };
}

// clip(text, max) -> text on one line, at most `max` characters. A cut ends at a word boundary
// with an ellipsis. A cut inside a `code` span closes the span.
export function clip(text, max) {
  const flat = String(text).replace(/\s+/g, " ").trim();
  if (flat.length <= max) return flat;
  let cut = cutAt(flat, max - ELLIPSIS.length);
  if (!isOdd(backticks(cut))) return `${cut}${ELLIPSIS}`;
  cut = cutAt(flat, max - ELLIPSIS.length - 1);
  return isOdd(backticks(cut)) ? `${cut}${ELLIPSIS}\`` : `${cut}${ELLIPSIS}`;
}

// terminalLines(steps) -> the rows of the hand-test terminal and the steps that fit TERMINAL.rows.
// Each step gives a "# n. title" row, its command rows ("$ " first, long rows wrapped) and an
// "expect" row. Returns { lines: [{ text, type, step }], shown }.
export function terminalLines(steps) {
  const lines = [];
  let shown = 0;
  for (const step of steps) {
    const rows = [
      { text: clip(`# ${step.n}. ${plain(step.title)}`, TERMINAL.cols), type: "log" },
      ...commandRows(step.command),
      ...(step.expect ? [{ text: clip(`→ ${plain(step.expect)}`, TERMINAL.cols), type: "success" }] : []),
    ];
    if (lines.length + rows.length > TERMINAL.rows && shown > 0) break;
    lines.push(...rows.slice(0, TERMINAL.rows).map((r) => ({ ...r, step: shown })));
    shown += 1;
  }
  return { lines, shown };
}

// ---- plan scenes ----

const PLAN = {
  intro: (p) => ({ chip: "mode: plan", title: p.meta.title, subtitle: p.meta.subtitle, facts: factsOf(p) }),
  tldr: (p) => {
    const t = p.tldr ?? { goal: p.goal, risks: topRisks(p.risks) };
    const risks = (t.risks ?? []).slice(0, 3).map((r, i) => `${NUMBERS[i]} ${r}`).join("  ");
    return { beats: [
      { label: "Goal", text: t.goal },
      ...(t.ask ? [{ label: "What I need from you", text: t.ask }] : []),
      ...(risks ? [{ label: "Top risks", text: risks }] : []),
    ] };
  },
  "done-when": (p) => {
    const rows = p.doneWhen?.length
      ? p.doneWhen.map((d) => ({ cells: [d.criterion, d.check] }))
      : (p.milestones[0]?.phases ?? []).filter((ph) => ph.doneWhen).map((ph) => ({ cells: [`${ph.title} — ${ph.doneWhen}`, "—"] }));
    return { columns: ["Criterion", "How we'll check"], ...limited(rows, LIMITS.table) };
  },
  phases: (p) => {
    const many = p.milestones.length > 1;
    let left = LIMITS.phases;
    let total = 0;
    const groups = [];
    for (const m of p.milestones) {
      total += m.phases.length;
      if (left <= 0) continue;
      const rows = m.phases.slice(0, left).map((ph) => ({
        title: ph.title, summary: ph.summary ?? ph.doneWhen ?? "", estimate: ph.estimate ?? "", review: ph.kind === "pr-review",
      }));
      left -= rows.length;
      groups.push({ title: many ? `Milestone ${m.number} — ${m.title}` : null, rows });
    }
    const shown = groups.reduce((n, g) => n + g.rows.length, 0);
    return { groups, more: total - shown };
  },
  architecture: (p, shots) => ({
    shot: shots.architecture ?? null,
    media: shots.media ?? null,
    summary: p.architecture.summary,
    layers: p.architecture.layers.map((l) => ({
      name: l.name, components: l.components.filter((c) => c.status !== "removed").map((c) => ({ name: c.name, status: c.status ?? "existing" })),
    })),
  }),
  decisions: (p) => {
    const cards = p.decisions.map((d) => ({
      title: d.title ?? clip(plain(d.decision), CHARS.label), decision: d.decision, why: d.why,
      rejected: d.rejected.slice(0, LIMITS.rejected).map((r) => r.option),
    }));
    const pages = [];
    for (let i = 0; i < cards.length && pages.length < LIMITS.decisionPages; i += LIMITS.decisionsPerPage) pages.push(cards.slice(i, i + LIMITS.decisionsPerPage));
    return { pages, more: Math.max(0, cards.length - LIMITS.decisionsPerPage * LIMITS.decisionPages) };
  },
  "rabbit-holes": (p) => {
    const items = (p.rabbitHoles ?? []).map((text) => {
      const lead = /^No\b/.exec(text);
      return lead ? { lead: "No", text: text.slice(2).trimStart() } : { lead: "", text };
    });
    return limited(items, LIMITS.rabbitHoles, "items");
  },
  outro: (p) => {
    const phases = p.milestones.flatMap((m) => m.phases);
    return {
      prefix: clip(`${plain(p.meta.title)} is`, CHARS.statusPrefix),
      statuses: [`planned in ${phases.length} phase${phases.length === 1 ? "" : "s"}`, "ready for review"],
    };
  },
};

// ---- ship scenes ----

const SHIP = {
  intro: (p) => {
    const checkpoint = checkpointOf(p);
    return {
      chip: checkpoint ? "mode: ship · progress checkpoint" : "mode: ship · PR review", title: p.meta.title, subtitle: p.meta.subtitle, facts: factsOf(p),
      stats: p.stats ?? null, verdict: checkpoint ? null : verdictOf(p), checkpoint,
    };
  },
  tldr: (p) => {
    const t = p.tldr ?? { changed: p.summary };
    return { beats: [
      { label: "What changed", text: t.changed },
      ...(t.lookFirst ? [{ label: "Look at first", text: t.lookFirst }] : []),
      ...(t.risk ? [{ label: "Risk", text: t.risk }] : []),
    ] };
  },
  "done-when": (p) => {
    const rows = p.delivered.map((d) => ({ cells: [d.criterion, d.result], status: d.status }));
    const counts = DELIVERED_STATUS.map((status) => ({ status, n: rows.filter((r) => r.status === status).length })).filter((c) => c.n);
    // The rows that need attention first, then the met rows, then the pending rows of a
    // checkpoint, each in props order.
    const rank = (r) => ({ missed: 0, partial: 0, met: 1, pending: 2 })[r.status] ?? 0;
    const ordered = [0, 1, 2].flatMap((k) => rows.filter((r) => rank(r) === k));
    return { columns: ["Planned criterion", "Delivered", "Status"], counts, ...limited(ordered, LIMITS.table) };
  },
  changes: (p, shots) => {
    const list = p.changes.map((c, i) => ({ n: i + 1, area: c.area }));
    const withSequence = p.changes.findIndex((c) => c.sequence);
    const picks = p.changes.map((_, i) => i).slice(0, LIMITS.featured);
    if (withSequence >= LIMITS.featured) picks[picks.length - 1] = withSequence;
    const featured = picks.map((i) => {
      const c = p.changes[i];
      const files = c.files ?? [];
      return {
        n: i + 1, area: c.area, how: c.how, why: c.why,
        files: files.slice(0, LIMITS.files), filesMore: Math.max(0, files.length - LIMITS.files),
        sequence: Boolean(c.sequence) && Boolean(shots.sequence),
      };
    });
    // One beat for each featured change; the sequence diagram gets a beat of its own after its change.
    const beats = featured.flatMap((c) => (c.sequence ? [{ type: "card", n: c.n }, { type: "shot", n: c.n }] : [{ type: "card", n: c.n }]));
    return { list: list.slice(0, LIMITS.changes), listMore: Math.max(0, list.length - LIMITS.changes), featured, beats, sequenceShot: shots.sequence ?? null };
  },
  findings: (p) => {
    const all = (p.reviews ?? []).flatMap((r) => r.findings ?? []).map((f) => ({
      severity: f.severity, title: f.title, location: f.location ?? "", status: f.status, id: f.id ?? "",
    }));
    all.sort((a, b) => SEVERITIES.indexOf(a.severity) - SEVERITIES.indexOf(b.severity) || FINDING_STATUS.indexOf(a.status) - FINDING_STATUS.indexOf(b.status));
    const counts = SEVERITIES.map((severity) => ({ severity, n: all.filter((f) => f.severity === severity).length })).filter((c) => c.n);
    return { counts, ...limited(all, LIMITS.findings, "items") };
  },
  "hand-test": (p) => {
    const steps = p.handTest.map((s, i) => ({
      n: i + 1, title: s.title ?? firstSentence(s.why), command: s.command, expect: s.expect ?? "", ref: s.ref,
    }));
    const { lines, shown } = terminalLines(steps);
    return { steps: steps.slice(0, LIMITS.steps), more: Math.max(0, steps.length - LIMITS.steps), lines, terminalSteps: shown };
  },
  rollback: (p) => limited(p.rollback.map((r) => ({ step: r.step, command: r.command ?? "" })), LIMITS.rollback, "steps"),
  outro: (p) => {
    const checkpoint = checkpointOf(p);
    if (checkpoint) {
      return { prefix: clip(`${plain(p.meta.title)} is`, CHARS.statusPrefix), statuses: ["in progress", `${checkpoint.done} of ${checkpoint.total} done`], tone: "ok" };
    }
    const verdict = verdictOf(p);
    return {
      prefix: clip(`${plain(p.meta.title)} is`, CHARS.statusPrefix),
      statuses: ["reviewed", verdict.tone === "ok" ? "ready to merge" : "not ready to merge"],
      tone: verdict.tone,
    };
  },
};

// ---- timing ----

function minimumSeconds(id, data) {
  const seconds = {
    intro: () => 5,
    tldr: () => 2.5 * data.beats.length,
    "done-when": () => 2 + 0.6 * data.rows.length,
    phases: () => 2 + 0.5 * data.groups.reduce((n, g) => n + g.rows.length, 0),
    architecture: () => (data.media ? 9 : 6),
    decisions: () => 4 * data.pages.length,
    "rabbit-holes": () => 2 + 0.6 * data.items.length,
    changes: () => 3.5 * data.beats.length,
    findings: () => 2 + 0.8 * data.items.length,
    "hand-test": () => 2 + 2 * Math.max(1, data.terminalSteps),
    rollback: () => 2 + 1.2 * data.steps.length,
    outro: () => OUTRO_SECONDS,
  }[id]();
  return clamp(seconds, MIN_SCENE_SECONDS, MAX_SCENE_SECONDS);
}

// The data with the frames of its beats, once the scene length is known: each beat starts at an
// equal share of the scene, after a short lead.
function timed(id, data, frames, fps) {
  const lead = Math.round(0.4 * fps);
  const tail = Math.round(0.8 * fps);
  const starts = (n) => Array.from({ length: n }, (_, i) => lead + Math.round(((frames - lead - tail) * i) / Math.max(1, n)));
  if (id === "tldr") return { ...data, starts: starts(data.beats.length) };
  if (id === "decisions") return { ...data, starts: starts(data.pages.length) };
  if (id === "changes") return { ...data, starts: starts(data.beats.length) };
  if (id === "outro") return { ...data, statusHold: Math.max(30, Math.floor((frames - OUTRO_INTRO_FRAMES - tail) / data.statuses.length)) };
  if (id === "architecture") return { ...data, starts: starts(data.media && data.shot ? 2 : 1) };
  if (id === "hand-test") {
    const stepStarts = starts(Math.max(1, data.terminalSteps));
    return { ...data, starts: stepStarts, lines: withDelays(data.lines, stepStarts) };
  }
  return data;
}

// The terminal-simulator types each row after `delay` frames, counted from the end of the row
// before it. Set the delays so that the first row of each step starts at its beat.
function withDelays(lines, stepStarts) {
  let at = TERMINAL.leadFrames;
  return lines.map((line, i) => {
    const first = i === 0 || lines[i - 1].step !== line.step;
    const delay = first ? Math.max(4, stepStarts[line.step] - at) : 4;
    at += delay + Math.ceil(line.text.length / (TERMINAL.charsPerFrame * TERMINAL.chunkSize));
    return { ...line, delay, pause: 0 };
  });
}

// The words of the text, each timed in proportion to its length across `frames` after `lead`.
function captionWords(text, lead, frames) {
  const words = String(text).split(/\s+/).filter(Boolean);
  const weights = words.map((w) => w.length + 1);
  const total = weights.reduce((a, b) => a + b, 0) || 1;
  let at = 0;
  return words.map((w, i) => {
    const startFrame = lead + Math.round((frames * at) / total);
    at += weights[i];
    return { text: w, startFrame, endFrame: Math.max(startFrame + 1, lead + Math.round((frames * at) / total)) };
  });
}

// ---- helpers ----

function titleOf(mode, id) {
  const title = SCENE_TITLES[id];
  return typeof title === "string" ? title : title[mode];
}

// meta.facts, else the date and the plan id.
function factsOf(p) {
  if (p.meta.facts?.length) return p.meta.facts.slice(0, LIMITS.facts);
  return [
    { label: "Date", value: p.meta.date },
    ...(p.meta.milestone !== undefined ? [{ label: "Milestone", value: String(p.meta.milestone) }] : []),
    ...(p.meta.planId ? [{ label: "Plan", value: `\`${p.meta.planId}\`` }] : []),
  ];
}

// The first 2 risks by impact, in props order within one impact.
function topRisks(risks = []) {
  return [...risks].sort((a, b) => IMPACT.indexOf(a.impact) - IMPACT.indexOf(b.impact)).slice(0, 2).map((r) => r.risk);
}

function limited(items, max, key = "rows") {
  return { [key]: items.slice(0, max), more: Math.max(0, items.length - max) };
}

// The rows of a command: "$ " before the first line, each line wrapped at TERMINAL.cols at a space.
function commandRows(command) {
  const rows = [];
  String(command).split("\n").filter((l) => l.trim()).forEach((line, i) => {
    let rest = `${i === 0 ? "$" : ">"} ${line.trim()}`;
    while (rest.length > TERMINAL.cols) {
      const space = rest.lastIndexOf(" ", TERMINAL.cols - 2);
      const cut = space > TERMINAL.cols / 2 ? space : TERMINAL.cols - 2;
      rows.push({ text: `${rest.slice(0, cut)} \\`, type: "command" });
      rest = `    ${rest.slice(cut).trimStart()}`;
    }
    rows.push({ text: rest, type: "command" });
  });
  return rows;
}

function firstSentence(text) {
  const plainText = plain(text);
  const end = plainText.search(/[.!?](\s|$)/);
  return clip(end > 0 ? plainText.slice(0, end) : plainText, CHARS.title);
}

function plain(text) {
  return String(text).replaceAll("`", "");
}

// The longest prefix of at most `budget` characters that ends before a space. One long word
// gets a hard cut, so a cut never drops more than half of the budget.
function cutAt(text, budget) {
  const hard = text.slice(0, budget);
  const space = hard.lastIndexOf(" ");
  return (space >= budget / 2 ? hard.slice(0, space) : hard).trimEnd();
}

function backticks(text) {
  return (text.match(/`/g) ?? []).length;
}

function isOdd(n) {
  return n % 2 === 1;
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}
