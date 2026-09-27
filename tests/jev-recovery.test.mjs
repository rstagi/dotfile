import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SERVER = path.join(ROOT, "loop-web", "server", "index.mjs");

test("a daemon restart recovers route, question, and risk decisions saved during an outage", async (t) => {
  const temp = await mkdtemp(path.join(os.tmpdir(), "loop-jev-recovery-"));
  const loopDir = path.join(temp, ".loop");
  const storeDir = path.join(temp, "store");
  const runDir = path.join(loopDir, "runs", "1-a1");
  const runId = "loop-jev-recovery-test";
  const port = await freePort();
  const baseUrl = `http://127.0.0.1:${port}`;
  let daemon = null;

  t.after(async () => {
    if (daemon) await stopDaemon(daemon);
    await rm(temp, { recursive: true, force: true });
  });

  await mkdir(runDir, { recursive: true });
  daemon = startDaemon(storeDir, port);
  await waitForHealth(baseUrl, daemon);
  const response = await fetch(`${baseUrl}/api/loops/${runId}/register`, {
    method: "POST",
    body: JSON.stringify({ loopDir, planText: "# Recovery test\n" }),
  });
  assert.equal(response.status, 200);
  await stopDaemon(daemon);
  daemon = null;

  await writeFile(path.join(runDir, "route-decision.json"), JSON.stringify({
    version: 1,
    phase: "1",
    attempt: 1,
    stage: "route",
    mode: "shadow",
    candidate: "light",
    confidence: 0.9,
    probabilities: { default: 0.1, light: 0.9 },
    appliedAction: "default",
    fallbackReason: "shadow-mode",
    resolvedModel: "jev-test",
    ts: "2026-09-27T10:00:00Z",
  }));
  await writeFile(path.join(runDir, "question-decision.json"), JSON.stringify({
    version: 1,
    phase: "1",
    attempt: 1,
    stage: "question",
    mode: "shadow",
    candidate: "plan-answer",
    appliedAction: "current-behavior",
    fallbackReason: "shadow-mode",
    questionRound: 1,
    ts: "2026-09-27T10:00:00.001Z",
  }));
  const head = "abcdef1234567890";
  await writeFile(path.join(runDir, `risk-decision-${head}.json`), JSON.stringify({
    version: 1,
    phase: "1",
    attempt: 1,
    stage: "merge-risk",
    mode: "active",
    head,
    candidate: "scope-gap:possible · risk:medium",
    appliedAction: "full-diff-skim",
    fallbackReason: null,
    requiredGates: ["verified-exit-zero", "full-diff-skim"],
    completedGates: ["verified-exit-zero"],
    remainingGates: ["full-diff-skim"],
    focus: ["src/app.ts"],
    ts: "2026-09-27T10:00:00.002Z",
  }));
  await writeFile(path.join(runDir, "risk-decision-deadbeef.json"), JSON.stringify({
    version: 1, phase: "1", attempt: 1, stage: "merge-risk", mode: "active",
    head: "different-head", ts: "2026-09-27T10:00:03Z",
  }));
  const mismatchedRunDir = path.join(loopDir, "runs", "2-a3");
  await mkdir(mismatchedRunDir);
  await writeFile(path.join(mismatchedRunDir, "route-decision.json"), JSON.stringify({
    version: 1, phase: "2", attempt: 2, stage: "route", mode: "active",
    ts: "2026-09-27T10:00:04Z",
  }));

  daemon = startDaemon(storeDir, port);
  await waitForHealth(baseUrl, daemon);
  const snapshot = await fetch(`${baseUrl}/api/loops/${runId}/snapshot`).then((result) => result.json());
  assert.deepEqual(snapshot.decisions.map(({ stage }) => stage), ["route", "question", "merge-risk"]);
  assert.equal(snapshot.jev.mode, "active");
  assert.equal(snapshot.decisions.find(({ stage }) => stage === "route").candidate, "light");
  const risk = snapshot.decisions.find(({ stage }) => stage === "merge-risk");
  assert.equal(risk.head, head);
  assert.deepEqual(risk.completedGates, ["verified-exit-zero"]);
  assert.deepEqual(risk.focus, ["src/app.ts"]);

  const later = "2026-09-27T10:30:00Z";
  const finish = await fetch(`${baseUrl}/api/loops/${runId}/event`, {
    method: "POST",
    body: JSON.stringify({ event: "phase.attempt.finish", phase: "1", attempt: 1,
      outcome: "done", exitCode: 0, ts: later }),
  });
  assert.equal(finish.status, 200);
  await stopDaemon(daemon);
  daemon = null;
  const storeFile = path.join(storeDir, `${runId}.json`);
  const stored = JSON.parse(await readFile(storeFile, "utf8"));
  stored.events = Array.from({ length: 5000 }, (_, index) => ({
    event: "test.tick", phase: "1", attempt: index + 1,
    ts: new Date(Date.parse("2026-09-27T10:10:00Z") + index * 1000).toISOString(),
  }));
  await writeFile(storeFile, JSON.stringify(stored));
  daemon = startDaemon(storeDir, port);
  await waitForHealth(baseUrl, daemon);
  const afterCap = await fetch(`${baseUrl}/api/loops/${runId}/snapshot`).then((result) => result.json());
  assert.equal(afterCap.events.length, 5000);
  assert.equal(afterCap.events.some(({ event }) => event === "jev.decision"), false);
  const summaries = await fetch(`${baseUrl}/api/loops`).then((result) => result.json());
  assert.equal(summaries.find(({ runId: id }) => id === runId).updatedAt, later);

  await stopDaemon(daemon);
  daemon = null;
  await rm(loopDir, { recursive: true, force: true });
  daemon = startDaemon(storeDir, port);
  await waitForHealth(baseUrl, daemon);
  const archived = await fetch(`${baseUrl}/api/loops/${runId}/snapshot`).then((result) => result.json());
  assert.deepEqual(archived.decisions, snapshot.decisions);
  const replay = spawnSync(process.execPath, [path.join(ROOT, "loop-jev-replay.mjs"), storeFile], {
    encoding: "utf8",
  });
  assert.equal(replay.status, 0, replay.stderr);
  const report = JSON.parse(replay.stdout);
  assert.equal(report.overall.total, 3);
  assert.equal(report.stages["merge-risk"].total, 1);
});

function startDaemon(storeDir, port) {
  const child = spawn(process.execPath, [SERVER, "--daemon", "--port", String(port)], {
    env: { ...process.env, LOOP_STORE_DIR: storeDir },
    stdio: ["ignore", "ignore", "pipe"],
  });
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => { child.errorOutput = (child.errorOutput ?? "") + chunk; });
  return child;
}

async function stopDaemon(child) {
  if (child.exitCode !== null) return;
  child.kill("SIGTERM");
  await once(child, "close");
}

async function waitForHealth(baseUrl, child) {
  for (let attempt = 0; attempt < 50; attempt++) {
    if (child.exitCode !== null) throw new Error(`daemon exited: ${child.errorOutput ?? ""}`);
    try {
      if ((await fetch(`${baseUrl}/api/health`)).ok) return;
    } catch {
      // The server is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`daemon did not start: ${child.errorOutput ?? ""}`);
}

async function freePort() {
  const server = net.createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = server.address().port;
  server.close();
  await once(server, "close");
  return port;
}
