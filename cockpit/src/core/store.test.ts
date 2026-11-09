import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test } from "vitest";
import { EventStore } from "./store.ts";

const NOW = Date.parse("2026-11-01T12:00:00Z");
const homes: string[] = [];
afterEach(async () => {
  await Promise.all(homes.splice(0).map((home) => rm(home, { recursive: true, force: true })));
});

async function temporaryHome() {
  const home = await mkdtemp(join(tmpdir(), "cockpit-test-"));
  homes.push(home);
  return home;
}

test("append preserves each event in its run JSONL file", async () => {
  const home = await temporaryHome();
  const store = await EventStore.open({ home, now: () => NOW });
  const event = {
    type: "action" as const, id: "event-1", runId: "run-1",
    timestamp: "2026-11-01T12:00:00Z",
    action: { id: "action-1", tool: "read_file", summary: "Read the source file" },
  };
  await store.append(event);
  await store.append({ ...event, id: "event-2", action: { ...event.action, id: "action-2" } });
  const lines = (await readFile(join(home, "runs/run-1.jsonl"), "utf8")).trim().split("\n");
  expect(lines.map((line) => JSON.parse(line).id)).toEqual(["event-1", "event-2"]);
  expect(store.getRun("run-1")?.actions.map((action) => action.id)).toEqual(["action-1", "action-2"]);
});

test("restart rebuilds metadata and updated entities from all run files", async () => {
  const home = await temporaryHome();
  const store = await EventStore.open({ home, now: () => NOW });
  const base = { runId: "run-1", timestamp: "2026-11-01T12:00:00Z" };
  await store.append({ ...base, id: "meta", type: "run.meta", meta: { title: "Build the cockpit", status: "active", lanes: [{ id: "A", title: "Store" }] } });
  await store.append({ ...base, id: "phase", type: "phase.start", phase: { id: "p1", title: "Build the store", inferred: true } });
  await store.append({ ...base, id: "intent1", type: "intent.upsert", intent: { id: "i1", text: "Read the source", actionIds: [] } });
  await store.append({ ...base, id: "intent2", type: "intent.upsert", intent: { id: "i1", text: "Read the source to copy the pure functions", actionIds: ["a1"] } });
  await store.append({ ...base, id: "group", type: "group.upsert", group: { id: "g1", text: "Copy the intent engine", intentIds: ["i1"] } });
  await store.append({ ...base, id: "ask", type: "ask", ask: { id: "q1", text: "Which source is correct?" } });
  await store.append({ ...base, id: "resolve", type: "ask", ask: { id: "q1", text: "Which source is correct?", resolved: true } });
  await store.append({ ...base, id: "media", type: "media", media: { id: "m1", path: "/tmp/example.png", mimeType: "image/png" } });
  await store.append({ ...base, id: "drift", type: "drift", drift: { id: "d1", summary: "The phase needs five more minutes" } });
  await store.append({ ...base, id: "checkpoint", type: "checkpoint", checkpoint: { id: "c1", summary: "The store is ready" } });
  await store.append({ ...base, id: "end", timestamp: "2026-11-01T12:01:00Z", type: "phase.end", phase: { id: "p1", status: "done" } });
  await store.append({ ...base, runId: "run-2", id: "other", type: "run.meta", meta: { title: "Other session" } });
  const restarted = await EventStore.open({ home, now: () => NOW });
  expect(restarted.snapshot()).toEqual(store.snapshot());
  const run = restarted.getRun("run-1")!;
  expect(run.meta.title).toBe("Build the cockpit");
  expect(run.intents).toEqual([{ id: "i1", text: "Read the source to copy the pure functions", actionIds: ["a1"] }]);
  expect(run.phases).toEqual([{ id: "p1", title: "Build the store", inferred: true, status: "done", startedAt: base.timestamp, endedAt: "2026-11-01T12:01:00Z" }]);
  expect(run.asks).toHaveLength(1);
  expect(run.asks[0].resolved).toBe(true);
  expect(run.groups[0].intentIds).toEqual(["i1"]);
  expect(run.media[0].id).toBe("m1");
  expect(run.drifts[0].id).toBe("d1");
  expect(run.checkpoints[0].id).toBe("c1");
  expect(await restarted.events("run-1")).toHaveLength(11);
  run.intents.length = 0;
  expect(restarted.getRun("run-1")?.intents).toHaveLength(1);
});

test("retention deletes inactive runs older than 30 days and keeps the boundary", async () => {
  const home = await temporaryHome();
  const now = Date.parse("2026-11-01T12:00:00Z");
  const store = await EventStore.open({ home, now: () => now });
  for (const [runId, timestamp] of [
    ["expired", "2026-10-02T11:59:59Z"],
    ["boundary", "2026-10-02T12:00:00Z"],
    ["active", "2026-09-01T12:00:00Z"],
  ]) {
    await store.append({ id: runId, runId, timestamp, type: "run.meta", meta: { title: runId } });
  }
  await store.append({ id: "recent", runId: "active", timestamp: "2026-11-01T11:00:00Z", type: "run.meta", meta: { status: "active" } });
  // A delayed event must not move the run's last activity backwards.
  await store.append({ id: "delayed", runId: "active", timestamp: "2026-09-02T12:00:00Z", type: "run.meta", meta: { branch: "feature/store" } });
  const reopened = await EventStore.open({ home, now: () => now });
  expect(reopened.snapshot().map((run) => run.id)).toEqual(["active", "boundary"]);
  await expect(readFile(join(home, "runs/expired.jsonl"))).rejects.toMatchObject({ code: "ENOENT" });
  expect(await reopened.prune(now + 1000)).toEqual(["boundary"]);
  expect(reopened.getRun("boundary")).toBeUndefined();
});
