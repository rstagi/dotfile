import { afterEach, describe, expect, it } from "vitest";
import { cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { readLoop } from "./index.ts";

const FIXTURE = fileURLToPath(new URL("../../../../loop-web/fixtures/.loop", import.meta.url));
const NOW = "2026-08-03T10:10:00.000Z";
const temporary: string[] = [];

afterEach(() => {
  for (const dir of temporary.splice(0)) rmSync(dir, { recursive: true, force: true });
});

describe("Loop adapter", () => {
  it("reads the fixture run, lanes, phase history and human request", async () => {
    const events = await readLoop(fixture(), { now: NOW });
    expect(events.find((event) => event.type === "run.meta")).toMatchObject({
      runId: "loop-checkout-revamp-2026-08-03",
      meta: {
        title: "Checkout revamp", source: "loop", status: "done",
        lanes: [
          { id: "A", title: "Lane A", phaseIds: ["1", "3"] },
          { id: "B", title: "Lane B", phaseIds: ["2"] },
          { id: "integration", title: "Lane integration", phaseIds: ["4"] },
        ],
      },
    });
    expect(events.find((event) => event.type === "phase.start" && event.phaseId === "1"))
      .toMatchObject({ timestamp: "2026-08-03T09:00:05.000Z", laneId: "A", phase: { id: "1", title: "Add the payments gateway" } });
    expect(events.find((event) => event.type === "phase.end" && event.phaseId === "1"))
      .toMatchObject({ timestamp: "2026-08-03T09:22:00.000Z", phase: { id: "1", status: "done" } });
    expect(events.some((event) => event.type === "phase.start" && event.phaseId === "4")).toBe(false);
    expect(events.filter((event) => event.type === "ask")).toMatchObject([
      { phaseId: "2", ask: { phaseId: "2", resolved: false, text: expect.stringContaining("cart API contract") } },
    ]);
  });

  it.each(["file", "directory", "json"])("ends the run when its %s lock PID dies", async (format) => {
    const dir = fixture();
    const lock = join(dir, "lock");
    if (format === "directory") {
      mkdirSync(lock);
      writeFileSync(join(lock, "pid"), String(process.pid));
    } else {
      writeFileSync(lock, format === "json" ? JSON.stringify({ pid: process.pid }) : String(process.pid));
    }
    const active = await readLoop(dir, { now: NOW });
    expect(active.find((event) => event.type === "run.meta")).toMatchObject({ meta: { status: "active" } });
    const done = await readLoop(dir, { now: NOW, isPidAlive: () => false });
    expect(done.find((event) => event.type === "run.meta")).toMatchObject({ meta: { status: "done" } });
  });

  it("keeps an owner-only lock active until the lock is removed", async () => {
    const dir = fixture();
    mkdirSync(join(dir, "lock"));
    writeFileSync(join(dir, "lock", "owner"), "loop-checkout-revamp-2026-08-03");
    expect((await readLoop(dir, { now: NOW })).find((event) => event.type === "run.meta"))
      .toMatchObject({ meta: { status: "active" } });
    rmSync(join(dir, "lock"), { recursive: true });
    expect((await readLoop(dir, { now: NOW })).find((event) => event.type === "run.meta"))
      .toMatchObject({ meta: { status: "done" } });
  });
});

function fixture(): string {
  const dir = mkdtempSync(join(tmpdir(), "cockpit-loop-"));
  temporary.push(dir);
  cpSync(FIXTURE, dir, { recursive: true });
  return dir;
}
