import { after, test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const CLI = path.join(import.meta.dirname, "..", "loop-retro.mjs");
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "loop-retro-"));
after(() => fs.rmSync(TMP, { recursive: true, force: true }));

const T0 = Date.parse("2026-10-17T00:00:00Z");
const at = (minutes) => new Date(T0 + minutes * 60_000);

// A fresh .loop directory. Each helper writes one kind of Loop artifact.
function loop(state) {
  const dir = fs.mkdtempSync(path.join(TMP, "loop-"));
  if (state) write(dir, "state.json", JSON.stringify(state));
  return dir;
}

function write(dir, rel, text, mtime) {
  const file = path.join(dir, rel);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text);
  if (mtime) fs.utimesSync(file, mtime, mtime);
}

// attempt(dir, slug, k, { meta, status, spawn, verify, start, end }): one runs/<slug>-a<k>/ directory.
// start and end are minutes after T0: prompt.md gets start, the result files get end.
function attempt(dir, slug, k, { meta, status, spawn, verify, start = 0, end = 10 } = {}) {
  const run = `runs/${slug}-a${k}`;
  write(dir, `${run}/prompt.md`, "prompt", at(start));
  if (meta) write(dir, `${run}/meta.json`, JSON.stringify(meta), at(end));
  if (status) write(dir, `${run}/status.json`, JSON.stringify(status), at(end));
  if (spawn !== undefined) write(dir, `${run}/spawn.log`, spawn, at(end));
  if (verify !== undefined) write(dir, `${run}/verify.log`, verify, at(end));
}

function events(dir, list) {
  write(dir, "events.jsonl", list.map((e) => JSON.stringify(e)).join("\n") + "\n");
}

function retro(dir, milestone, { json = true } = {}) {
  const args = [CLI, "--dir", dir, "--milestone", String(milestone), ...(json ? ["--json"] : [])];
  const res = spawnSync(process.execPath, args, { encoding: "utf8" });
  return { ...res, out: json && res.status === 0 ? JSON.parse(res.stdout) : null };
}

const codex = { engine: "codex", model: "gpt-6.1-sol", engineExit: 0, timedOut: false, routeFallbackReason: "missing_credentials", modelOverride: null, headBefore: "aaa", headAfter: "bbb" };
const claude = { engine: "claude", model: "claude-fable-5-1", engineExit: 0, timedOut: false, routeFallbackReason: null, modelOverride: null, headBefore: "bbb", headAfter: "ccc" };

test("lists each attempt of a milestone phase: engine, model, outcome, verify and minutes", () => {
  const dir = loop({ milestone: 2, phases: { 21: { slug: "p21-headless", status: "done", attempt: 2, kind: "work", milestone: 2 } } });
  attempt(dir, "p21-headless", 1, {
    meta: codex, status: { outcome: "done", checkpoint: false }, start: 0, end: 12,
    spawn: "loop-runner: claimed done but verify failed (see verify.log)\nRUNNER_EXIT=12\n", verify: "FAILED\n",
  });
  attempt(dir, "p21-headless", 2, {
    meta: claude, status: { outcome: "done" }, start: 20, end: 45, spawn: "RUNNER_EXIT=0\n", verify: "ok\n",
  });

  const r = retro(dir, 2);
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.out.phases.map((p) => [p.phase, p.slug, p.kind, p.status]), [[21, "p21-headless", "work", "done"]]);
  assert.deepEqual(r.out.phases[0].attempts.map((a) => ({
    attempt: a.attempt, engine: a.engine, model: a.model, outcome: a.outcome, exit: a.exit,
    verify: a.verify, timedOut: a.timedOut, fallbackReason: a.fallbackReason, minutes: a.minutes,
  })), [
    { attempt: 1, engine: "codex", model: "gpt-6.1-sol", outcome: "done", exit: 12, verify: "fail", timedOut: false, fallbackReason: "missing_credentials", minutes: 12 },
    { attempt: 2, engine: "claude", model: "claude-fable-5-1", outcome: "done", exit: 0, verify: "pass", timedOut: false, fallbackReason: null, minutes: 25 },
  ]);
});

test("without milestones in state, milestone 1 covers every phase; missing files give null values", () => {
  const dir = loop({ phases: { 2: { slug: "p2-api", status: "done", attempt: 1 }, 1: { slug: "p1-doc", status: "done", attempt: 1, kind: "work" } } });
  write(dir, "runs/p2-api-a1/meta.json", "{ not json");
  write(dir, "runs/p1-doc-a1/status.json", JSON.stringify({ outcome: "question", question: "Which port?" }));

  const r = retro(dir, 1);
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.out.phases.map((p) => [p.phase, p.kind]), [[1, "work"], [2, null]]);
  const [doc, api] = r.out.phases.map((p) => p.attempts[0]);
  assert.deepEqual([doc.outcome, doc.question, doc.engine, doc.minutes, doc.verify, doc.exit], ["question", "Which port?", null, null, null, null]);
  assert.deepEqual([api.outcome, api.engine, api.timedOut, api.fallbackReason], [null, null, null, null]);
  assert.deepEqual(retro(dir, 2).out.phases, [], "milestone 2 of a plan without milestones has no phases");
});

test("summarizes each review ledger of a review phase: outer rounds, fix rounds, disputed rounds, final verdict", () => {
  const dir = loop({ milestone: 2, phases: { 38: { slug: "p38-review", status: "done", attempt: 0, kind: "pr-review", milestone: 2 } } });
  const rows = (repo, list) => list.map(([k, stage, role, provider, verdict]) =>
    [k, stage, role, provider, verdict, role === "disputed" ? "runs" : `runs/review-p38-${repo}-${k}`].join("\t")).join("\n") + "\n";
  write(dir, "runs/review-p38-acme--api.ledger.tsv", rows("acme--api", [
    ["a1", "round1", "review", "gpt", "NEEDS_CHANGES"], ["a2", "round1", "critic", "claude", "AGREE"],
    ["a3", "fix1", "fix", "claude", "PUSHED"], ["a4", "round2", "review", "claude", "NEEDS_CHANGES"],
    ["a5", "round2", "critic", "grok", "DISAGREE_EVIDENCE"], ["a6", "round2", "respond", "claude", "NEEDS_CHANGES"],
    ["a7", "round2", "critic", "grok", "DISAGREE_EVIDENCE"], ["a8", "disputed", "disputed", "-", "DISPUTED"],
    ["a9", "final", "final", "claude", "COMMENT"],
  ]));
  write(dir, "runs/review-p38-acme--api.ledger.disputed.md", "# Disputed AR round2\n");
  write(dir, "runs/review-p38-acme--web.ledger.tsv", rows("acme--web", [
    ["a1", "round1", "review", "gpt", "APPROVE"], ["a2", "round1", "critic", "claude", "AGREE"],
    ["a3", "round1", "confirm", "grok", "AGREE"],
  ]));
  write(dir, "runs/review-p380-acme--api.ledger.tsv", rows("acme--api", [["a1", "final", "final", "claude", "APPROVE"]]));

  const r = retro(dir, 2);
  assert.equal(r.status, 0, r.stderr);
  const [review] = r.out.phases;
  assert.deepEqual(review.attempts, []);
  assert.deepEqual(review.reviews, [
    {
      repository: "acme/api", ledger: "runs/review-p38-acme--api.ledger.tsv", runs: 9,
      outerRounds: 2, fixRounds: 1, fixes: [{ stage: "fix1", verdict: "PUSHED", runDir: "runs/review-p38-acme--api-a3" }],
      disputedRounds: 1, disputed: ["round2"], finalVerdict: "COMMENT",
      disputedFile: "runs/review-p38-acme--api.ledger.disputed.md",
    },
    {
      repository: "acme/web", ledger: "runs/review-p38-acme--web.ledger.tsv", runs: 3,
      outerRounds: 1, fixRounds: 0, fixes: [], disputedRounds: 0, disputed: [], finalVerdict: null, disputedFile: null,
    },
  ]);
  assert.deepEqual(r.out.setbackCandidates.map(({ phase, attempt: k, kind, fact, evidence }) => [phase, k, kind, fact, evidence]), [
    [38, null, "review-round", "acme/api fix1: PUSHED", ["runs/review-p38-acme--api.ledger.tsv", "runs/review-p38-acme--api-a3"]],
    [38, null, "review-dispute", "acme/api round2 reached the inner cap without agreement",
      ["runs/review-p38-acme--api.ledger.tsv", "runs/review-p38-acme--api.ledger.disputed.md"]],
  ]);
  assert.deepEqual([r.out.totals.reviewRounds, r.out.totals.fixRounds, r.out.totals.disputedRounds], [3, 1, 1]);
});

test("lists each process cost of a phase as a setback candidate with its evidence paths", () => {
  const dir = loop({
    milestone: 2,
    phases: {
      21: { slug: "p21-headless", status: "done", attempt: 6, kind: "work", milestone: 2 },
      28: { slug: "p28-import", status: "done", attempt: 3, kind: "work", milestone: 2 },
      30: { slug: "p30-mcp", status: "done", attempt: 1, kind: "work", milestone: 2 },
      31: { slug: "p31-runners", status: "blocked", attempt: 1, kind: "work", milestone: 2 },
      5: { slug: "p5-old", status: "done", attempt: 2, kind: "work", milestone: 1 },
    },
  });
  const verifyFail = "loop-runner: claimed done but verify failed (see verify.log)\nRUNNER_EXIT=12\n";
  attempt(dir, "p21-headless", 1, { meta: codex, status: { outcome: "done" }, spawn: verifyFail, verify: "FAILED" });
  attempt(dir, "p21-headless", 2, { meta: codex, status: { outcome: "done" }, spawn: "RUNNER_EXIT=0\n", verify: "ok" });
  attempt(dir, "p21-headless", 3, { meta: claude, status: { outcome: "done" }, spawn: verifyFail, verify: "FAILED" });
  attempt(dir, "p21-headless", 4, { meta: claude, status: { outcome: "question", question: "May I edit F7?", checkpoint: true }, spawn: "RUNNER_EXIT=10\n" });
  write(dir, "runs/p21-headless-a4/answer.md", "Yes.");
  attempt(dir, "p21-headless", 5, { meta: { ...claude, model: "resume" }, status: { outcome: "question", question: "Read passes.txt?" }, spawn: "RUNNER_EXIT=10\n" });
  attempt(dir, "p21-headless", 6, { meta: { ...claude, model: "resume" }, status: { outcome: "done" }, spawn: "RUNNER_EXIT=0\n", verify: "ok" });
  write(dir, "hil/p21-headless.md", "# HIL");
  write(dir, "hil/p21-headless.answer.md", "B");
  attempt(dir, "p28-import", 1, { meta: codex, status: { outcome: "question", question: "Repair the FIFO test?", checkpoint: false }, spawn: "RUNNER_EXIT=10\n" });
  write(dir, "answers/p28-a1-answer.md", "No.");
  attempt(dir, "p28-import", 2, {
    meta: { ...codex, model: "resume", engineExit: 40 },
    spawn: "loop-runner: rate/usage limit on codex:resume — next leg\nloop-runner: model chain exhausted\nRUNNER_EXIT=40\n",
  });
  attempt(dir, "p28-import", 3, {
    meta: { ...claude, timedOut: true, engineExit: 124 }, spawn: "loop-runner: timeout on claude:fable after 1800s\nRUNNER_EXIT=124\n",
  });
  attempt(dir, "p30-mcp", 1, { meta: codex, status: { outcome: "done" }, spawn: "RUNNER_EXIT=0\n", verify: "ok" });
  attempt(dir, "p31-runners", 1, { meta: codex, status: { outcome: "blocked", details: "no base" }, spawn: "RUNNER_EXIT=20\n" });
  attempt(dir, "p5-old", 1, { meta: codex, status: { outcome: "done" }, spawn: verifyFail, verify: "FAILED" });
  events(dir, [
    { ts: "2026-10-17T01:00:00Z", event: "phase.spawn", phase: "21", detail: "attempt a1 chain task pid 1" },
    { ts: "2026-10-17T01:20:00Z", event: "phase.spawn", phase: "21", detail: "attempt a2 chain task pid 2" },
    { ts: "2026-10-17T02:00:00Z", event: "phase.escalate", phase: "21", detail: "L3: merged-tree verify failed" },
    { ts: "2026-10-17T02:00:00Z", event: "phase.spawn", phase: "21", detail: "attempt a3 chain escalate pid 3" },
    { ts: "2026-10-17T02:30:00Z", event: "hil.raise", phase: "21", detail: "verify flaky under load" },
    { ts: "2026-10-17T03:00:00Z", event: "phase.spawn", phase: "21", detail: "attempt a4 chain escalate pid 4" },
    { ts: "2026-10-17T03:30:00Z", event: "phase.resume", phase: "21", detail: "attempt a5 resume sid (claude) pid 5" },
    { ts: "2026-10-17T04:00:00Z", event: "phase.resume", phase: "21", detail: "attempt a6 resume sid (claude) pid 6" },
    { ts: "2026-10-17T04:30:00Z", event: "hil.resolve", phase: "21", detail: "option B" },
    { ts: "2026-10-17T05:00:00Z", event: "phase.spawn", phase: "28", detail: "attempt a1 chain task pid 7" },
    { ts: "2026-10-17T05:10:00Z", event: "phase.resume", phase: "28", detail: "attempt a2 resume sid (codex) pid 8" },
    { ts: "2026-10-17T05:20:00Z", event: "phase.exit40", phase: "28", detail: "a2 codex resume usage limit; backoff then retry (1/3)" },
    { ts: "2026-10-17T05:30:00Z", event: "phase.spawn", phase: "28", detail: "attempt a3 chain task pid 9" },
    { ts: "2026-10-17T06:00:00Z", event: "merge.conflict", phase: "30", repository: "acme/api", detail: "src/a.rs" },
  ]);

  const r = retro(dir, 2);
  assert.equal(r.status, 0, r.stderr);
  const p21 = r.out.phases.find((p) => p.phase === 21);
  assert.deepEqual(p21.attempts.map((a) => a.chain), ["task", "task", "escalate", "escalate", "resume", "resume"]);
  assert.deepEqual(r.out.setbackCandidates.map(({ phase, attempt: k, kind, evidence }) => [phase, k, kind, evidence]), [
    [21, null, "retry", ["runs/p21-headless-a1", "runs/p21-headless-a2", "runs/p21-headless-a3", "runs/p21-headless-a4", "runs/p21-headless-a5", "runs/p21-headless-a6"]],
    [21, null, "hil", ["hil/p21-headless.md", "hil/p21-headless.answer.md", "events.jsonl"]],
    [21, 1, "verify-fail", ["runs/p21-headless-a1/spawn.log", "runs/p21-headless-a1/verify.log"]],
    [21, 3, "verify-fail", ["runs/p21-headless-a3/spawn.log", "runs/p21-headless-a3/verify.log"]],
    [21, 3, "model-escalation", ["events.jsonl", "runs/p21-headless-a3/meta.json"]],
    [21, 4, "question", ["runs/p21-headless-a4/status.json", "runs/p21-headless-a4/answer.md"]],
    [21, 5, "question", ["runs/p21-headless-a5/status.json"]],
    [28, null, "retry", ["runs/p28-import-a1", "runs/p28-import-a2", "runs/p28-import-a3"]],
    [28, 1, "question", ["runs/p28-import-a1/status.json", "answers/p28-a1-answer.md"]],
    [28, 2, "rate-limit", ["runs/p28-import-a2/spawn.log", "runs/p28-import-a2/meta.json", "events.jsonl"]],
    [28, 3, "timeout", ["runs/p28-import-a3/spawn.log", "runs/p28-import-a3/meta.json"]],
    [30, null, "merge-conflict", ["events.jsonl"]],
    [31, 1, "other", ["runs/p31-runners-a1/status.json"]],
  ]);
  for (const candidate of r.out.setbackCandidates) assert.ok(candidate.fact, `candidate ${candidate.kind} states its fact`);
  const fact = (kind) => r.out.setbackCandidates.find((c) => c.kind === kind).fact;
  assert.equal(fact("merge-conflict"), "merge conflict (acme/api): src/a.rs");
  assert.equal(fact("model-escalation"), "a3 moved to the escalate chain (codex gpt-6.1-sol -> claude claude-fable-5-1). phase.escalate: L3: merged-tree verify failed");
  assert.equal(fact("hil"), "HIL raised at 2026-10-17T02:30:00Z: verify flaky under load. Resolved at 2026-10-17T04:30:00Z: option B");
  assert.deepEqual(
    (({ phases, attempts, retries, escalations, questions, hil, mergeConflicts, verifyFailures, timeouts, rateLimits }) =>
      ({ phases, attempts, retries, escalations, questions, hil, mergeConflicts, verifyFailures, timeouts, rateLimits }))(r.out.totals),
    { phases: 4, attempts: 11, retries: 7, escalations: 1, questions: 3, hil: 1, mergeConflicts: 1, verifyFailures: 2, timeouts: 1, rateLimits: 1 },
  );
});

test("counts each merge.verify-fail event as a merged-tree Verify failure of the attempt that the loop merged", () => {
  const dir = loop({
    milestone: 2,
    phases: {
      13: { slug: "p13-lock", status: "done", kind: "work", milestone: 2 },
      21: { slug: "p21-headless", status: "done", kind: "work", milestone: 2 },
      22: { slug: "p22-legacy", status: "done", kind: "work", milestone: 2 },
    },
  });
  const passed = { meta: codex, status: { outcome: "done" }, spawn: "RUNNER_EXIT=0\n", verify: "ok" };
  for (const k of [1, 2]) attempt(dir, "p13-lock", k, passed);
  for (const k of [1, 2, 3]) attempt(dir, "p21-headless", k, passed);
  const lockFail = "verify exit 101\ntest lockfile::fork ... FAILED";
  events(dir, [
    { ts: "2026-10-17T01:00:00Z", event: "phase.spawn", phase: "13", detail: "attempt a1 chain task pid 1" },
    { ts: "2026-10-17T01:30:00Z", event: "merge.verify-fail", phase: "13", repository: "acme/api", detail: lockFail },
    // The L3 spawn has the same second as the failure. File order puts the failure on a1.
    { ts: "2026-10-17T01:30:00Z", event: "phase.spawn", phase: "13", detail: "attempt a2 chain escalate pid 2" },
    { ts: "2026-10-17T02:00:00Z", event: "phase.spawn", phase: "21", detail: "attempt a1 chain task pid 3" },
    { ts: "2026-10-17T02:20:00Z", event: "merge.verify-fail", phase: "21", repository: "acme/api", detail: "notifications regression" },
    { ts: "2026-10-17T02:30:00Z", event: "phase.spawn", phase: "21", detail: "attempt a2 chain task pid 4" },
    { ts: "2026-10-17T02:50:00Z", event: "merge.verify-fail", phase: "21", repository: "acme/api", detail: "notifications regression again" },
    { ts: "2026-10-17T03:00:00Z", event: "phase.spawn", phase: "21", detail: "attempt a3 chain escalate pid 5" },
    { ts: "2026-10-17T04:00:00Z", event: "merge.verify-fail", phase: "22", detail: "verify exit 2" },
  ]);

  const r = retro(dir, 2);
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.out.phases.find((p) => p.phase === 13).mergeVerifyFailures, [
    { ts: "2026-10-17T01:30:00Z", repository: "acme/api", attempt: 1, detail: lockFail },
  ]);
  assert.deepEqual(r.out.setbackCandidates.filter((c) => c.kind === "verify-fail"), [
    { phase: 13, attempt: 1, kind: "verify-fail", stage: "merged-tree", fact: `a1 merged, then the merged-tree Verify failed (acme/api): ${lockFail}`, evidence: ["events.jsonl"] },
    { phase: 21, attempt: 1, kind: "verify-fail", stage: "merged-tree", fact: "a1 merged, then the merged-tree Verify failed (acme/api): notifications regression", evidence: ["events.jsonl"] },
    { phase: 21, attempt: 2, kind: "verify-fail", stage: "merged-tree", fact: "a2 merged, then the merged-tree Verify failed (acme/api): notifications regression again", evidence: ["events.jsonl"] },
    { phase: 22, attempt: null, kind: "verify-fail", stage: "merged-tree", fact: "the merged-tree Verify failed: verify exit 2", evidence: ["events.jsonl"] },
  ]);
  assert.equal(r.out.totals.verifyFailures, 4);
});

test("a runner Verify failure and the first merged-tree failure of the same attempt count once", () => {
  const dir = loop({ milestone: 2, phases: { 21: { slug: "p21-headless", status: "done", kind: "work", milestone: 2 } } });
  const verifyFail = "loop-runner: claimed done but verify failed (see verify.log)\nRUNNER_EXIT=12\n";
  for (const k of [1, 2, 3]) attempt(dir, "p21-headless", k, { meta: claude, status: { outcome: "done" }, spawn: verifyFail, verify: "FAILED" });
  events(dir, [
    { ts: "2026-10-17T01:00:00Z", event: "phase.spawn", phase: "21", detail: "attempt a1 chain task pid 1" },
    { ts: "2026-10-17T01:30:00Z", event: "merge.verify-fail", phase: "21", repository: "acme/api", detail: "notifications regression" },
    { ts: "2026-10-17T02:00:00Z", event: "phase.spawn", phase: "21", detail: "attempt a2 chain task pid 2" },
    { ts: "2026-10-17T02:30:00Z", event: "merge.verify-fail", phase: "21", repository: "acme/api", detail: "first merge" },
    { ts: "2026-10-17T02:40:00Z", event: "merge.verify-fail", phase: "21", repository: "acme/api", detail: "second merge" },
    { ts: "2026-10-17T03:00:00Z", event: "phase.spawn", phase: "21", detail: "attempt a3 chain escalate pid 3" },
  ]);

  const r = retro(dir, 2);
  assert.equal(r.status, 0, r.stderr);
  const runnerLogs = (k) => [`runs/p21-headless-a${k}/spawn.log`, `runs/p21-headless-a${k}/verify.log`];
  assert.deepEqual(r.out.setbackCandidates.filter((c) => c.kind === "verify-fail"), [
    { phase: 21, attempt: 1, kind: "verify-fail", stage: "runner", fact: "a1 claimed done, then Verify failed. The merged-tree Verify failed too (acme/api): notifications regression", evidence: [...runnerLogs(1), "events.jsonl"] },
    { phase: 21, attempt: 2, kind: "verify-fail", stage: "runner", fact: "a2 claimed done, then Verify failed. The merged-tree Verify failed too (acme/api): first merge", evidence: [...runnerLogs(2), "events.jsonl"] },
    { phase: 21, attempt: 2, kind: "verify-fail", stage: "merged-tree", fact: "a2 merged, then the merged-tree Verify failed (acme/api): second merge", evidence: ["events.jsonl"] },
    { phase: 21, attempt: 3, kind: "verify-fail", stage: "runner", fact: "a3 claimed done, then Verify failed", evidence: runnerLogs(3) },
  ]);
  assert.equal(r.out.totals.verifyFailures, 4);
});

test("the milestone window bounds wall time and the plan edits and recycles without a phase", () => {
  const dir = loop({ milestone: 2, phases: { 1: { slug: "p1-a", status: "done", milestone: 1 }, 10: { slug: "p10-b", status: "done", milestone: 2 } } });
  events(dir, [
    { ts: "2026-10-16T21:00:00Z", event: "loop.start", phase: "", detail: "M1 supervise" },
    { ts: "2026-10-16T22:00:00Z", event: "plan.edit", phase: "", detail: "M1: Phase 9 added" },
    { ts: "2026-10-16T23:00:00Z", event: "milestone.awaiting-merge", phase: "", detail: "M1 acme/api=https://github.com/acme/api/pull/1" },
    { ts: "2026-10-17T00:00:00Z", event: "plan.edit", phase: "", detail: "M2: revised at the gate" },
    { ts: "2026-10-17T00:10:00Z", event: "milestone.resumed", phase: "", detail: "M2" },
    { ts: "2026-10-17T05:00:00Z", event: "plan.edit", phase: "", detail: "M2: Phase 12 added" },
    { ts: "2026-10-17T06:00:00Z", event: "sub.recycle", phase: "", detail: "instance 4 recycle at 151515 tokens" },
    { ts: "2026-10-17T20:31:00Z", event: "milestone.awaiting-merge", phase: "", detail: "M2 acme/api=https://github.com/acme/api/pull/2" },
    { ts: "2026-10-17T22:00:00Z", event: "plan.edit", phase: "", detail: "M3: Phase 71 added" },
  ]);

  const m2 = retro(dir, 2).out;
  assert.deepEqual(m2.window, { start: "2026-10-17T00:10:00Z", end: "2026-10-17T20:31:00Z" });
  assert.deepEqual([m2.totals.wallTime, m2.totals.wallMinutes, m2.totals.replans, m2.totals.subRecycles], ["20h 21m", 1221, 1, 1]);
  assert.deepEqual(m2.setbackCandidates, [
    { phase: null, attempt: null, kind: "replan", fact: "plan.edit at 2026-10-17T05:00:00Z: M2: Phase 12 added", evidence: ["events.jsonl"] },
  ]);
  const m1 = retro(dir, 1).out;
  assert.deepEqual(m1.window, { start: "2026-10-16T21:00:00Z", end: "2026-10-16T23:00:00Z" });
  assert.deepEqual([m1.totals.wallTime, m1.totals.replans, m1.totals.subRecycles], ["2h 0m", 1, 0]);
  assert.deepEqual(retro(loop({ phases: {} }), 1).out.window, { start: null, end: null });
  assert.equal(retro(loop({ phases: {} }), 1).out.totals.wallTime, null);
});

test("copies the progress.md entries of each phase and of the milestone, continuation lines included", () => {
  const dir = loop({ milestone: 2, phases: { 2: { slug: "p2-a", milestone: 1 }, 21: { slug: "p21-b", milestone: 2 } } });
  write(dir, "progress.md", [
    "- 2026-10-16 — Phase 2 done (lane A, codex): scaffold",
    "- 2026-10-17 — Milestone 2 resumed on `feat/x`.",
    "- 2026-10-17 — Phase 21 blocked (HIL): Verify flakes",
    "  under load",
    "",
    "- 2026-10-17T09:00:00+00:00 — Phase 21 done (lane A, claude): tests event-bound",
    "- 2026-10-17 — Phase 210 done: not phase 21",
    "- 2026-10-17 — M2 review: COMMENT",
  ].join("\n") + "\n");

  const out = retro(dir, 2).out;
  assert.deepEqual(out.phases[0].progress, [
    "- 2026-10-17 — Phase 21 blocked (HIL): Verify flakes",
    "  under load",
    "- 2026-10-17T09:00:00+00:00 — Phase 21 done (lane A, claude): tests event-bound",
  ]);
  assert.deepEqual(out.progress, ["- 2026-10-17 — Milestone 2 resumed on `feat/x`.", "- 2026-10-17 — M2 review: COMMENT"]);
});

test("without --json, prints the window, the totals, one line per phase and the candidates", () => {
  const dir = loop({ milestone: 1, phases: { 3: { slug: "p3-spike", status: "done", kind: "work", milestone: 1 }, 4: { slug: "p4-doc", status: "done", kind: "work", milestone: 1 } } });
  attempt(dir, "p3-spike", 1, { meta: codex, status: { outcome: "question", question: "May I edit lib.rs?" }, start: 0, end: 5 });
  attempt(dir, "p3-spike", 2, { meta: { ...codex, model: "resume" }, status: { outcome: "done" }, spawn: "RUNNER_EXIT=0\n", verify: "ok", start: 6, end: 20 });
  attempt(dir, "p4-doc", 1, { meta: codex, status: { outcome: "done" }, spawn: "RUNNER_EXIT=0\n", verify: "ok", start: 0, end: 9.5 });
  events(dir, [
    { ts: "2026-10-16T21:00:00Z", event: "loop.start", phase: "" },
    { ts: "2026-10-16T21:23:00Z", event: "phase.spawn", phase: "3", detail: "attempt a1 chain task pid 1" },
    { ts: "2026-10-16T21:45:00Z", event: "milestone.awaiting-merge", phase: "", detail: "M1 acme/api=https://github.com/acme/api/pull/1" },
  ]);

  const r = retro(dir, 1, { json: false });
  assert.equal(r.status, 0, r.stderr);
  const lines = r.stdout.split("\n");
  assert.equal(lines[0], "Milestone 1 · window 2026-10-16T21:00:00Z -> 2026-10-16T21:45:00Z (45m)");
  assert.match(lines[1], /^Totals: phases 2 · attempts 3 · retries 1 · escalations 0 · questions 1 · HIL 0 · merge conflicts 0 · review rounds 0 · fix rounds 0 · disputed rounds 0 · verify failures 0 · timeouts 0 · rate limits 0 · replans 0 · recycles 0 · attempt minutes 29 · wall time 45m$/);
  assert.ok(lines.includes("P3 p3-spike · done · 2 attempts · 19 min"), r.stdout);
  assert.ok(lines.includes("  a1 task codex gpt-6.1-sol · question · verify unknown · 5 min"), r.stdout);
  assert.ok(lines.includes("  a2 - codex resume · done · exit 0 · verify pass · 14 min"), r.stdout);
  assert.ok(lines.includes("P4 p4-doc · done · 1 attempt · 9.5 min · codex gpt-6.1-sol · exit 0 · verify pass"), r.stdout);
  assert.ok(lines.includes("Setback candidates (2):"), r.stdout);
  assert.ok(lines.includes("  P3 retry: 2 attempts (runs/p3-spike-a1, runs/p3-spike-a2)"), r.stdout);
  assert.ok(lines.includes("  P3 a1 question: a1 asked a question (runs/p3-spike-a1/status.json)"), r.stdout);
});

test("usage errors exit 1 and name the problem", () => {
  for (const args of [[], ["--dir", TMP], ["--dir", TMP, "--milestone", "0"], ["--dir", path.join(TMP, "missing"), "--milestone", "1"], ["--bogus"]]) {
    const res = spawnSync(process.execPath, [CLI, ...args], { encoding: "utf8" });
    assert.equal(res.status, 1, `args ${args.join(" ")}`);
    assert.match(res.stderr, /^loop-retro: .+\nusage: loop-retro\.mjs --dir <\.loop> --milestone <N> \[--json\]/);
  }
});

test("tolerates a .loop directory with no state.json", () => {
  const r = retro(loop(), 1);
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.out.phases, []);
});
