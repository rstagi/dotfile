#!/usr/bin/env node
// Loop engineering — the pure half of `loop-plan.sh archive`: compute the archived plan text and
// the archive file text for one shipped milestone. Writes nothing outside --out; the caller
// checks the gate PRs and moves the files into place. Requires Node >= 22.18
// for the retained TypeScript plan model.
//
//   loop-plan-archive.mjs --plan <plan.md> --milestone <N> --out <dir> [--outcome <text>] [--retro <file>]
//
// --retro names the pending milestones/m<N>.retro.md. Its text goes first in the archive file,
// before `## Raw log`, and its Outcome bullet becomes the stub Outcome unless --outcome is given.
//
// Prints one JSON line on stdout:
//   {"status":"archived","archiveFile":"milestones/mN.md","prs":{slug:url}}  + <out>/plan.md, <out>/archive.md
//   {"status":"attached","archiveFile":"milestones/mN.md"}  + <out>/plan.md, <out>/archive.md
//     (milestone N is archived and --retro is given: the retro goes into the existing archive file)
//   {"status":"already","archiveFile":"milestones/mN.md"}
//   {"status":"refused","reason":"..."}
// Exit 0 for every status above; 1 on a usage or I/O error (also a missing archive file to attach to).
import fs from "node:fs";
import path from "node:path";
import { archiveMilestone, attachRetro } from "./loop-plan-model/archive-milestone.ts";

const args = parseArgs(process.argv.slice(2));
const planText = fs.readFileSync(args.plan, "utf8");
const retro = args.retro === undefined ? undefined : fs.readFileSync(args.retro, "utf8");
const result = archiveMilestone(planText, args.milestone, { outcome: args.outcome, retro });
if (result.status === "archived") {
  writeOut(result.plan, result.archive);
  console.log(JSON.stringify({ status: result.status, archiveFile: result.archiveFile, prs: result.prs }));
} else if (result.status === "already" && retro !== undefined) {
  const archiveFile = result.archiveFile ?? `milestones/m${args.milestone}.md`;
  const archivePath = path.join(path.dirname(args.plan), archiveFile);
  if (!fs.existsSync(archivePath)) usage(`milestone ${args.milestone} is archived, but ${archivePath} is missing: cannot attach the retro`);
  const attached = attachRetro(planText, fs.readFileSync(archivePath, "utf8"), args.milestone, retro, { outcome: args.outcome });
  if (attached.status === "attached") {
    writeOut(attached.plan, attached.archive);
    console.log(JSON.stringify({ status: attached.status, archiveFile: attached.archiveFile }));
  } else {
    console.log(JSON.stringify(attached));
  }
} else {
  console.log(JSON.stringify(result));
}

function writeOut(plan, archive) {
  fs.writeFileSync(path.join(args.out, "plan.md"), plan);
  fs.writeFileSync(path.join(args.out, "archive.md"), archive);
}

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]];
    if (!["--plan", "--milestone", "--out", "--outcome", "--retro"].includes(flag) || value === undefined) usage(`bad argument ${flag}`);
    out[flag.slice(2)] = value;
  }
  if (!out.plan || !out.out || !/^[1-9][0-9]*$/.test(out.milestone ?? "")) usage("--plan, --milestone <N> and --out are required");
  return { ...out, milestone: Number(out.milestone) };
}

function usage(message) {
  console.error(`loop-plan-archive: ${message}`);
  process.exit(1);
}
