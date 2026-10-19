#!/usr/bin/env node
// visual-explainer renderer: props JSON -> one self-contained index.html with the concept graph.
// Zero dependencies.
//
//   node render.mjs --props <file> --out <dir> [--video <path>] [--narration <file>]   validate, write <dir>/index.html, print its path
//   node render.mjs --props <file> --check                                            validate only
//   node render.mjs --props <file> --ids                                              validate, print the node and group ids
//
// --video adds a video drawer to the page. The path is relative to <dir>, ends in .mp4 and has no
// ".." segment (render-video.mjs writes <dir>/video.mp4).
// --narration gives the tour captions. Without it, the script uses narration.json in the directory
// of the props file when that file exists. A bad narration.json there gives one stderr warning and
// the default tour. A bad --narration file exits 1.
// --ids prints one "<id>\t<kind>\t<title>" line for each node, then one for each group ("group").
//
// Exit: 0 valid (page written unless --check or --ids) · 1 invalid props or --narration, one
// "<field path>: <error>" line each on stderr · 2 usage or I/O error.
import fs from "node:fs";
import path from "node:path";
import { buildGraph, groupIds } from "./graph.mjs";
import { checkNarration, tourStops } from "./narration.mjs";
import { renderPage } from "./page.mjs";
import { checkProps } from "./props.mjs";
import { formatErrors } from "./validate.mjs";

const SKILL_DIR = path.join(import.meta.dirname, "..");
const TEMPLATE = path.join(SKILL_DIR, "template", "page.html");
const USAGE = "usage: render.mjs --props <file> (--out <dir> | --check | --ids) [--video <path>, --narration <file>: with --out only]";
const NARRATION_FILE = "narration.json";
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
  const graph = buildGraph(checked.props);
  if (args.ids) return printIds(graph);
  const stops = tour(args, checked.props, graph);
  const html = renderPage(fs.readFileSync(TEMPLATE, "utf8"), checked.props, { video: args.video, stops });
  const page = path.join(path.resolve(args.out), "index.html");
  try {
    fs.mkdirSync(path.dirname(page), { recursive: true });
    fs.writeFileSync(page, html);
  } catch (err) {
    fail(`render.mjs: cannot write ${page}: ${err.message}`);
  }
  process.stdout.write(`${page}\n`);
}

// The tour stops: from --narration (exit 1 when invalid), else from narration.json beside the
// props (a warning when invalid), else the default tour.
function tour(args, props, graph) {
  const explicit = args.narration !== undefined;
  const file = explicit ? args.narration : path.join(path.dirname(path.resolve(args.props)), NARRATION_FILE);
  if (!explicit && !fs.existsSync(file)) return tourStops(graph, null, props);
  let checked;
  try {
    checked = checkNarration(file, props, graph);
  } catch (err) {
    fail(`render.mjs: ${err.message}`);
  }
  if (!checked.errors.length) return checked.stops;
  if (explicit) {
    process.stderr.write(`${formatErrors(checked.errors)}\n`);
    process.exit(1);
  }
  process.stderr.write(`render.mjs: the tour ignores ${NARRATION_FILE}: ${formatErrors(checked.errors.slice(0, 1))}\n`);
  return tourStops(graph, null, props);
}

// Returns null when the arguments do not form exactly one valid call.
function parseArgs(argv) {
  const args = { check: false, ids: false };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag === "--check" || flag === "--ids") args[flag.slice(2)] = true;
    else if (["--props", "--out", "--video", "--narration"].includes(flag) && argv[i + 1]) args[flag.slice(2)] = argv[++i];
    else return null;
  }
  if ((args.video !== undefined || args.narration !== undefined) && !args.out) return null;
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

function isVideoPath(value) {
  return VIDEO_PATH.test(value) && !value.split("/").includes("..");
}

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(2);
}
