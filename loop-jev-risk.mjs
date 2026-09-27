#!/usr/bin/env node

const MAX_INPUT_BYTES = 32 * 1024;
const SCOPE_LABELS = ["none", "possible", "likely"];
const RISK_LABELS = ["low", "medium", "high"];
const EVIDENCE = ["done-when", "changed-paths", "diff-stat", "patch-excerpts", "verification", "full-diff"];
const REQUIRED_GATES = [
  "verified-exit-zero", "jev-risk", "full-diff-skim", "stall-check",
  "clean-worktree-check", "reread-steering-notes", "serialized-merge", "post-merge-verify",
];
const COMPLETED_GATE_COUNT = 6;

try {
  const input = JSON.parse(await readStdin());
  process.stdout.write(`${JSON.stringify(riskDecision(input))}\n`);
} catch (error) {
  process.stdout.write(`${JSON.stringify({ error: error?.message || "invalid-input" })}\n`);
  process.exitCode = 2;
}

function riskDecision(input) {
  validateGateEvidence(input);
  const jev = input.jev;
  if (!isRecord(jev) || jev.stage !== "merge-risk" || !["off", "shadow", "active"].includes(jev.mode)) {
    throw new Error("invalid-jev-result");
  }
  const scope = isRecord(jev.answers?.scopeGap) ? jev.answers.scopeGap : {};
  const risk = isRecord(jev.answers?.changeRisk) ? jev.answers.changeRisk : {};
  const validAdvice = jev.status === "ok" && isChoice(scope, SCOPE_LABELS)
    && isChoice(risk, RISK_LABELS) && isProbability(jev.confidence);
  const candidate = validAdvice ? `scope-gap:${scope.choice} · risk:${risk.choice}` : null;
  const probabilities = validAdvice ? {
    ...prefixedDistribution("scope-gap", scope.probabilities),
    ...prefixedDistribution("risk", risk.probabilities),
  } : {};
  let appliedAction = input.action;
  let focus = sanitizeFocus(input.focus);
  let fallbackReason = null;
  if (!validAdvice) fallbackReason = nonEmpty(jev.reason) || "invalid-response";
  else if (jev.mode !== "active") fallbackReason = jev.mode === "shadow" ? "shadow-mode" : "disabled";
  if (fallbackReason !== null) {
    appliedAction = "full-diff-skim";
    focus = [];
  }
  if (!["full-diff-skim", "focused-full-diff-skim"].includes(appliedAction)) throw new Error("invalid-disposition");
  if (appliedAction === "focused-full-diff-skim" && focus.length === 0) throw new Error("missing-focus");

  return {
    version: 1,
    phase: input.phase,
    attempt: input.attempt,
    head: input.headAfter,
    stage: "merge-risk",
    mode: jev.mode,
    candidate,
    confidence: validAdvice ? jev.confidence : null,
    probabilities,
    appliedAction,
    fallbackReason,
    resolvedModel: nonEmpty(jev.model),
    evidenceChecked: true,
    evidenceSources: EVIDENCE,
    requiredGates: REQUIRED_GATES,
    completedGates: REQUIRED_GATES.slice(0, COMPLETED_GATE_COUNT),
    remainingGates: REQUIRED_GATES.slice(COMPLETED_GATE_COUNT),
    focus,
    ts: nonEmpty(input.ts) || new Date().toISOString(),
  };
}

function validateGateEvidence(input) {
  if (!isRecord(input) || !nonEmpty(input.phase) || !Number.isInteger(input.attempt) || input.attempt < 1) {
    throw new Error("invalid-correlation");
  }
  if (input.verificationExitCode !== 0) throw new Error("required-gate:verified-exit-zero");
  if (!nonEmpty(input.headBefore) || !nonEmpty(input.headAfter) || input.headBefore === input.headAfter) {
    throw new Error("required-gate:stall-check");
  }
  if (input.decisionHead !== input.headAfter) throw new Error("attempt-head-mismatch");
  if (input.fullDiffSkimmed !== true) throw new Error("required-gate:full-diff-skim");
  if (input.worktreeClean !== true) throw new Error("required-gate:clean-worktree-check");
  const expected = REQUIRED_GATES.slice(0, COMPLETED_GATE_COUNT);
  if (!Array.isArray(input.completedGates) || input.completedGates.length !== expected.length) {
    throw new Error("required-gate:order");
  }
  expected.forEach((gate, index) => {
    if (input.completedGates[index] !== gate) throw new Error(`required-gate:${gate}`);
  });
  if (!Array.isArray(input.evidenceSources) || EVIDENCE.some((item) => !input.evidenceSources.includes(item))) {
    throw new Error("missing-evidence");
  }
}

function sanitizeFocus(value) {
  if (!Array.isArray(value) || value.length > 10) throw new Error("invalid-focus");
  return [...new Set(value.map((item) => {
    if (!nonEmpty(item) || item.length > 200) throw new Error("invalid-focus");
    return item;
  }))];
}

function isChoice(answer, labels) {
  return answer.type === "choice" && labels.includes(answer.choice)
    && isProbability(answer.confidence) && isDistribution(answer.probabilities, labels);
}

function isDistribution(value, labels) {
  return isRecord(value) && Object.keys(value).length === labels.length
    && labels.every((label) => isProbability(value[label]))
    && Math.abs(labels.reduce((sum, label) => sum + value[label], 0) - 1) <= 0.01;
}

function prefixedDistribution(prefix, value) {
  return Object.fromEntries(Object.entries(value).map(([label, probability]) => [`${prefix}:${label}`, probability]));
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

function nonEmpty(value) {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isProbability(value) {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 1;
}
