#!/usr/bin/env node
// Loop engineering — the pure half of `loop-plan.sh archive`: compute the archived plan text and
// the archive file text for one shipped milestone. Writes nothing outside --out; the caller
// checks the gate PRs and moves the files into place. Needs Node >= 22.18 (it imports the
// daemon's TypeScript plan parser).
//
//   loop-plan-archive.mjs --plan <plan.md> --milestone <N> --out <dir> [--outcome <text>]
//
// Prints one JSON line on stdout:
//   {"status":"archived","archiveFile":"milestones/mN.md","prs":{slug:url}}  + <out>/plan.md, <out>/archive.md
//   {"status":"already","archiveFile":"milestones/mN.md"}
//   {"status":"refused","reason":"..."}
// Exit 0 for every status above; 1 on a usage or I/O error.
import fs from "node:fs";
import path from "node:path";
import { archiveMilestone } from "./loop-web/src/model/archive-milestone.ts";

const args = parseArgs(process.argv.slice(2));
const result = archiveMilestone(fs.readFileSync(args.plan, "utf8"), args.milestone, { outcome: args.outcome });
if (result.status === "archived") {
  fs.writeFileSync(path.join(args.out, "plan.md"), result.plan);
  fs.writeFileSync(path.join(args.out, "archive.md"), result.archive);
  console.log(JSON.stringify({ status: result.status, archiveFile: result.archiveFile, prs: result.prs }));
} else {
  console.log(JSON.stringify(result));
}

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]];
    if (!["--plan", "--milestone", "--out", "--outcome"].includes(flag) || value === undefined) usage(`bad argument ${flag}`);
    out[flag.slice(2)] = value;
  }
  if (!out.plan || !out.out || !/^[1-9][0-9]*$/.test(out.milestone ?? "")) usage("--plan, --milestone <N> and --out are required");
  return { ...out, milestone: Number(out.milestone) };
}

function usage(message) {
  console.error(`loop-plan-archive: ${message}`);
  process.exit(1);
}
