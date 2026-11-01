import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test, vi } from "vitest";
import type { CockpitEvent } from "./events.ts";
import { EventStore } from "./store.ts";

const NOW = Date.parse("2026-11-01T12:00:00Z");
const homes: string[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(homes.splice(0).map((home) => rm(home, { recursive: true, force: true })));
});

async function temporaryHome() {
  const home = await mkdtemp(join(tmpdir(), "cockpit-contract-"));
  homes.push(home);
  return home;
}

function action(id: string): CockpitEvent {
  return {
    id, runId: "run-1", timestamp: "2026-11-01T12:00:00Z", type: "action",
    action: { id, tool: "read_file", summary: "Read a test fixture" },
  };
}

test("the home environment override controls persistence", async () => {
  const home = await temporaryHome();
  vi.stubEnv("COCKPIT_HOME", home);
  const store = await EventStore.open({ now: () => NOW });
  await store.append(action("a1"));
  expect(JSON.parse(await readFile(join(home, "runs/run-1.jsonl"), "utf8"))).toEqual(action("a1"));
});

test("concurrent appends preserve submission order and capture inputs", async () => {
  const home = await temporaryHome();
  const store = await EventStore.open({ home, now: () => NOW });
  const first = action("a0");
  const pending = store.append(first);
  first.id = "changed";
  if (first.type === "action") first.action.summary = "Changed after submission";
  await Promise.all([pending, ...Array.from({ length: 20 }, (_, i) => store.append(action(`a${i + 1}`)))]);
  const events = await store.events("run-1");
  expect(events.map((event) => event.id)).toEqual(Array.from({ length: 21 }, (_, i) => `a${i}`));
  expect(events[0]).toEqual(action("a0"));
  const reopened = await EventStore.open({ home, now: () => NOW });
  expect(reopened.snapshot()).toEqual(store.snapshot());
});

test("invalid IDs cannot escape the runs directory or stop later appends", async () => {
  const home = await temporaryHome();
  const store = await EventStore.open({ home, now: () => NOW });
  await expect(store.append({ ...action("bad"), runId: "../../outside" })).rejects.toThrow("The run ID");
  await expect(store.append({ ...action("bad-time"), timestamp: "invalid" })).rejects.toThrow("valid timestamp");
  await store.append(action("valid"));
  expect(store.getRun("run-1")?.actions.map((item) => item.id)).toEqual(["valid"]);
  await expect(store.events("../outside")).rejects.toThrow("The run ID");
});

test("replay rejects corrupt logs with the run and line, without the event content", async () => {
  const home = await temporaryHome();
  const store = await EventStore.open({ home, now: () => NOW });
  await store.append(action("valid"));
  await writeFile(join(home, "runs/run-1.jsonl"), `${JSON.stringify(action("valid"))}\ninvalid fixture content\n`);
  await expect(EventStore.open({ home, now: () => NOW })).rejects.toThrow("The event log for run-1 has an invalid event at line 2.");
  await writeFile(join(home, "runs/run-1.jsonl"), `${JSON.stringify({ ...action("wrong-run"), runId: "run-2" })}\n`);
  await expect(EventStore.open({ home, now: () => NOW })).rejects.toThrow("The event log for run-1 has an invalid event at line 1.");
});
