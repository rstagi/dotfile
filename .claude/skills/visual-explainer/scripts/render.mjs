#!/usr/bin/env node
// visual-explainer renderer: props JSON -> two self-contained pages. index.html is the report,
// read top to bottom. graph.html is the concept graph to explore. Zero dependencies.
//
//   node render.mjs --props <file> --out <dir> [--video <path>]   validate, write <dir>/index.html and <dir>/graph.html, print both paths
//   node render.mjs --props <file> --check                        validate only
//   node render.mjs --props <file> --ids                          validate, print the node and group ids of the graph
//
// --video adds a video player to both pages. The path is relative to <dir>, ends in .mp4 and has
// no ".." segment (render-video.mjs writes <dir>/video.mp4).
// Media paths in the props are relative to the directory of the props file. The report inlines them.
// --ids prints one "<id>\t<kind>\t<title>" line for each node, then one for each group ("group").
//
// Exit: 0 valid (pages written unless --check or --ids) · 1 invalid props, one
// "<field path>: <error>" line each on stderr · 2 usage or I/O error.
import fs from "node:fs";
import path from "node:path";
import { buildGraph, groupIds } from "./graph.mjs";
import { tourStops } from "./narration.mjs";
import { renderPage } from "./page.mjs";
import { checkProps } from "./props.mjs";
import { renderReport } from "./report.mjs";
import { formatErrors } from "./validate.mjs";

const SKILL_DIR = path.join(import.meta.dirname, "..");
const GRAPH_TEMPLATE = path.join(SKILL_DIR, "template", "page.html");
const REPORT_TEMPLATE = path.join(SKILL_DIR, "template", "report.html");
const USAGE = "usage: render.mjs --props <file> (--out <dir> | --check | --ids) [--video <path>: with --out only]";
// Letters, digits, ".", "_", "-" and "/" only: no scheme, no absolute path, no "%"-encoded dots.
const VIDEO_PATH = /^[A-Za-z0-9_.-]+(?:\/[A-Za-z0-9_.-]+)*\.mp4$/;

main(process.argv.slice(2));

function main(argv) {
  const args = parseArgs(argv);
  if (!args) fail(USAGE);
  if (args.video !== undefined && !isVideoPath(args.video)) {
    fail(`render.mjs: --video must be a relative path that ends in .mp4 and has no ".." segment: ${args.video}`);
  }
  let checked;
  try {
    checked = checkProps(args.props);
  } catch (err) {
    fail(`render.mjs: ${err.message}`);
  }
  if (checked.errors.length) {
    process.stderr.write(`${formatErrors(checked.errors)}\n`);
    process.exit(1);
  }
  if (args.check) return;
  const props = checked.props;
  const graph = buildGraph(props);
  if (args.ids) return printIds(graph);
  const out = path.resolve(args.out);
  const pages = [
    ["index.html", () => renderReport(read(REPORT_TEMPLATE), props, { video: args.video, dir: path.dirname(path.resolve(args.props)) })],
    ["graph.html", () => renderPage(read(GRAPH_TEMPLATE), props, { video: args.video, stops: tourStops(graph, null, props) })],
  ];
  for (const [name, render] of pages) {
    const file = path.join(out, name);
    const html = render();
    try {
      fs.mkdirSync(out, { recursive: true });
      fs.writeFileSync(file, html);
    } catch (err) {
      fail(`render.mjs: cannot write ${file}: ${err.message}`);
    }
    process.stdout.write(`${file}\n`);
  }
}

// Returns null when the arguments do not form exactly one valid call.
function parseArgs(argv) {
  const args = { check: false, ids: false };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag === "--check" || flag === "--ids") args[flag.slice(2)] = true;
    else if (["--props", "--out", "--video"].includes(flag) && argv[i + 1]) args[flag.slice(2)] = argv[++i];
    else return null;
  }
  if (args.video !== undefined && !args.out) return null;
  const modes = [args.check, args.ids, Boolean(args.out)].filter(Boolean).length;
  return args.props && modes === 1 ? args : null;
}

function printIds(graph) {
  const lines = [
    ...graph.nodes.map((n) => [n.id, n.kind, n.title]),
    ...groupIds(graph).map((g) => [g.id, "group", g.label]),
  ].map((cols) => cols.map((c) => String(c).replace(/\s+/g, " ")).join("\t"));
  process.stdout.write(`${lines.join("\n")}\n`);
}

function read(file) {
  try {
    return fs.readFileSync(file, "utf8");
  } catch (err) {
    return fail(`render.mjs: cannot read ${file}: ${err.message}`);
  }
}

function isVideoPath(value) {
  return VIDEO_PATH.test(value) && !value.split("/").includes("..");
}

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(2);
}
