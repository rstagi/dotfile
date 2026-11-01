import { afterEach, expect, it } from "vitest";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { readLoop } from "./index.ts";

const FIXTURE = fileURLToPath(new URL("../../../../loop-web/fixtures/.loop", import.meta.url));
const NOW = "2026-08-03T10:10:00.000Z";
let dir = "";

afterEach(() => { if (dir) rmSync(dir, { recursive: true, force: true }); });

it("emits computed drift, checkpoint markers, proof media and journal phase events", async () => {
  dir = mkdtempSync(join(tmpdir(), "cockpit-observations-"));
  cpSync(FIXTURE, dir, { recursive: true });
  const plan = readFileSync(join(dir, "plan.md"), "utf8");
  writeFileSync(join(dir, "plan.md"), plan.replace("- **Touches:** src/checkout", "- **Estimate:** 2–5 min\n- **Touches:** src/checkout"));
  const attempt = join(dir, "runs", "wire-checkout-a1");
  const start = new Date("2026-08-03T09:59:00Z");
  utimesSync(join(attempt, "prompt.md"), start, start);
  writeFileSync(join(attempt, "verify.log"), "2 tests passed\n");
  writeFileSync(join(attempt, "verified.json"), "{}");
  // A proof file needs no prompt file to appear in the cockpit.
  const proof = join(dir, "runs", "payments-gateway-a1", "proof");
  mkdirSync(proof);
  writeFileSync(join(proof, "01-contract-tests-pass.gif"), "test media");
  writeFileSync(join(proof, "tests.tape"), "test command");
  writeFileSync(join(dir, "checkpoints.jsonl"), [
    '{"n":1,"ts":"2026-08-03T10:00:00Z","done":1,"total":4}',
    "{broken", "null", '{"n":2,"ts":"2026-08-03T10:05:00Z","done":2,"total":4}', "",
  ].join("\n"));
  writeFileSync(join(dir, "emitted.jsonl"), [
    JSON.stringify({ ts: "2026-08-03T10:01:00Z", body: { event: "phase.verify-fail", phase: "3" } }),
    JSON.stringify({ ts: "2026-08-03T10:02:00Z", body: { event: "phase.attempt.start", phase: "2" } }),
    JSON.stringify({ ts: "2026-08-03T10:06:00Z", body: { event: "phase.merged", phase: "2" } }),
    "{partial",
  ].join("\n"));

  const events = await readLoop(dir, { now: NOW });
  expect(events.find((event) => event.type === "drift" && event.phaseId === "3"))
    .toMatchObject({ drift: {
      phaseId: "3", estimateMinutes: 5, elapsedMinutes: 11,
      summary: expect.stringContaining("over-time: 11 min (plan 2–5)"),
      verification: "passed: 2 tests passed",
    } });
  expect(events.find((event) => event.type === "drift" && event.phaseId === "3"))
    .toMatchObject({ drift: { summary: expect.stringContaining("verify-fail: 1 failed Verify") } });
  expect(events.filter((event) => event.type === "checkpoint")).toMatchObject([
    { timestamp: "2026-08-03T10:00:00.000Z", checkpoint: { id: "checkpoint-1", summary: "1/4 phases done" } },
    { timestamp: "2026-08-03T10:05:00.000Z", checkpoint: { id: "checkpoint-2", summary: "2/4 phases done" } },
  ]);
  expect(events.filter((event) => event.type === "media")).toMatchObject([
    { phaseId: "1", media: { path: join(proof, "01-contract-tests-pass.gif"), mimeType: "image/gif", phaseId: "1" } },
  ]);
  expect(events.find((event) => event.type === "phase.start" && event.timestamp === "2026-08-03T10:02:00.000Z"))
    .toMatchObject({ phaseId: "2", laneId: "B" });
  expect(events.find((event) => event.type === "phase.end" && event.phaseId === "2"))
    .toMatchObject({ timestamp: "2026-08-03T10:06:00.000Z" });
});
