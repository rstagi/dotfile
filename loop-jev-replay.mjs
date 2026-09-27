#!/usr/bin/env node

import { statSync } from "node:fs";
import { readFile } from "node:fs/promises";

const STAGES = ["route", "question", "merge-risk"];
const NOTICE = "Synthetic fixtures validate replay mechanics only; no savings are claimed.";

await replay(process.argv.slice(2));

async function replay(paths) {
  if (paths.length === 0) fail("usage: loop-jev-replay.mjs <snapshot-or-store.json> [...]");

  const sources = await Promise.all(paths.map(loadSource));
  const decisions = sources.flatMap(decisionsFromSource);
  process.stdout.write(`${JSON.stringify(buildReport(paths.length, decisions))}\n`);
}

function buildReport(sourceCount, decisions) {
  const stageEntries = STAGES.map((stage) => [stage, summarize(decisions.filter((decision) => decision.stage === stage))]);
  return {
    version: 1,
    sourceCount,
    stages: Object.fromEntries(stageEntries),
    overall: summarize(decisions),
    fallbackReasons: countFallbackReasons(decisions),
    metrics: summarizeMetrics(decisions),
    notice: NOTICE,
  };
}

function summarize(decisions) {
  const comparable = decisions.filter(isComparable);
  const agreements = comparable.filter(agrees).length;
  const fallbacks = decisions.filter((decision) => nonEmpty(decision.fallbackReason)).length;
  return {
    total: decisions.length,
    comparable: comparable.length,
    agreements,
    agreementRate: comparable.length === 0 ? null : agreements / comparable.length,
    fallbacks,
    fallbackRate: decisions.length === 0 ? null : fallbacks / decisions.length,
  };
}

function summarizeMetrics(decisions) {
  const latencies = numbers(decisions, "latencyMs");
  const retries = numbers(decisions, "runnerRetries");
  const costs = numbers(decisions, "costUsd");
  return {
    latency: latencies.length === 0
      ? "unavailable"
      : { recorded: latencies.length, averageMs: sum(latencies) / latencies.length },
    runnerRetries: retries.length === 0
      ? "unavailable"
      : { recorded: retries.length, total: sum(retries) },
    cost: costs.length === 0
      ? "unavailable"
      : { recorded: costs.length, totalUsd: sum(costs) },
  };
}

function agrees(decision) {
  if (decision.stage === "route") return decision.candidate === decision.appliedAction;
  if (decision.stage === "question") {
    return ({
      "plan-answer": ["answer-from-plan"],
      "code-investigation": ["investigate-code", "answer-after-investigation"],
      "human-preference": ["raise-hil"],
      uncertain: ["current-behavior"],
    }[decision.candidate] ?? []).includes(decision.appliedAction);
  }
  if (decision.stage === "merge-risk") {
    const focused = /scope-gap:(possible|likely)|risk:(medium|high)/.test(decision.candidate);
    return decision.appliedAction === (focused ? "focused-full-diff-skim" : "full-diff-skim");
  }
  return false;
}

function isComparable(decision) {
  return STAGES.includes(decision.stage) && nonEmpty(decision.candidate) && nonEmpty(decision.appliedAction);
}

function countFallbackReasons(decisions) {
  const counts = {};
  for (const { fallbackReason } of decisions) {
    if (nonEmpty(fallbackReason)) counts[fallbackReason] = (counts[fallbackReason] ?? 0) + 1;
  }
  return Object.fromEntries(Object.entries(counts).sort(([left], [right]) => left.localeCompare(right)));
}

function decisionsFromSource(source) {
  if (Array.isArray(source)) return source.filter(isDecision);
  if (!isRecord(source)) return [];
  const decisions = Array.isArray(source.lastSnapshot?.decisions)
    && !isDirectory(source.loopDir)
    ? source.lastSnapshot.decisions
    : source.decisions;
  return Array.isArray(decisions) ? decisions.filter(isDecision) : [];
}

async function loadSource(path) {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch {
    fail(`cannot read replay source: ${path}`);
  }
}

function numbers(decisions, key) {
  return decisions.map((decision) => decision[key]).filter((value) => Number.isFinite(value) && value >= 0);
}

function sum(values) {
  return values.reduce((total, value) => total + value, 0);
}

function isDecision(value) {
  return isRecord(value) && STAGES.includes(value.stage);
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isDirectory(path) {
  if (!nonEmpty(path)) return false;
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}

function nonEmpty(value) {
  return typeof value === "string" && value.length > 0;
}

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(2);
}
