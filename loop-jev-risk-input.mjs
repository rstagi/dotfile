#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import path from "node:path";

const MAX_INPUT_BYTES = 48 * 1024;
const MAX_FIELD_CHARS = 4_000;
const MAX_PATCH_BYTES = 24 * 1024;
const MAX_PATCH_PER_PATH = 2_000;
const SECRET_PATH = /(^|\/)(\.env(?:\.|$)|\.npmrc$|\.pypirc$|credentials?(?:\.|$)|secrets?(?:\.|$)|id_[^/]+$)|\.(?:pem|key|p12|pfx)$/i;
const SECRET_LINE = /(?:api[_-]?key|authorization|bearer|credential|password|secret|token)\s*[:=]/i;
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
  const changedPaths = parseChangedPaths(git(root, ["diff", "--name-status", input.base, input.head]));
  if (changedPaths.length === 0) throw new Error("empty-diff");
  const totals = diffTotals(git(root, ["diff", "--numstat", input.base, input.head]));
  const patchExcerpts = buildPatchExcerpts(root, input.base, input.head, changedPaths);
  const state = {
    phase: input.phase,
    attempt: input.attempt,
    doneWhen: bounded(input.doneWhen),
    changedPaths,
    diff: { files: changedPaths.length, additions: totals.additions, deletions: totals.deletions },
    patchExcerpts,
    verification: { exitCode: 0, summary: bounded(input.verification.summary || "passed") },
  };
  return {
    stage: "merge-risk",
    state,
    questions: {
      scopeGap: {
        type: "choice",
        instructions: "Judge whether the patch may miss the phase Done when. Use only supplied bounded evidence.",
        criteria: { none: "No apparent scope gap", possible: "A scope gap deserves focused inspection", likely: "A likely scope gap needs focused inspection" },
      },
      changeRisk: {
        type: "choice",
        instructions: "Judge change risk to focus, never replace, the mandatory full diff skim.",
        criteria: { low: "Routine localized change", medium: "Meaningful interaction risk", high: "Broad or sensitive change" },
      },
    },
  };
}

function parseChangedPaths(output) {
  return output.split("\n").filter(Boolean).map((line) => {
    const [status, ...names] = line.split("\t");
    const value = names.at(-1);
    if (!status || !value || Buffer.byteLength(value) > 1_000) throw new Error("invalid-changed-path");
    return { status: status[0], path: value };
  });
}

function diffTotals(output) {
  let additions = 0;
  let deletions = 0;
  for (const line of output.split("\n").filter(Boolean)) {
    const [added, deleted] = line.split("\t");
    additions += added === "-" ? 0 : Number.parseInt(added, 10) || 0;
    deletions += deleted === "-" ? 0 : Number.parseInt(deleted, 10) || 0;
  }
  return { additions, deletions };
}

function buildPatchExcerpts(root, base, head, changedPaths) {
  let remaining = MAX_PATCH_BYTES;
  return changedPaths.map(({ path: changedPath }) => {
    if (SECRET_PATH.test(changedPath)) return { path: changedPath, excerpt: "[REDACTED SECRET PATH]" };
    if (remaining <= 0) return { path: changedPath, excerpt: "[OMITTED: PATCH BUDGET EXHAUSTED]" };
    const raw = git(root, ["diff", "--no-color", "--unified=1", base, head, "--", changedPath]);
    const redacted = redact(raw);
    const excerpt = truncateBytes(redacted, Math.min(remaining, MAX_PATCH_PER_PATH));
    remaining -= Buffer.byteLength(excerpt);
    return { path: changedPath, excerpt };
  });
}

function redact(value) {
  return value.split("\n").map((line) => SECRET_LINE.test(line)
    ? `${line.slice(0, 1)}[REDACTED SECRET-LIKE LINE]`
    : line.replaceAll(TOKEN, "[REDACTED TOKEN]"))
    .join("\n");
}

function truncateBytes(value, limit) {
  if (Buffer.byteLength(value) <= limit) return value;
  let output = value.slice(0, limit);
  while (Buffer.byteLength(output) > limit) output = output.slice(0, -1);
  return `${output}\n[TRUNCATED]`;
}

function bounded(value) {
  return String(value).slice(0, MAX_FIELD_CHARS).replaceAll(TOKEN, "[REDACTED TOKEN]");
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
