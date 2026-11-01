import { afterEach, describe, expect, it } from "vitest";
import { cpSync, mkdtempSync, rmSync } from "node:fs";
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
});

function fixture(): string {
  const dir = mkdtempSync(join(tmpdir(), "cockpit-loop-"));
  temporary.push(dir);
  cpSync(FIXTURE, dir, { recursive: true });
  return dir;
}
