// Explore graph spec: the JSON that Claude writes for explore.mjs. Modeled on Beluga's
// `beluga-graph` block (nodes, edges, groups, steps). Zero dependencies, browser-safe.
//
//   parseExplore(text) -> { ok: true, graph, warnings } | { ok: false, error }
//
// Small mistakes are dropped with a warning (a duplicate id, an edge to a missing node, a bad
// group). A graph with nothing left to draw fails.

export const NODE_KINDS = [
  "client", "service", "database", "queue", "storage", "external", "actor", "person", "function",
  "file", "decision", "note", "task", "milestone", "metric", "group",
];
export const EDGE_KINDS = ["flow", "call", "data", "async", "depends", "blocks"];
export const STATUSES = ["todo", "active", "review", "done", "blocked", "failed", "idle"];
export const DIRECTIONS = ["right", "down", "left", "up"];
export const LIMITS = { nodes: 150, edges: 300, steps: 30, links: 8, fields: 40 };

const NODE_ALIASES = {
  db: "database", table: "database", app: "client", ui: "client", frontend: "client", browser: "client",
  server: "service", api: "service", backend: "service", worker: "service", topic: "queue", stream: "queue",
  bucket: "storage", cache: "storage", user: "actor", thirdparty: "external", "third-party": "external",
  fn: "function", method: "function", module: "file", choice: "decision", comment: "note", issue: "task",
  stat: "metric", cluster: "group", boundary: "group",
};
const EDGE_ALIASES = {
  request: "call", http: "call", sync: "call", event: "async", message: "async", publish: "async",
  read: "data", write: "data", dependency: "depends", requires: "depends", blocker: "blocks",
};
const DIRECTION_ALIASES = {
  lr: "right", "left-right": "right", horizontal: "right", td: "down", tb: "down", "top-down": "down", vertical: "down",
};

export function parseExplore(text) {
  let raw;
  try {
    raw = JSON.parse(text);
  } catch (err) {
    return { ok: false, error: `not valid JSON (${err.message})` };
  }
  if (!isObject(raw)) return { ok: false, error: "the graph must be a JSON object" };
  if (!Array.isArray(raw.nodes) || raw.nodes.length === 0) return { ok: false, error: "nodes: needs at least one node" };
  const warnings = [];
  const nodes = readNodes(raw.nodes.slice(0, LIMITS.nodes), warnings);
  if (raw.nodes.length > LIMITS.nodes) warnings.push(`nodes: only the first ${LIMITS.nodes} are drawn`);
  if (nodes.length === 0) return { ok: false, error: "nodes: no node has an id and a label" };
  const ids = new Set(nodes.map((n) => n.id));
  const edges = readEdges(raw.edges, ids, warnings);
  const known = new Set([...ids, ...edges.map((e) => e.id)]);
  const steps = readSteps(raw.steps, known);
  return {
    ok: true,
    warnings,
    graph: {
      title: clip(raw.title, 120),
      summary: clip(raw.summary, 400),
      direction: kindOf(DIRECTIONS, DIRECTION_ALIASES, "right", raw.direction),
      nodes: fixGroups(nodes, warnings),
      edges,
      steps,
    },
  };
}

// An id as written, made safe: other characters become "-" ("Auth Service" -> "Auth-Service").
export function cleanId(raw) {
  return String(raw ?? "").trim().replace(/[^\w.:/-]+/g, "-").slice(0, 64);
}

function readNodes(list, warnings) {
  const seen = new Set();
  const nodes = [];
  for (const [i, n] of list.entries()) {
    const id = isObject(n) ? cleanId(n.id) : "";
    const label = isObject(n) ? clip(n.label, 80) : undefined;
    if (!id || !label) {
      warnings.push(`nodes[${i}]: needs an id and a label, left out`);
    } else if (seen.has(id)) {
      warnings.push(`Two nodes are called "${id}"; the second is left out`);
    } else {
      seen.add(id);
      nodes.push(compact({
        id, label,
        type: kindOf(NODE_KINDS, NODE_ALIASES, "service", n.type),
        sublabel: clip(n.sublabel, 120),
        tag: clip(n.tag, 24),
        status: STATUSES.includes(n.status) ? n.status : undefined,
        group: n.group === undefined ? undefined : cleanId(n.group),
        details: clip(n.details, 4000),
        value: clip(n.value, 24),
        emphasis: n.emphasis === true ? true : undefined,
        fields: readFields(n.fields),
        links: readLinks(n.links),
      }));
    }
  }
  return nodes;
}

function readFields(list) {
  if (!Array.isArray(list)) return undefined;
  const fields = list.slice(0, LIMITS.fields).filter((f) => isObject(f) && clip(f.name, 60)).map((f) => compact({
    name: clip(f.name, 60), type: clip(f.type, 40), key: f.key === "pk" || f.key === "fk" ? f.key : undefined, note: clip(f.note, 120),
  }));
  return fields.length ? fields : undefined;
}

// A link opens only as http(s), a relative path or an absolute path: never a "javascript:" URL.
function readLinks(list) {
  if (!Array.isArray(list)) return undefined;
  const links = list.slice(0, LIMITS.links)
    .filter((l) => isObject(l) && clip(l.label, 60) && clip(l.href, 500) && isSafeHref(l.href))
    .map((l) => ({ label: clip(l.label, 60), href: clip(l.href, 500) }));
  return links.length ? links : undefined;
}

export function isSafeHref(href) {
  const scheme = /^\s*([a-z][a-z0-9+.-]*):/i.exec(String(href));
  return !scheme || ["http", "https"].includes(scheme[1].toLowerCase());
}

function readEdges(list, ids, warnings) {
  if (!Array.isArray(list)) return [];
  const used = new Set();
  const edges = [];
  for (const [i, e] of list.slice(0, LIMITS.edges).entries()) {
    const from = isObject(e) ? cleanId(e.from ?? e.source) : "";
    const to = isObject(e) ? cleanId(e.to ?? e.target) : "";
    if (!ids.has(from) || !ids.has(to)) {
      warnings.push(`The line from "${from}" to "${to}" needs both ends to be nodes, left out`);
      continue;
    }
    let id = cleanId(e.id) || `${from}->${to}`;
    if (used.has(id) || ids.has(id)) id = `${id}#${i}`;
    used.add(id);
    edges.push(compact({
      id, from, to,
      label: clip(e.label, 60),
      kind: kindOf(EDGE_KINDS, EDGE_ALIASES, "flow", e.kind),
      animated: e.animated === true ? true : undefined,
      emphasis: e.emphasis === true ? true : undefined,
    }));
  }
  return edges;
}

// A step may name an id as written ("Auth Service") or as it became ("Auth-Service").
function readSteps(list, known) {
  if (!Array.isArray(list)) return [];
  return list.slice(0, LIMITS.steps).filter((s) => isObject(s) && clip(s.title, 80)).map((s) => {
    const focus = (Array.isArray(s.focus) ? s.focus : []).slice(0, 40)
      .map((f) => (known.has(f) ? f : cleanId(f))).filter((f) => known.has(f));
    return compact({ title: clip(s.title, 80), text: clip(s.text, 600), focus });
  });
}

// A node may sit only in a `group` node, and a group may not sit inside itself.
function fixGroups(nodes, warnings) {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  return nodes.map((n) => {
    if (n.group === undefined) return n;
    const parent = byId.get(n.group);
    if (parent?.type === "group" && n.group !== n.id && !loops(n, byId)) return n;
    warnings.push(`"${n.id}" is in "${n.group}", which is not a group it can be in`);
    const { group: _drop, ...rest } = n;
    return rest;
  });
}

function loops(node, byId) {
  let at = byId.get(node.group);
  for (let hops = 0; at && hops <= byId.size; hops += 1) {
    if (at.id === node.id) return true;
    at = at.group === undefined ? undefined : byId.get(at.group);
  }
  return false;
}

function kindOf(all, aliases, fallback, value) {
  const key = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (all.includes(key)) return key;
  return Object.hasOwn(aliases, key) ? aliases[key] : fallback;
}

function clip(value, max) {
  if (typeof value !== "string" || !value.trim()) return undefined;
  const text = value.trim();
  return text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text;
}

function compact(obj) {
  return Object.fromEntries(Object.entries(obj).filter(([, v]) => v !== undefined));
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
