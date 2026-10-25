#!/usr/bin/env node
// Parse a plan file and print its JSON. Use --plan - to read standard input.
// Requires Node >= 22.18 for TypeScript support.
import fs from "node:fs";
import { parsePlan } from "./loop-plan-model/parse-plan.ts";

const [flag, file, ...extra] = process.argv.slice(2);
if (flag !== "--plan" || !file || extra.length > 0) {
  console.error("loop-plan-parse: use --plan <file|->");
  process.exit(1);
}

try {
  const markdown = fs.readFileSync(file === "-" ? 0 : file, "utf8");
  console.log(JSON.stringify(parsePlan(markdown)));
} catch (error) {
  console.error(`loop-plan-parse: ${error.message}`);
  process.exit(1);
}
