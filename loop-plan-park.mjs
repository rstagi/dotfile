#!/usr/bin/env node
// Loop engineering — the pure half of `loop-milestone.sh park`: print the plan text of a
// milestone parked at its merge gate. Writes nothing. Requires Node >= 22.18 for the retained
// TypeScript plan model.
//
//   loop-plan-park.mjs --plan <plan.md> --milestone <N> --prs '<json slug→url>'
//
// Prints the new plan text on stdout. Exit 3 with the reason on stderr when the plan refuses
// (unknown or archived milestone, a repository without PR); 1 on a usage or I/O error.
import fs from "node:fs";
import { parkMilestone } from "./loop-plan-model/park-milestone.ts";

const args = parseArgs(process.argv.slice(2));
const result = parkMilestone(fs.readFileSync(args.plan, "utf8"), args.milestone, args.prs);
if (result.status === "refused") {
  console.error(`loop-plan-park: ${result.reason}`);
  process.exit(3);
}
process.stdout.write(result.plan);

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]];
    if (!["--plan", "--milestone", "--prs"].includes(flag) || value === undefined) usage(`bad argument ${flag}`);
    out[flag.slice(2)] = value;
  }
  if (!out.plan || !/^[1-9][0-9]*$/.test(out.milestone ?? "") || !out.prs) usage("--plan, --milestone <N> and --prs are required");
  let prs;
  try { prs = JSON.parse(out.prs); } catch { usage("--prs is not JSON"); }
  if (prs === null || typeof prs !== "object" || Array.isArray(prs) || Object.keys(prs).length === 0 || !Object.values(prs).every((url) => typeof url === "string" && url.length > 0)) {
    usage("--prs must be a non-empty JSON object slug→url");
  }
  return { plan: out.plan, milestone: Number(out.milestone), prs };
}

function usage(message) {
  console.error(`loop-plan-park: ${message}`);
  process.exit(1);
}
