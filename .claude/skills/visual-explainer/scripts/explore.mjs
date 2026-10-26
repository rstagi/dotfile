#!/usr/bin/env node
// visual-explainer explorer: a graph JSON -> one self-contained page that the reader explores.
// Claude writes the graph (nodes, edges, groups, steps). The page lays it out with ELK, inlined.
//
//   node explore.mjs --graph <file> --out <dir>   validate, write <dir>/explore.html, print its path
//   node explore.mjs --graph <file> --check       validate only, print the warnings
//   --timeline <file>   add the time bar: each node's status over time (timeline.mjs)
//
// Format: SKILL.md, section "Explore mode". Warnings (dropped items) go to stderr.
// Exit: 0 valid · 1 invalid graph, one line on stderr · 2 usage or I/O error.
import fs from "node:fs";
import path from "node:path";
import { parseExplore } from "./explore-spec.mjs";
import { parseTimeline } from "./timeline.mjs";

const SKILL_DIR = path.join(import.meta.dirname, "..");
const TEMPLATE = path.join(SKILL_DIR, "template", "explore.html");
const ELK = path.join(SKILL_DIR, "vendor", "elk.bundled.js");
const USAGE = "usage: explore.mjs --graph <file> [--timeline <file>] (--out <dir> | --check)";

if (import.meta.url === `file://${process.argv[1]}`) main(process.argv.slice(2));

// renderExplore(graph, { template, elk, timeline }) -> the page HTML. Without a timeline, the
// page has no time bar.
export function renderExplore(graph, { template, elk, timeline = null }) {
  const title = graph.title ?? "Graph";
  const summary = graph.summary ? `<p>${escapeHtml(graph.summary)}</p>` : "";
  const values = {
    TITLE: escapeHtml(title),
    DESCRIPTION: escapeHtml(graph.summary ?? title),
    HEADING: escapeHtml(title),
    SUMMARY: summary,
    OUTLINE: outline(graph),
    DATA: scriptJson(graph),
    TIMELINE: scriptJson(timeline),
    ELK: elk,
  };
  // One pass with a function, so "$&" in the data is never read as a replacement pattern.
  return template.replace(/\{\{(TITLE|DESCRIPTION|HEADING|SUMMARY|OUTLINE|DATA|TIMELINE|ELK)\}\}/g, (_, key) => values[key]);
}

function main(argv) {
  const args = parseArgs(argv);
  if (!args) fail(USAGE);
  const parsed = parseExplore(read(args.graph));
  if (!parsed.ok) {
    process.stderr.write(`${parsed.error}\n`);
    process.exit(1);
  }
  const timeline = args.timeline ? readTimeline(args.timeline, parsed.graph) : { timeline: null, warnings: [] };
  for (const w of [...parsed.warnings, ...timeline.warnings]) process.stderr.write(`warning: ${w}\n`);
  if (args.check) return;
  const html = renderExplore(parsed.graph, { template: read(TEMPLATE), elk: read(ELK), timeline: timeline.timeline });
  const file = path.join(path.resolve(args.out), "explore.html");
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, html);
  } catch (err) {
    fail(`explore.mjs: cannot write ${file}: ${err.message}`);
  }
  process.stdout.write(`${file}\n`);
}

// The page without scripts shows this list: each node with where it leads.
function outline(graph) {
  const label = new Map(graph.nodes.map((n) => [n.id, n.label]));
  const items = graph.nodes.filter((n) => n.type !== "group").map((n) => {
    const out = graph.edges.filter((e) => e.from === n.id)
      .map((e) => `${e.label ? `${escapeHtml(e.label)}: ` : ""}${escapeHtml(label.get(e.to))}`);
    const detail = [n.sublabel, n.details].filter(Boolean).map(escapeHtml).join(" ");
    return `<li><strong>${escapeHtml(n.label)}</strong>${detail ? `: ${detail}` : ""}${out.length ? ` <em>To: ${out.join("; ")}</em>` : ""}</li>`;
  });
  return `<ul>${items.join("")}</ul>`;
}

function escapeHtml(text) {
  return String(text).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

// Returns null when the arguments do not form exactly one valid call.
function parseArgs(argv) {
  const args = { check: false };
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === "--check") args.check = true;
    else if (["--graph", "--out", "--timeline"].includes(argv[i]) && argv[i + 1]) args[argv[i].slice(2)] = argv[++i];
    else return null;
  }
  return args.graph && args.check !== Boolean(args.out) ? args : null;
}

// JSON that is safe inside a <script> element: no "</script>" and no line separators.
function scriptJson(value) {
  return JSON.stringify(value).replace(/</g, "\\u003c").replace(/[\u2028\u2029]/g, (c) => `\\u${c.charCodeAt(0).toString(16)}`);
}

function readTimeline(file, graph) {
  try {
    return parseTimeline(JSON.parse(read(file)), new Set(graph.nodes.map((n) => n.id)));
  } catch (err) {
    process.stderr.write(`explore.mjs: ${file} is not valid JSON (${err.message})\n`);
    return process.exit(1);
  }
}

function read(file) {
  try {
    return fs.readFileSync(file, "utf8");
  } catch (err) {
    return fail(`explore.mjs: cannot read ${file}: ${err.message}`);
  }
}

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(2);
}
