#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import path from "node:path";

const MAX_INPUT_BYTES = 48 * 1024;
const MAX_FIELD_CHARS = 4_000;
const SECRET_LINE = /(?:api[_-]?key|authorization|bearer|credential|password|secret|token)\s*[:=]/i;
const CREDENTIAL_URL = /\b[a-z][a-z0-9+.-]*:\/\/[^\s/'"<>@]+@/i;
const TOKEN = /\b(?:gh[opsu]_|sk-|xox[baprs]-)[A-Za-z0-9_-]{8,}\b/g;

try {
  const input = JSON.parse(await readStdin());
  const output = buildRiskInput(input);
  const serialized = JSON.stringify(output);
  if (Buffer.byteLength(serialized) > MAX_INPUT_BYTES) throw new Error("input-too-large");
  process.stdout.write(`${serialized}\n`);
} catch (error) {
  process.stdout.write(`${JSON.stringify({ error: error?.message || "invalid-input" })}\n`);
  process.exitCode = 2;
}

function buildRiskInput(input) {
  if (!isRecord(input) || !nonEmpty(input.repositoryRoot) || !nonEmpty(input.base)
    || !nonEmpty(input.head) || !nonEmpty(input.phase) || !Number.isInteger(input.attempt)
    || input.attempt < 1 || !nonEmpty(input.doneWhen) || !isRecord(input.verification)) {
    throw new Error("invalid-input");
  }
  if (input.verification.exitCode !== 0) throw new Error("verification-not-passed");
  const root = path.resolve(input.repositoryRoot);
  git(root, ["rev-parse", "--show-toplevel"]);
  const range = `${input.base}...${input.head}`;
  const changedPaths = parseChangedPaths(git(root, ["diff", "--name-status", "-z", range]));
  if (changedPaths.length === 0) throw new Error("empty-diff");
  const totals = diffTotals(git(root, ["diff", "--numstat", "-z", range]));
  const state = {
    phase: input.phase,
    attempt: input.attempt,
    doneWhen: bounded(input.doneWhen),
    changedPaths,
    diff: { files: changedPaths.length, additions: totals.additions, deletions: totals.deletions },
    verification: { exitCode: 0, summary: bounded(input.verification.summary || "passed") },
  };
  return {
    stage: "merge-risk",
    state,
    questions: {
      scopeGap: {
        type: "choice",
        instructions: "Estimate a possible gap against Done when using only changed paths, diff totals, and verification summary. Do not infer file contents.",
        criteria: { none: "No apparent scope gap", possible: "A scope gap deserves focused inspection", likely: "A likely scope gap needs focused inspection" },
      },
      changeRisk: {
        type: "choice",
        instructions: "Estimate change risk from changed paths and diff totals to focus, never replace, the mandatory full diff skim.",
        criteria: { low: "Routine localized change", medium: "Meaningful interaction risk", high: "Broad or sensitive change" },
      },
    },
  };
}

function parseChangedPaths(output) {
  const fields = output.split("\0");
  if (fields.at(-1) === "") fields.pop();
  const paths = [];
  for (let index = 0; index < fields.length;) {
    const status = fields[index++];
    const previousPath = /^[RC]/.test(status) ? fields[index++] : null;
    const changedPath = fields[index++];
    if (!status || !changedPath || Buffer.byteLength(changedPath) > 1_000
      || (previousPath !== null && (!previousPath || Buffer.byteLength(previousPath) > 1_000))) {
      throw new Error("invalid-changed-path");
    }
    paths.push(previousPath === null
      ? { status: status[0], path: changedPath }
      : { status: status[0], path: changedPath, previousPath });
  }
  return paths;
}

function diffTotals(output) {
  let additions = 0;
  let deletions = 0;
  for (const field of output.split("\0")) {
    const match = field.match(/^(-|\d+)\t(-|\d+)\t/);
    if (!match) continue;
    const [, added, deleted] = match;
    additions += added === "-" ? 0 : Number.parseInt(added, 10) || 0;
    deletions += deleted === "-" ? 0 : Number.parseInt(deleted, 10) || 0;
  }
  return { additions, deletions };
}

function bounded(value) {
  return String(value).slice(0, MAX_FIELD_CHARS).split("\n").map((line) => SECRET_LINE.test(line) || CREDENTIAL_URL.test(line)
    ? "[REDACTED SECRET-LIKE TEXT]"
    : line.replaceAll(TOKEN, "[REDACTED TOKEN]"))
    .join("\n");
}

function git(root, args) {
  const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8", maxBuffer: 2 * 1024 * 1024 });
  if (result.status !== 0) throw new Error("git-error");
  return result.stdout;
}

async function readStdin() {
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) {
    input += chunk;
    if (Buffer.byteLength(input) > MAX_INPUT_BYTES) throw new Error("input-too-large");
  }
  return input;
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function nonEmpty(value) {
  return typeof value === "string" && value.length > 0;
}
