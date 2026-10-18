// Storyboard: validated props -> the scene list and the frame timing of the video. Zero dependencies,
// so node --test checks it without the Remotion packages.
//
// Phone rule: each scene shows few items with short text. A long list splits across scenes up to a
// cap. Items past the cap are cut, and the scene says how many more the page has.

export const FPS = 30;
export const WIDTH = 1920;
export const HEIGHT = 1080;

// Minimum visual time of a scene: reading time for its words, within fixed bounds.
const TITLE_SECONDS = 5;
const END_SECONDS = 4;
const MIN_SCENE_SECONDS = 5;
const MAX_SCENE_SECONDS = 10;
const READ_BASE_SECONDS = 2;
const WORDS_PER_SECOND = 2.5;
// A narrated section adds 0.3 s of silence before its audio and 0.3 s after it.
const AUDIO_LEAD_SECONDS = 0.3;
const AUDIO_PAD_SECONDS = 0.6;

const ELLIPSIS = "…";
const IMPACT_ORDER = { high: 0, medium: 1, low: 2 };
const SEVERITIES = ["blocker", "major", "minor", "nit", "style"];
const FINDING_STATUS_ORDER = { open: 0, deferred: 1, fixed: 2 };
// Missed and partial rows first: a cut drops met rows before the others.
const DELIVERED_ORDER = { missed: 0, partial: 1, met: 2 };
// The process numbers in the order and with the labels of the page (PROCESS_STATS in scripts/scenes.mjs).
const PROCESS_LABELS = [
  ["phases", "phases"], ["attempts", "attempts"], ["retries", "retries"], ["escalations", "escalations"],
  ["questions", "questions"], ["hil", "HIL"], ["mergeConflicts", "merge conflicts"], ["reviewRounds", "review rounds"],
  ["fixRounds", "fix rounds"], ["wallTime", "wall time"],
];

// perScene: items in one scene. maxScenes: scenes for the list before the cut.
const LIMITS = {
  layers: { perScene: 2, maxScenes: 2 },
  components: 4,
  flows: { perScene: 4, maxScenes: 1 },
  phases: { perScene: 5, maxScenes: 4 },
  decisions: { perScene: 2, maxScenes: 2 },
  risks: { perScene: 3, maxScenes: 2 },
  strip: 64,
  changes: { perScene: 4, maxScenes: 2 },
  delivered: { perScene: 4, maxScenes: 2 },
  deviations: { perScene: 2, maxScenes: 2 },
  setbacks: { perScene: 2, maxScenes: 2 },
  reviews: { perScene: 1, maxScenes: 2 },
  findings: 3,
  handTest: { perScene: 2, maxScenes: 3 },
  rollback: { perScene: 2, maxScenes: 2 },
  commandLines: 2,
};

// The narration section ids of each mode, in video order.
export const SECTION_IDS = {
  plan: ["intro", "goal", "architecture", "milestones", "decisions", "risks", "outro"],
  ship: ["intro", "summary", "changes", "delivered", "deviations", "setbacks", "reviews", "handTest", "rollback", "outro"],
};

// The narration section of each scene kind. One section can hold several scenes.
const SECTION_OF_KIND = {
  title: "intro", end: "outro",
  goal: "goal", architecture: "architecture", flows: "architecture", timeline: "milestones", decisions: "decisions", risks: "risks",
  summary: "summary", changes: "changes", delivered: "delivered", deviations: "deviations", setbacks: "setbacks",
  review: "reviews", "hand-test": "handTest", rollback: "rollback",
};

// storyboard(props, voiceover, { fps }) -> { fps, width, height, durationInFrames, scenes, sections }.
// `voiceover` maps a section id to its audio clip: { src, seconds }. `fps` defaults to FPS.
// Each scene has a kind, its narration section, its page section, its text (cut for a phone),
// `more` (items that only the page shows), and its timing (`from`, `durationInFrames`).
// Each section has its timing and its audio ({ src, from, durationInFrames } or null).
export function storyboard(props, voiceover = {}, { fps = FPS } = {}) {
  return { fps, width: WIDTH, height: HEIGHT, ...timing(scenesOf(props), voiceover ?? {}, fps) };
}

// sectionIds(props) -> the narration section ids that these props show, in video order.
export function sectionIds(props) {
  return [...new Set(scenesOf(props).map((s) => s.narration))];
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

// paginate(items, perScene, maxScenes) -> { pages, more }. Splits a list into balanced scenes of
// at most `perScene` items, in list order. Items past `maxScenes` scenes are cut: `more` counts them.
export function paginate(items, perScene, maxScenes) {
  const shown = items.slice(0, perScene * maxScenes);
  const count = Math.ceil(shown.length / perScene);
  const pages = [];
  let start = 0;
  for (let i = 0; i < count; i += 1) {
    const size = Math.ceil((shown.length - start) / (count - i));
    pages.push(shown.slice(start, start + size));
    start += size;
  }
  return { pages, more: items.length - shown.length };
}

function scenesOf(props) {
  const scenes = props.mode === "plan" ? planScenes(props) : shipScenes(props);
  return scenes.map((scene) => ({ narration: SECTION_OF_KIND[scene.kind], ...scene }));
}

// ---- plan ----

function planScenes(p) {
  const sections = ["Goal", "Architecture", "Timeline", "Decisions", "Risks"];
  const section = (title) => ({ number: sections.indexOf(title) + 1, title });
  return [
    titleScene(p.meta, "Plan", planStats(p)),
    { kind: "goal", section: section("Goal"), text: clip(p.goal, 280) },
    ...listScenes("architecture", section("Architecture"), p.architecture.layers, LIMITS.layers, (layers) => ({
      summary: clip(p.architecture.summary, 140),
      layers: layers.map(layer),
    })),
    ...listScenes("flows", section("Architecture"), p.architecture.flows ?? [], LIMITS.flows, (flows) => ({
      flows: flows.map((f) => ({ from: clip(f.from, 32), to: clip(f.to, 32), label: clip(f.label, 70) })),
    })),
    ...timelineScenes(p.milestones, section("Timeline")),
    ...listScenes("decisions", section("Decisions"), p.decisions, LIMITS.decisions, (items) => ({
      decisions: items.map(decision),
    })),
    ...listScenes("risks", section("Risks"), sortBy(p.risks, (r) => IMPACT_ORDER[r.impact]), LIMITS.risks, (items) => ({
      risks: items.map((r) => ({ risk: clip(r.risk, 90), impact: r.impact, likelihood: r.likelihood, mitigation: clip(r.mitigation, 100) })),
    })),
    endScene(p.meta),
  ];
}

function planStats(p) {
  const phases = p.milestones.flatMap((m) => m.phases);
  const strip = p.milestones.flatMap((m, i) => [...(i ? ["sep"] : []), ...m.phases.map((ph) => (ph.kind === "pr-review" ? "gate" : "work"))]);
  return {
    milestones: p.milestones.length,
    phases: phases.length,
    lanes: new Set(phases.map((ph) => ph.lane).filter(Boolean)).size,
    strip: strip.slice(0, LIMITS.strip),
  };
}

function layer(l) {
  return {
    name: clip(l.name, 40),
    components: l.components.slice(0, LIMITS.components).map((c) => ({ name: clip(c.name, 32), role: clip(c.role, 70), status: c.status })),
    more: Math.max(0, l.components.length - LIMITS.components),
  };
}

// Phases split by milestone: one scene never mixes two milestones.
function timelineScenes(milestones, section) {
  const pages = milestones.flatMap((m) => paginate(m.phases, LIMITS.phases.perScene, Infinity).pages
    .map((phases) => ({ milestone: { number: m.number, title: clip(m.title, 60) }, phases })));
  const shown = pages.slice(0, LIMITS.phases.maxScenes);
  const more = pages.slice(shown.length).reduce((n, page) => n + page.phases.length, 0);
  return shown.map((page, i) => ({
    kind: "timeline",
    section,
    part: { index: i + 1, count: shown.length },
    milestone: page.milestone,
    phases: page.phases.map((ph) => ({
      number: ph.number, title: clip(ph.title, 64), kind: ph.kind, gate: Boolean(ph.gate),
      lane: ph.lane && clip(ph.lane, 12), estimate: ph.estimate && clip(ph.estimate, 16),
    })),
    more: i === shown.length - 1 ? more : 0,
  }));
}

function decision(d) {
  const [first] = d.rejected;
  return {
    decision: clip(d.decision, 90),
    why: clip(d.why, 140),
    rejected: { option: clip(first.option, 60), reason: clip(first.reason, 90) },
    moreRejected: d.rejected.length - 1,
  };
}

// ---- ship ----

function shipScenes(p) {
  const process = processStats(p.process);
  const setbacks = p.setbacks ?? [];
  // Like the page: the scene shows when either field is present, also when it is empty.
  const hasSetbacks = Boolean(p.process || p.setbacks);
  const sections = ["Outcome", "Delivered", "Deviations", ...(hasSetbacks ? ["Setbacks and hiccups"] : []), "Review", "Hand test", "Rollback"];
  const section = (title) => ({ number: sections.indexOf(title) + 1, title });
  return [
    titleScene(p.meta, "Ship report", shipStats(p)),
    { kind: "summary", section: section("Outcome"), text: clip(p.summary, 280) },
    ...listScenes("changes", section("Outcome"), p.changes ?? [], LIMITS.changes, (items) => ({
      changes: items.map((c) => ({ area: clip(c.area, 40), how: clip(c.how, 100), why: clip(c.why, 80) })),
    })),
    ...listScenes("delivered", section("Delivered"), sortBy(p.delivered, (d) => DELIVERED_ORDER[d.status]), LIMITS.delivered, (items) => ({
      delivered: items.map((d) => ({ phase: d.phase, criterion: clip(d.criterion, 90), result: clip(d.result, 90), status: d.status })),
    })),
    ...nonEmpty(listScenes("deviations", section("Deviations"), p.deviations, LIMITS.deviations, (items) => ({
      deviations: items.map((d) => ({ planned: clip(d.planned, 110), actual: clip(d.actual, 110), reason: clip(d.reason, 120) })),
    })), { kind: "deviations", section: section("Deviations"), deviations: [], more: 0 }),
    ...(hasSetbacks ? nonEmpty(listScenes("setbacks", section("Setbacks and hiccups"), setbacks, LIMITS.setbacks, (items, i) => ({
      process: i === 0 ? process : [],
      setbacks: items.map(setback),
    })), { kind: "setbacks", section: section("Setbacks and hiccups"), process, setbacks: [], more: 0 }) : []),
    ...listScenes("review", section("Review"), p.reviews, LIMITS.reviews, ([r]) => review(r)),
    ...listScenes("hand-test", section("Hand test"), numbered(p.handTest), LIMITS.handTest, (items) => ({
      steps: items.map(({ n, item: s }) => ({ n, title: s.title && clip(s.title, 50), command: commandLines(s.command), ref: clip(s.ref, 60), why: clip(s.why, 110) })),
    })),
    ...listScenes("rollback", section("Rollback"), numbered(p.rollback), LIMITS.rollback, (items) => ({
      steps: items.map(({ n, item: s }) => ({ n, step: clip(s.step, 110), command: s.command && commandLines(s.command) })),
    })),
    endScene(p.meta),
  ];
}

function shipStats(p) {
  const findings = p.reviews.flatMap((r) => r.findings);
  return {
    met: p.delivered.filter((d) => d.status === "met").length,
    total: p.delivered.length,
    verdicts: p.reviews.map((r) => r.verdict),
    unresolved: findings.filter((f) => f.status !== "fixed").length,
  };
}

function processStats(process = {}) {
  return PROCESS_LABELS.filter(([key]) => process[key] !== undefined).map(([key, label]) => [label, clip(process[key], 24)]);
}

function setback(s) {
  return {
    phase: s.phase, kind: s.kind, title: clip(s.title, 60), what: clip(s.what, 90),
    cost: s.cost && clip(s.cost, 32), resolution: clip(s.resolution, 80), lesson: s.lesson && clip(s.lesson, 70),
  };
}

function review(r) {
  const findings = [...r.findings].sort((a, b) => SEVERITIES.indexOf(a.severity) - SEVERITIES.indexOf(b.severity)
    || FINDING_STATUS_ORDER[a.status] - FINDING_STATUS_ORDER[b.status]);
  return {
    verdict: r.verdict,
    repository: r.repository && clip(r.repository, 40),
    head: r.head && r.head.slice(0, 7),
    summary: clip(r.summary, 140),
    severities: SEVERITIES.map((severity) => {
      const of = r.findings.filter((f) => f.severity === severity);
      return { severity, count: of.length, unresolved: of.filter((f) => f.status !== "fixed").length };
    }),
    findings: findings.slice(0, LIMITS.findings).map((f) => ({ id: f.id, severity: f.severity, status: f.status, title: clip(f.title, 80) })),
    moreFindings: Math.max(0, findings.length - LIMITS.findings),
  };
}

// The first lines of a command, each cut for the screen. `more` counts the lines that only the page shows.
function commandLines(command) {
  const lines = command.split("\n").filter((l) => l.trim());
  return { lines: lines.slice(0, LIMITS.commandLines).map((l) => clip(l, 64)), more: Math.max(0, lines.length - LIMITS.commandLines) };
}

// ---- shared scenes ----

function titleScene(meta, eyebrow, stats) {
  const chips = [meta.date, meta.planId && clip(meta.planId, 48), meta.milestone && `Milestone ${meta.milestone}`].filter(Boolean);
  return { kind: "title", eyebrow, title: meta.title, subtitle: clip(meta.subtitle, 150), chips, stats };
}

function endScene(meta) {
  return { kind: "end", title: meta.title, text: "The page has every item, link and code reference." };
}

// One scene for each page of a list. Only the last scene of the list carries `more`.
function listScenes(kind, section, items, limits, build) {
  const { pages, more } = paginate(items, limits.perScene, limits.maxScenes);
  return pages.map((page, i) => ({
    kind,
    section,
    part: { index: i + 1, count: pages.length },
    ...build(page, i),
    more: i === pages.length - 1 ? more : 0,
  }));
}

// A list with no items still gets one scene: `empty` is that scene.
function nonEmpty(scenes, empty) {
  return scenes.length ? scenes : [empty];
}

// Steps keep their page number after a split.
function numbered(items) {
  return items.map((item, i) => ({ n: i + 1, item }));
}

// ---- timing ----

// The narration sets the length. A section lasts the longer of the minimum visual time of its
// scenes and its audio plus 0.6 s. Its scenes share that time in the ratio of their minimum times.
function timing(scenes, voiceover, fps) {
  const timed = [];
  const sections = [];
  let from = 0;
  for (const group of sectionGroups(scenes)) {
    const minimum = group.scenes.map((scene) => Math.round(sceneSeconds(scene) * fps));
    const audio = voiceover[group.id];
    const audioFrames = audio ? Math.ceil(audio.seconds * fps) : 0;
    const total = Math.max(sum(minimum), audio ? audioFrames + Math.round(AUDIO_PAD_SECONDS * fps) : 0);
    const frames = share(minimum, total);
    sections.push({
      id: group.id, from, durationInFrames: total,
      audio: audio ? { src: audio.src, from: from + Math.round(AUDIO_LEAD_SECONDS * fps), durationInFrames: audioFrames } : null,
    });
    group.scenes.forEach((scene, i) => {
      timed.push({ ...scene, from, durationInFrames: frames[i] });
      from += frames[i];
    });
  }
  return { durationInFrames: from, scenes: timed, sections };
}

// Consecutive scenes of one narration section.
function sectionGroups(scenes) {
  const groups = [];
  for (const scene of scenes) {
    if (groups.at(-1)?.id === scene.narration) groups.at(-1).scenes.push(scene);
    else groups.push({ id: scene.narration, scenes: [scene] });
  }
  return groups;
}

// Integer parts of `total` in the ratio of `weights`. The first parts take the remainder.
function share(weights, total) {
  const whole = sum(weights);
  const parts = weights.map((w) => Math.floor((w / whole) * total));
  for (let i = 0, left = total - sum(parts); left > 0; i += 1, left -= 1) parts[i % parts.length] += 1;
  return parts;
}

function sceneSeconds(scene) {
  if (scene.kind === "title") return TITLE_SECONDS;
  if (scene.kind === "end") return END_SECONDS;
  return clamp(READ_BASE_SECONDS + countWords(scene) / WORDS_PER_SECOND, MIN_SCENE_SECONDS, MAX_SCENE_SECONDS);
}

// Words of every text value in the scene. The kind and the section names are not on screen as text to read.
function countWords(value, key = "") {
  if (key === "kind" || key === "section" || key === "narration") return 0;
  if (typeof value === "string") return value.split(/\s+/).filter(Boolean).length;
  if (Array.isArray(value)) return value.reduce((n, v) => n + countWords(v), 0);
  if (value && typeof value === "object") return Object.entries(value).reduce((n, [k, v]) => n + countWords(v, k), 0);
  return 0;
}

// ---- helpers ----

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

function sum(values) {
  return values.reduce((a, b) => a + b, 0);
}

// A stable sort by a numeric key.
function sortBy(items, key) {
  return items.map((item, i) => [item, i]).sort((a, b) => key(a[0]) - key(b[0]) || a[1] - b[1]).map(([item]) => item);
}
