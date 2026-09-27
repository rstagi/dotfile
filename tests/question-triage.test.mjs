import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const POLICY = path.join(ROOT, "loop-jev-question.mjs");
const LABELS = ["plan-answer", "code-investigation", "human-preference", "uncertain"];

for (const label of LABELS) {
  test(`records ${label} advice separately from the evidence-backed action`, async () => {
    const result = await runPolicy(input(label));

    assert.equal(result.code, 0);
    assert.deepEqual(result.json, {
      version: 1,
      phase: "4",
      attempt: 2,
      stage: "question",
      mode: "active",
      candidate: label,
      confidence: 0.93,
      probabilities: Object.fromEntries(LABELS.map((value) => [value, value === label ? 0.93 : label === "uncertain" ? 0.07 / 3 : 0.07 / 3])),
      appliedAction: label === "uncertain" ? "current-behavior" : "answer-from-plan",
      fallbackReason: label === "uncertain" ? "uncertain" : null,
      resolvedModel: "jev-test",
      evidenceChecked: true,
      evidenceSources: ["question", "plan"],
      questionRound: 2,
      ts: "2026-09-27T10:00:00Z",
    });
  });
}

test("shadow and decision errors preserve current behavior", async () => {
  const active = input("code-investigation");
  const shadow = await runPolicy({ ...active, action: "current-behavior", jev: { ...active.jev, mode: "shadow" } });
  assert.equal(shadow.json.appliedAction, "current-behavior");
  assert.equal(shadow.json.fallbackReason, "shadow-mode");

  const failed = await runPolicy(input("plan-answer", {
    action: "current-behavior",
    jev: { version: 1, status: "fallback", stage: "question", mode: "active", reason: "api_error" },
  }));
  assert.equal(failed.json.appliedAction, "current-behavior");
  assert.equal(failed.json.fallbackReason, "api_error");
});

test("malformed typed advice cannot become an action or persisted payload", async () => {
  const malformed = input("plan-answer");
  malformed.jev.answers.triage.probabilities = { "plan-answer": "secret-like-string" };
  const result = await runPolicy({ ...malformed, action: "current-behavior" });

  assert.equal(result.code, 0);
  assert.equal(result.json.candidate, null);
  assert.equal(result.json.confidence, null);
  assert.deepEqual(result.json.probabilities, {});
  assert.equal(result.json.appliedAction, "current-behavior");
  assert.equal(result.json.fallbackReason, "invalid-response");
});

test("rejects answers and HIL without inspecting required evidence", async () => {
  const answer = await runPolicy(input("plan-answer", { evidenceSources: ["question"] }));
  assert.equal(answer.code, 2);
  assert.equal(answer.json.error, "missing-evidence:plan");

  const hil = await runPolicy(input("human-preference", { action: "raise-hil", hilReason: "" }));
  assert.equal(hil.code, 2);
  assert.equal(hil.json.error, "hil-requires-substantive-reason");

  const earlyHil = await runPolicy(input("human-preference", { action: "raise-hil", hilReason: "Only the user can choose" }));
  assert.equal(earlyHil.code, 2);
  assert.equal(earlyHil.json.error, "hil-requires-l4");

  const investigated = await runPolicy(input("code-investigation", {
    action: "answer-after-investigation", evidenceSources: ["question", "plan"],
  }));
  assert.equal(investigated.code, 2);
  assert.equal(investigated.json.error, "missing-evidence:code");
});

test("permits exactly three ordinary L2 rounds and bypasses checkpoints", async () => {
  for (const questionRound of [1, 2, 3]) {
    const result = await runPolicy(input("plan-answer", { questionRound }));
    assert.equal(result.code, 0);
    assert.equal(result.json.questionRound, questionRound);
  }
  const exhausted = await runPolicy(input("plan-answer", { questionRound: 4 }));
  assert.deepEqual(exhausted.json, { bypass: true, reason: "l2-round-cap", appliedAction: "escalate-l3" });

  const checkpoint = await runPolicy({ ...input("plan-answer"), checkpoint: true, jev: "not consulted" });
  assert.deepEqual(checkpoint.json, { bypass: true, reason: "checkpoint", appliedAction: "current-checkpoint-path" });
});

function input(label, overrides = {}) {
  const probabilities = Object.fromEntries(LABELS.map((value) => [value, value === label ? 0.93 : 0.07 / 3]));
  return {
    phase: "4", attempt: 2, questionRound: 2, checkpoint: false,
    evidenceSources: ["question", "plan"], action: label === "uncertain" ? "current-behavior" : "answer-from-plan",
    hilReason: null, ts: "2026-09-27T10:00:00Z",
    jev: {
      version: 1, status: "ok", stage: "question", mode: "active", model: "jev-test",
      confidence: 0.93,
      answers: { triage: { type: "choice", choice: label, confidence: 0.93, probabilities } },
    },
    ...overrides,
  };
}

async function runPolicy(payload) {
  const child = spawn(process.execPath, [POLICY], { stdio: ["pipe", "pipe", "pipe"] });
  let stdout = "";
  child.stdout.on("data", (chunk) => { stdout += chunk; });
  child.stdin.end(JSON.stringify(payload));
  const [code] = await new Promise((resolve) => child.on("close", (...args) => resolve(args)));
  return { code, json: JSON.parse(stdout) };
}
