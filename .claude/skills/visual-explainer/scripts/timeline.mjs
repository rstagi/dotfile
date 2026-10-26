// Timeline spec: the optional time data of an explore page. Zero dependencies, browser-safe.
// The page shows a time bar: a slider, a play button and one button for each mark. At a
// time t, each node in `nodes` shows the status of its last entry at or before t.
//
//   parseTimeline(raw, ids) -> { timeline, warnings }   timeline is null when nothing is usable
//
// Shape (each time is an ISO 8601 string):
//   { start, end,
//     nodes:   { <node id>: [{ at, status }] },           status: one of STATUSES of explore-spec
//     marks:   [{ at, label, text?, href? }],             href: http(s) or a path only
//     stepsAt: [at | null],                               the time that walkthrough step i shows
//     merges:  [at],                                      the label counts merges up to t
//     total?:  <number of phases> }
import { isSafeHref, STATUSES } from "./explore-spec.mjs";

export const TIMELINE_LIMITS = { entries: 200, marks: 40 };

export function parseTimeline(raw, ids) {
  const warnings = [];
  if (!isObject(raw)) return { timeline: null, warnings };
  const start = iso(raw.start);
  let end = iso(raw.end);
  if (!start || !end) {
    warnings.push("timeline: needs a start and an end time, left out");
    return { timeline: null, warnings };
  }
  if (Date.parse(end) <= Date.parse(start)) end = new Date(Date.parse(start) + 60_000).toISOString();
  const nodes = {};
  for (const [id, list] of Object.entries(isObject(raw.nodes) ? raw.nodes : {})) {
    if (!ids.has(id)) {
      warnings.push(`timeline: "${id}" is not a node, left out`);
      continue;
    }
    nodes[id] = (Array.isArray(list) ? list : []).slice(0, TIMELINE_LIMITS.entries)
      .filter((x) => isObject(x) && iso(x.at) && STATUSES.includes(x.status))
      .map((x) => ({ at: iso(x.at), status: x.status }));
  }
  const marks = (Array.isArray(raw.marks) ? raw.marks : []).slice(0, TIMELINE_LIMITS.marks)
    .filter((m) => isObject(m) && iso(m.at) && clip(m.label, 24))
    .map((m) => compact({
      at: iso(m.at),
      label: clip(m.label, 24),
      text: clip(m.text, 300),
      href: typeof m.href === "string" && isSafeHref(m.href) ? clip(m.href, 500) : undefined,
    }));
  return {
    warnings,
    timeline: compact({
      start, end, nodes, marks,
      stepsAt: (Array.isArray(raw.stepsAt) ? raw.stepsAt : []).map((t) => iso(t) ?? null),
      merges: (Array.isArray(raw.merges) ? raw.merges : []).map(iso).filter(Boolean),
      total: Number.isInteger(raw.total) && raw.total >= 0 ? raw.total : undefined,
    }),
  };
}

// The time as a canonical ISO string, or null when it is not a time.
function iso(value) {
  if (typeof value !== "string") return null;
  const t = Date.parse(value);
  return Number.isFinite(t) ? new Date(t).toISOString().replace(/\.000Z$/, "Z") : null;
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
