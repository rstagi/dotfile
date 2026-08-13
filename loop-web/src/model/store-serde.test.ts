import { describe, it, expect } from "vitest";
import { serializeRecord, parseStoreFile } from "./store-serde.ts";
import { emptyRecord, reduceLoop } from "./reduce-loop.ts";
import { summarize } from "./materialize.ts";
import type { LoopRecord } from "./store-types.ts";
import { EVENT_CAP, STORE_SCHEMA_VERSION } from "./store-types.ts";

function sampleRecord(): LoopRecord {
  return [
    { kind: "register", info: { runId: "loop-x", effort: "X", integrationBranch: "feat/x", planText: "# X — Multi-Phase Plan\n" } },
    { kind: "state", state: { phases: { "1": { slug: "a", status: "merged" }, "2": { slug: "b", status: "running" } } } },
    { kind: "event", event: { event: "phase.attempt.finish", phase: "2", outcome: "done", exitCode: 0, ts: "2026-08-03T10:00:00Z" } },
  ].reduce((rec, m) => reduceLoop(rec, m as never), emptyRecord("loop-x"));
}

describe("store serde — round-trip", () => {
  it("survives serialize → parse with the summary intact", () => {
    const rec = sampleRecord();
    const parsed = parseStoreFile(serializeRecord(rec))!;
    expect(parsed.runId).toBe("loop-x");
    expect(summarize(parsed)).toEqual(summarize(rec));
  });

  it("is a stable fixed point (re-serialize is byte-identical)", () => {
    const s1 = serializeRecord(sampleRecord());
    const s2 = serializeRecord(parseStoreFile(s1)!);
    expect(s2).toBe(s1);
  });

  it("round-trips a NON-ZERO subRecycles / occupancy (cross-restart persistence)", () => {
    const rec = reduceLoop(sampleRecord(), {
      kind: "event",
      event: { event: "sub.recycle", tokens: 151000, recycleIndex: 3, ts: "2026-08-03T11:00:00Z" },
    } as never);
    expect(rec.subRecycles).toBe(3);
    const parsed = parseStoreFile(serializeRecord(rec))!;
    expect(parsed.subRecycles).toBe(3);
    expect(parsed.occupancy).toEqual({ tokens: 151000, percent: null });
  });

  it("stamps the current schema version", () => {
    const parsed = parseStoreFile(serializeRecord(sampleRecord()))!;
    expect(parsed.schemaVersion).toBe(STORE_SCHEMA_VERSION);
  });
});

describe("store serde — corrupt tolerance", () => {
  it("returns null on malformed JSON", () => {
    expect(parseStoreFile("{ not json")).toBeNull();
  });
  it("returns null on a non-object / missing runId", () => {
    expect(parseStoreFile("null")).toBeNull();
    expect(parseStoreFile("[]")).toBeNull();
    expect(parseStoreFile('{"effort":"x"}')).toBeNull();
  });
  it("fills missing optional fields with safe defaults", () => {
    const parsed = parseStoreFile('{"runId":"loop-y"}')!;
    expect(parsed.runId).toBe("loop-y");
    expect(parsed.phases).toEqual({});
    expect(parsed.events).toEqual([]);
    expect(parsed.status).toBe("active");
    expect(parsed.subRecycles).toBe(0);
    expect(parsed.occupancy).toBeNull();
  });

  it("migrates a v1 scalar PR/review record into the primary repository", () => {
    const parsed = parseStoreFile(JSON.stringify({
      schemaVersion: 1,
      runId: "legacy",
      integrationBranch: "feat/legacy",
      prUrl: "https://github.com/acme/app/pull/1",
      review: { outcome: "done", summary: "ok", reportPath: null, commentUrl: null },
      phases: { "1": { rank: "done", hilOpen: false, problem: null } },
    }))!;
    expect(STORE_SCHEMA_VERSION).toBe(2);
    expect(parsed.repositories.primary).toMatchObject({
      integrationBranch: "feat/legacy",
      prUrl: "https://github.com/acme/app/pull/1",
    });
    expect(parsed.phases["1"].repository).toBe("primary");
  });

  it("migrates archived v1 snapshot singular effort data", () => {
    const parsed = parseStoreFile(JSON.stringify({
      schemaVersion: 1,
      runId: "archived",
      lastSnapshot: { effort: { name: "Old", integrationBranch: "feat/old", pr: null }, graph: { nodes: [] } },
    }))!;
    expect(parsed.lastSnapshot?.effort.repositories).toEqual([]);
    expect(parsed.schemaVersion).toBe(2);
  });
});

describe("store serde — event cap", () => {
  it("keeps only the newest EVENT_CAP events on write (drops oldest)", () => {
    const events = Array.from({ length: EVENT_CAP + 500 }, (_, i) => ({
      ts: `t${i}`,
      event: "tick",
      phase: "1",
      detail: String(i),
    }));
    const rec: LoopRecord = { ...emptyRecord("loop-z"), events };
    const parsed = parseStoreFile(serializeRecord(rec))!;
    expect(parsed.events).toHaveLength(EVENT_CAP);
    // newest kept: last detail is the highest index
    expect(parsed.events[parsed.events.length - 1].detail).toBe(String(EVENT_CAP + 499));
    expect(parsed.events[0].detail).toBe(String(500));
  });
});
