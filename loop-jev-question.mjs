#!/usr/bin/env node

const MAX_INPUT_BYTES = 32 * 1024;
const LABELS = ["plan-answer", "code-investigation", "human-preference", "uncertain"];
const ACTION_EVIDENCE = {
  "answer-from-plan": ["question", "plan"],
  "investigate-code": ["question", "plan"],
  "answer-after-investigation": ["question", "plan", "code"],
  "raise-hil": ["question", "plan"],
  "current-behavior": ["question", "plan"],
};

try {
  const input = JSON.parse(await readStdin());
  const result = questionDecision(input);
  process.stdout.write(`${JSON.stringify(result)}\n`);
} catch (error) {
  process.stdout.write(`${JSON.stringify({ error: error?.message || "invalid-input" })}\n`);
  process.exitCode = 2;
}

function questionDecision(input) {
  if (!isRecord(input)) throw new Error("invalid-input");
  if (input.checkpoint === true) {
    return { bypass: true, reason: "checkpoint", appliedAction: "current-checkpoint-path" };
  }
  if (!Number.isInteger(input.questionRound) || input.questionRound < 1) throw new Error("invalid-question-round");
  if (input.questionRound > 3) {
    return { bypass: true, reason: "l2-round-cap", appliedAction: "escalate-l3" };
  }

  const jev = input.jev;
  if (!isRecord(jev) || jev.stage !== "question" || !["off", "shadow", "active"].includes(jev.mode)) {
    throw new Error("invalid-jev-result");
  }
  const answer = isRecord(jev.answers?.triage) ? jev.answers.triage : {};
  let candidate = LABELS.includes(answer.choice) ? answer.choice : null;
  let probabilities = isDistribution(answer.probabilities) ? answer.probabilities : {};
  let confidence = isProbability(jev.confidence) ? jev.confidence : null;
  let appliedAction = input.action;
  let fallbackReason = null;

  if (jev.status === "ok" && (candidate === null || confidence === null || !isDistribution(answer.probabilities))) {
    candidate = null;
    probabilities = {};
    confidence = null;
    appliedAction = "current-behavior";
    fallbackReason = "invalid-response";
  } else if (jev.status !== "ok") {
    appliedAction = "current-behavior";
    fallbackReason = nonEmpty(jev.reason) || "decision-error";
  } else if (jev.mode !== "active") {
    appliedAction = "current-behavior";
    fallbackReason = jev.mode === "shadow" ? "shadow-mode" : "disabled";
  } else if (candidate === "uncertain" || candidate === null) {
    appliedAction = "current-behavior";
    fallbackReason = candidate === "uncertain" ? "uncertain" : "invalid-label";
  }

  if (!Object.hasOwn(ACTION_EVIDENCE, appliedAction)) throw new Error("invalid-action");
  const evidenceSources = Array.isArray(input.evidenceSources)
    ? [...new Set(input.evidenceSources.filter((value) => ["question", "plan", "code"].includes(value)))]
    : [];
  for (const source of ACTION_EVIDENCE[appliedAction]) {
    if (!evidenceSources.includes(source)) throw new Error(`missing-evidence:${source}`);
  }
  if (appliedAction === "raise-hil" && !nonEmpty(input.hilReason)) {
    throw new Error("hil-requires-substantive-reason");
  }
  if (appliedAction === "raise-hil" && input.escalationLevel !== 4) {
    throw new Error("hil-requires-l4");
  }
  if (!nonEmpty(input.phase) || !Number.isInteger(input.attempt) || input.attempt < 1) {
    throw new Error("invalid-correlation");
  }

  return {
    version: 1,
    phase: input.phase,
    attempt: input.attempt,
    stage: "question",
    mode: jev.mode,
    candidate,
    confidence,
    probabilities,
    appliedAction,
    fallbackReason,
    resolvedModel: nonEmpty(jev.model),
    evidenceChecked: true,
    evidenceSources,
    questionRound: input.questionRound,
    ts: nonEmpty(input.ts) || new Date().toISOString(),
  };
}

async function readStdin() {
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) {
    input += chunk;
    if (Buffer.byteLength(input, "utf8") > MAX_INPUT_BYTES) throw new Error("input-too-large");
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

function isDistribution(value) {
  if (!isRecord(value) || Object.keys(value).length !== LABELS.length
    || LABELS.some((label) => !isProbability(value[label]))) return false;
  return Math.abs(LABELS.reduce((sum, label) => sum + value[label], 0) - 1) <= 0.01;
}
