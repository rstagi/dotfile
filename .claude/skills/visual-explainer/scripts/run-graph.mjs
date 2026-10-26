#!/usr/bin/env node
// visual-explainer run graph: one Loop milestone as it ran -> one explore page with a time bar.
// The plan lanes are the base. Each phase node tells what happened to it. The gate node holds
// the review. The slider replays the phase status over time, and each progress checkpoint is a
// mark with a link to its page. It works for both Loop engines (run-facts.mjs).
//
//   node run-graph.mjs --loop <.loop dir> --milestone <N> [--notes <file>] [--ws-log <file>] --out <dir>
//
// Writes <dir>/run.html, <dir>/run.graph.json and <dir>/run.timeline.json, and prints the
// path of run.html. --notes adds written text: lane what and why, joins, findings and the gate
// (run-model.mjs). --ws-log is the saved `ws log <loop> --type loop.` output of a skills loop.
// Its default is <loop>/explainers/m<N>-ship/loop-log.json.
// Warnings go to stderr. A milestone without timestamps gets the page without a time bar.
//
// Exit: 0 page written · 1 no phases for the milestone or an unreadable plan · 2 usage or I/O error.
import fs from "node:fs";
import path from "node:path";
import { parseExplore } from "./explore-spec.mjs";
import { renderExplore } from "./explore.mjs";
import { readRunFacts, RunFactsError } from "./run-facts.mjs";
import { buildRun } from "./run-model.mjs";
import { parseTimeline } from "./timeline.mjs";

const SKILL_DIR = path.join(import.meta.dirname, "..");
const TEMPLATE = path.join(SKILL_DIR, "template", "explore.html");
const ELK = path.join(SKILL_DIR, "vendor", "elk.bundled.js");
const USAGE = "usage: run-graph.mjs --loop <dir> --milestone <N> [--notes <file>] [--ws-log <file>] --out <dir>";

main(process.argv.slice(2));

function main(argv) {
  const args = parseArgs(argv);
  if (!args) fail(USAGE);
  const out = path.resolve(args.out);
  const notes = args.notes ? readNotes(args.notes, args.milestone) : {};
  let read;
  try {
    read = readRunFacts(path.resolve(args.loop), args.milestone, { wsLog: args["ws-log"] && path.resolve(args["ws-log"]), outDir: out });
  } catch (err) {
    if (!(err instanceof RunFactsError)) throw err;
    process.stderr.write(`run-graph.mjs: ${err.message}\n`);
    process.exit(1);
  }
  const built = buildRun(read.facts, notes);
  const parsed = parseExplore(JSON.stringify(built.graph));
  if (!parsed.ok) fail(`run-graph.mjs: ${parsed.error}`);
  const timeline = parseTimeline(built.timeline, new Set(parsed.graph.nodes.map((n) => n.id)));
  for (const w of [...read.warnings, ...parsed.warnings, ...timeline.warnings]) process.stderr.write(`warning: ${w}\n`);
  const html = renderExplore(parsed.graph, { template: readFile(TEMPLATE), elk: readFile(ELK), timeline: timeline.timeline });
  try {
    fs.mkdirSync(out, { recursive: true });
    fs.writeFileSync(path.join(out, "run.graph.json"), `${JSON.stringify(parsed.graph, null, 2)}\n`);
    fs.writeFileSync(path.join(out, "run.timeline.json"), `${JSON.stringify(timeline.timeline, null, 2)}\n`);
    fs.writeFileSync(path.join(out, "run.html"), html);
  } catch (err) {
    fail(`run-graph.mjs: cannot write to ${out}: ${err.message}`);
  }
  process.stdout.write(`${path.join(out, "run.html")}\n`);
}

// Returns null when the arguments do not form exactly one valid call.
function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (["--loop", "--milestone", "--notes", "--ws-log", "--out"].includes(flag) && argv[i + 1]) args[flag.slice(2)] = argv[++i];
    else return null;
  }
  if (!args.loop || !args.out || !/^[1-9][0-9]*$/.test(args.milestone ?? "")) return null;
  return { ...args, milestone: Number(args.milestone) };
}

// The notes of this milestone. A file may also hold { milestones: { <N>: notes } }.
function readNotes(file, m) {
  try {
    const raw = JSON.parse(readFile(file));
    return { ...raw, ...(raw.milestones?.[m] ?? {}) };
  } catch (err) {
    return fail(`run-graph.mjs: ${file} is not valid JSON (${err.message})`);
  }
}

function readFile(file) {
  try {
    return fs.readFileSync(file, "utf8");
  } catch (err) {
    return fail(`run-graph.mjs: cannot read ${file}: ${err.message}`);
  }
}

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(2);
}
