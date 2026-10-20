// Narration: the spoken text of the video. Version 3 has one entry for each scene of the video:
// { version: 3, mode, scenes: [{ scene, text }] }. The scenes follow the sections of the report
// page (SCENES). A scene that the narration omits is not in the video.
//
// The graph page (graph.html) does not read a narration. Its tour is the default tour of
// tourStops: the overview, the root node, each group, then a last "explore" stop.
import fs from "node:fs";
import path from "node:path";
import { resolveFocus } from "./graph.mjs";
import { validate } from "./validate.mjs";

const SCHEMA_FILE = path.join(import.meta.dirname, "..", "schema", "narration.schema.json");
const TITLE_CHARS = 60;

// The scene ids of each mode, in video order.
export const SCENES = {
  plan: ["intro", "tldr", "done-when", "phases", "architecture", "decisions", "rabbit-holes", "outro"],
  ship: ["intro", "tldr", "done-when", "changes", "findings", "hand-test", "rollback", "outro"],
};
// Word limits: one scene, and all scenes together. At about 2.5 words a second, a narration
// of 80 to 450 words gives a video of about 45 s to 3 min.
export const WORDS = { scene: [10, 90], total: [80, 450] };

// The props field that each scene shows, for the scenes that can have no data. The test gets
// valid props and returns true when the scene has something to show.
const DATA = {
  plan: {
    "done-when": ["$.doneWhen", (p) => p.doneWhen?.length > 0 || (p.milestones?.[0]?.phases ?? []).some((ph) => ph.doneWhen)],
    phases: ["$.milestones[].phases", (p) => (p.milestones ?? []).some((m) => m.phases?.length)],
    architecture: ["$.architecture.layers", (p) => p.architecture?.layers?.length > 0],
    decisions: ["$.decisions", (p) => p.decisions?.length > 0],
    "rabbit-holes": ["$.rabbitHoles", (p) => p.rabbitHoles?.length > 0],
  },
  ship: {
    "done-when": ["$.delivered", (p) => p.delivered?.length > 0],
    changes: ["$.changes", (p) => p.changes?.length > 0],
    findings: ["$.reviews[].findings", (p) => (p.reviews ?? []).some((r) => r.findings?.length)],
    "hand-test": ["$.handTest", (p) => p.handTest?.length > 0],
    rollback: ["$.rollback", (p) => p.rollback?.length > 0],
  },
};

// checkNarration(file, props) -> { narration, scenes, errors }. `props` must be valid. `errors`
// is [] for a valid narration, else [{ path, message }] with paths that start with "narration".
// `scenes` is [{ id, text }] in video order for a valid narration, else null.
// Throws an Error "cannot read <file>: <reason>" when the file cannot be read.
export function checkNarration(file, props) {
  let text;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch (err) {
    throw new Error(`cannot read ${file}: ${err.message}`);
  }
  let narration;
  try {
    narration = JSON.parse(text);
  } catch (err) {
    return { narration: null, scenes: null, errors: [{ path: "narration", message: `invalid JSON (${err.message})` }] };
  }
  const schemaErrors = validate(JSON.parse(fs.readFileSync(SCHEMA_FILE, "utf8")), narration);
  const errors = schemaErrors.length ? schemaErrors : sceneErrors(narration, props);
  return {
    narration,
    scenes: errors.length ? null : narration.scenes.map((s) => ({ id: s.scene, text: s.text })),
    errors: errors.map((e) => ({ ...e, path: e.path.replace(/^\$/, "narration") })),
  };
}

// hasSceneData(props, id) -> true when the props have data for the scene. intro, tldr and outro
// always have data.
export function hasSceneData(props, id) {
  const rule = DATA[props.mode]?.[id];
  return !rule || Boolean(rule[1](props));
}

// countWords(text) -> the number of words, split on white space.
export function countWords(text) {
  return String(text).split(/\s+/).filter(Boolean).length;
}

// tourStops(graph, narration, props) -> the default tour of the graph page. The page no longer
// reads a narration, so `narration` and `props` do not change the tour; the signature stays for
// the callers. A stop: { id, title, text, caption, focus, nodes, overview }.
export function tourStops(graph, _narration = null, _props = null) {
  const root = graph.nodes.find((n) => n.group === null);
  return [
    stopOf(graph, "overview", ["all"], graph.title, graph.subtitle),
    stopOf(graph, root.id, [root.id], null, root.summary),
    ...graph.groups.map((g) => stopOf(graph, g.id, [g.id], g.name, groupCaption(graph, g))),
    stopOf(graph, "explore", ["all"], "Explore", "Open any node for all of its facts and connections. Search, filter or follow the links between the nodes."),
  ];
}

// The checks after the schema: the mode, the scene ids and their order, the word limits, then the data.
function sceneErrors(narration, props) {
  if (narration.mode !== props.mode) return [{ path: "$.mode", message: `must be ${props.mode}, the mode of the props` }];
  const order = SCENES[props.mode];
  const errors = [];
  const seen = new Set();
  let last = -1;
  narration.scenes.forEach(({ scene }, i) => {
    const at = `$.scenes[${i}].scene`;
    const index = order.indexOf(scene);
    if (index < 0) errors.push({ path: at, message: `"${scene}" is not a ${props.mode} scene (${order.join(", ")})` });
    else if (seen.has(scene)) errors.push({ path: at, message: `"${scene}" appears more than once` });
    else if (index < last) errors.push({ path: at, message: `"${scene}" must come before "${order[last]}" (scene order: ${order.join(", ")})` });
    else if (!hasSceneData(props, scene)) errors.push({ path: at, message: `the props have no data for "${scene}" (${DATA[props.mode][scene][0]})${noDataHint(props, scene)}` });
    seen.add(scene);
    last = Math.max(last, index);
  });
  if (!seen.has("intro")) errors.push({ path: "$.scenes", message: 'must have the "intro" scene' });
  return [...errors, ...wordErrors(narration.scenes)];
}

// A progress checkpoint (meta.checkpoint) comes before the review, so it has no findings yet.
function noDataHint(props, scene) {
  return scene === "findings" && props.meta?.checkpoint ? ": a progress checkpoint has no review yet, omit the scene" : "";
}

function wordErrors(scenes) {
  const errors = [];
  const [min, max] = WORDS.scene;
  let total = 0;
  scenes.forEach(({ text }, i) => {
    const n = countWords(text);
    total += n;
    if (n < min || n > max) errors.push({ path: `$.scenes[${i}].text`, message: `has ${n} words, it must have ${min} to ${max}` });
  });
  const [low, high] = WORDS.total;
  if (total < low || total > high) errors.push({ path: "$.scenes", message: `has ${total} words in total, it must have ${low} to ${high}` });
  return errors;
}

function stopOf(graph, id, focus, title, caption) {
  const { nodes, overview } = resolveFocus(graph, focus);
  return { id, title: title ?? stopTitle(graph, focus), text: "", caption: caption || "", focus, nodes, overview };
}

function stopTitle(graph, focus) {
  const node = graph.nodes.find((n) => n.id === focus[0]);
  return clip(node ? node.title.replaceAll("`", "") : graph.title, TITLE_CHARS);
}

function groupCaption(graph, group) {
  const nodes = graph.nodes.filter((n) => n.group === group.id);
  const counts = group.kinds.map((kind) => {
    const n = nodes.filter((node) => node.kind === kind).length;
    return `${n} ${kindWord(kind, n)}`;
  });
  const titles = nodes.slice(0, 3).map((n) => clip(n.title.replaceAll("`", ""), 70));
  return `${counts.join(", ")}: ${titles.join("; ")}${nodes.length > 3 ? "; and more" : ""}.`;
}

// The singular and the plural word of each kind in a group caption.
const KIND_WORDS = {
  milestone: ["milestone", "milestones"], phase: ["phase", "phases"], layer: ["layer", "layers"], component: ["component", "components"],
  decision: ["decision", "decisions"], risk: ["risk", "risks"], process: ["process summary", "process summaries"],
  change: ["change", "changes"], delivered: ["delivered phase", "delivered phases"], deviation: ["deviation", "deviations"],
  setback: ["setback", "setbacks"], review: ["review", "reviews"], finding: ["finding", "findings"], step: ["step", "steps"],
  rollback: ["rollback step", "rollback steps"],
};

function kindWord(kind, n) {
  return KIND_WORDS[kind]?.[n === 1 ? 0 : 1] ?? kind;
}

function clip(text, max) {
  return text.length <= max ? text : `${text.slice(0, max - 1).trimEnd()}…`;
}
