// Narration: the tour of the graph. Version 2 is an ordered list of stops, each with node or group
// ids to focus and a text. A version 1 narration (one text for each section) still works: each
// section becomes one stop on its cluster of nodes. Without a narration, the page tour visits the
// root node and each group.
//
// A resolved stop: { id, title, text, caption, focus, nodes, overview }. `text` is the spoken text
// ("" for a version 1 section without narration). `caption` is the text of the page tour.
// `nodes` lists the focus nodes in graph order. `overview` is true for "all".
import fs from "node:fs";
import path from "node:path";
import { buildGraph, groupIds, resolveFocus } from "./graph.mjs";
import { validate } from "./validate.mjs";

const SCHEMA_DIR = path.join(import.meta.dirname, "..", "schema");
const MAX_TOTAL_CHARS = 9000;
const TITLE_CHARS = 60;
const ID_HINT = "(render.mjs --ids lists them)";

// The version 1 section ids of each mode, in video order, with the focus of each section.
// A group id with no nodes in the props falls back to the root node.
const SECTIONS = {
  plan: [
    ["intro", "Introduction", ["all"]], ["goal", "Goal", ["goal"]], ["architecture", "Architecture", ["group-architecture"]],
    ["milestones", "Timeline", ["group-timeline"]], ["decisions", "Decisions", ["group-decisions"]], ["risks", "Risks", ["group-risks"]],
    ["outro", "Wrap-up", ["all"]],
  ],
  ship: [
    ["intro", "Introduction", ["all"]], ["summary", "Outcome", ["summary"]], ["changes", "Changes", ["group-changes"]],
    ["delivered", "Delivered", ["group-delivered"]], ["deviations", "Deviations", ["group-deviations"]],
    ["setbacks", "Setbacks and hiccups", ["group-setbacks"]], ["reviews", "Review", ["group-review"]],
    ["handTest", "Hand test", ["group-hand-test"]], ["rollback", "Rollback", ["group-rollback"]], ["outro", "Wrap-up", ["all"]],
  ],
};

// checkNarration(file, props) -> { narration, stops, errors }. `props` must be valid. `errors` is []
// for a valid narration, else [{ path, message }] with paths that start with "narration". `stops`
// is the resolved tour of a valid narration, else null.
// Throws an Error "cannot read <file>: <reason>" when the file cannot be read.
export function checkNarration(file, props, graph = buildGraph(props)) {
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
    return { narration: null, stops: null, errors: [{ path: "narration", message: `invalid JSON (${err.message})` }] };
  }
  const v1 = isV1(narration);
  const schema = JSON.parse(fs.readFileSync(path.join(SCHEMA_DIR, v1 ? "narration-v1.schema.json" : "narration.schema.json"), "utf8"));
  const schemaErrors = validate(schema, narration);
  const errors = schemaErrors.length ? schemaErrors : v1 ? v1Errors(narration, props) : v2Errors(narration, props, graph);
  return {
    narration,
    stops: errors.length ? null : tourStops(graph, narration, props),
    errors: errors.map((e) => ({ ...e, path: e.path.replace(/^\$/, "narration") })),
  };
}

// tourStops(graph, narration, props) -> the resolved stops of a valid narration, or the default
// tour when `narration` is null.
export function tourStops(graph, narration, props) {
  if (!narration) return defaultStops(graph);
  if (isV1(narration)) return v1Stops(graph, narration, props);
  return narration.stops.map((s, i) => stopOf(graph, `stop-${i + 1}`, s.focus, s.text, s.title));
}

// sectionIds(props) -> the version 1 section ids that these props show, in video order.
export function sectionIds(props) {
  if (props.mode === "plan") return SECTIONS.plan.map(([id]) => id);
  return SECTIONS.ship.map(([id]) => id).filter((id) => (id !== "changes" || props.changes?.length) && (id !== "setbacks" || props.process || props.setbacks));
}

// The default tour: the overview, the root node, each group, the overview.
function defaultStops(graph) {
  const root = graph.nodes.find((n) => n.group === null);
  return [
    stopOf(graph, "overview", ["all"], "", graph.title, graph.subtitle),
    stopOf(graph, root.id, [root.id], "", null, root.summary),
    ...graph.groups.map((g) => stopOf(graph, g.id, [g.id], "", g.name, groupCaption(graph, g))),
    stopOf(graph, "explore", ["all"], "", "Explore", "Open any node for all of its facts and connections. Search, filter or follow the links between the nodes."),
  ];
}

function v1Stops(graph, narration, props) {
  const texts = new Map(narration.sections.map((s) => [s.id, s.text]));
  const root = graph.nodes.find((n) => n.group === null);
  const present = new Set(sectionIds(props));
  return SECTIONS[props.mode].filter(([id]) => present.has(id)).map(([id, title, focus]) => {
    const known = focus.filter((f) => f === "all" || graph.nodes.some((n) => n.id === f) || graph.groups.some((g) => g.id === f));
    const group = graph.groups.find((g) => g.id === focus[0]);
    const caption = focus[0] === "all" ? graph.subtitle : group ? groupCaption(graph, group) : root.summary;
    return stopOf(graph, id, known.length ? known : [root.id], texts.get(id) ?? "", title, caption);
  });
}

function stopOf(graph, id, focus, text, title, caption) {
  const { nodes, overview } = resolveFocus(graph, focus);
  return { id, title: title ?? stopTitle(graph, focus), text, caption: text || caption || "", focus, nodes, overview };
}

function stopTitle(graph, focus) {
  const label = (id) => {
    if (id === "all") return graph.title;
    const group = graph.groups.find((g) => g.id === id);
    if (group) return group.name;
    const node = graph.nodes.find((n) => n.id === id);
    return node.title.replaceAll("`", "");
  };
  const first = clip(label(focus[0]), TITLE_CHARS);
  return focus.length === 1 ? first : `${first} and ${focus.length - 1} more`;
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
  return KIND_WORDS[kind][n === 1 ? 0 : 1];
}

function isV1(narration) {
  return narration !== null && typeof narration === "object" && !("version" in narration) && "sections" in narration;
}

function v1Errors(narration, props) {
  if (narration.mode !== props.mode) return [{ path: "$.mode", message: `must be ${props.mode}, the mode of the props` }];
  const ids = SECTIONS[props.mode].map(([id]) => id);
  const present = new Set(sectionIds(props));
  const seen = new Set();
  const errors = [];
  narration.sections.forEach(({ id }, i) => {
    const at = `$.sections[${i}].id`;
    if (!ids.includes(id)) errors.push({ path: at, message: `"${id}" is not a ${props.mode} section` });
    else if (seen.has(id)) errors.push({ path: at, message: `"${id}" appears more than once` });
    else if (!present.has(id)) errors.push({ path: at, message: `the props have no ${id} section` });
    seen.add(id);
  });
  return [...errors, ...lengthErrors(narration.sections, "$.sections")];
}

function v2Errors(narration, props, graph) {
  if (narration.mode !== props.mode) return [{ path: "$.mode", message: `must be ${props.mode}, the mode of the props` }];
  const known = new Set([...graph.nodes.map((n) => n.id), ...groupIds(graph).map((g) => g.id)]);
  const errors = [];
  narration.stops.forEach((s, si) => {
    const seen = new Set();
    s.focus.forEach((id, fi) => {
      const at = `$.stops[${si}].focus[${fi}]`;
      if (!known.has(id)) errors.push({ path: at, message: `"${id}" is not a node id or a group id ${ID_HINT}` });
      else if (seen.has(id)) errors.push({ path: at, message: `"${id}" appears more than once in this stop` });
      seen.add(id);
    });
  });
  return [...errors, ...lengthErrors(narration.stops, "$.stops")];
}

function lengthErrors(items, at) {
  const total = items.reduce((n, s) => n + s.text.length, 0);
  return total > MAX_TOTAL_CHARS ? [{ path: at, message: `has ${total} characters in total, the limit is ${MAX_TOTAL_CHARS}` }] : [];
}

function clip(text, max) {
  return text.length <= max ? text : `${text.slice(0, max - 1).trimEnd()}…`;
}
