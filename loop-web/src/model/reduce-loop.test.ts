import { describe, it, expect } from "vitest";
import { emptyRecord, reduceLoop, effectivePhaseStatus } from "./reduce-loop.ts";
import type { Ingest, LoopRecord } from "./store-types.ts";
import type { StateJson } from "./types.ts";

// A convenience folder so a test reads as a sequence of ingests over the empty record.
function fold(runId: string, ...msgs: Ingest[]): LoopRecord {
  return msgs.reduce((rec, m) => reduceLoop(rec, m), emptyRecord(runId));
}

const stateWith = (phases: StateJson["phases"], extra: Partial<StateJson> = {}): Ingest => ({
  kind: "state",
  state: { runId: "r", effort: "Demo", phases, ...extra },
});

describe("reduceLoop — register", () => {
  it("seeds identity + planText and marks the loop active", () => {
    const rec = fold("run-1", {
      kind: "register",
      info: {
        runId: "run-1",
        effort: "Checkout revamp",
        projectId: "proj_shop",
        loopDir: "/repo/.kestral/loop",
        planFile: "/repo/.kestral/plan.md",
        integrationBranch: "feat/checkout",
        startedAt: "2026-08-03T09:00:00Z",
        planText: "# Checkout revamp — Multi-Phase Plan\n",
      },
    });
    expect(rec.runId).toBe("run-1");
    expect(rec.effort).toBe("Checkout revamp");
    expect(rec.integrationBranch).toBe("feat/checkout");
    expect(rec.planText).toMatch(/Checkout revamp/);
    expect(rec.status).toBe("active");
    expect(rec.startedAt).toBe("2026-08-03T09:00:00Z");
  });
});

describe("reduceLoop — promotion lattice (the staleness fix)", () => {
  it("promotes a phase to done from a finished attempt even while state.json still says running", () => {
    const rec = fold(
      "r",
      stateWith({ "3": { slug: "wire-checkout", status: "running" } }),
      {
        kind: "event",
        event: { event: "phase.attempt.finish", phase: "3", outcome: "done", exitCode: 0 },
      },
    );
    // state.json still says running, but the overlay promotes the effective status to done.
    expect(rec.lastState?.phases?.["3"]?.status).toBe("running");
    expect(rec.phases["3"].rank).toBe("done");
    expect(effectivePhaseStatus(rec, "3")).toBe("done");
  });

  it("promotes to merged and never regresses below it (merged over a later running push)", () => {
    const rec = fold(
      "r",
      stateWith({ "1": { slug: "pg", status: "running" } }),
      { kind: "event", event: { event: "phase.merged", phase: "1" } },
      // a stale/late state push tries to move it back to running — must be ignored
      stateWith({ "1": { slug: "pg", status: "running" } }),
    );
    expect(rec.phases["1"].rank).toBe("merged");
    expect(effectivePhaseStatus(rec, "1")).toBe("merged");
  });

  it("does NOT promote on a verify-fail (exit 12) — records a problem, keeps running", () => {
    const rec = fold(
      "r",
      stateWith({ "2": { slug: "cart", status: "running" } }),
      {
        kind: "event",
        event: { event: "phase.attempt.finish", phase: "2", outcome: "done", exitCode: 12 },
      },
    );
    expect(rec.phases["2"].rank).toBe("running");
    expect(rec.phases["2"].problem).toBe("verify-fail");
    expect(effectivePhaseStatus(rec, "2")).toBe("running");
  });

  it("is order-independent: a late attempt.start after a merged event does not regress", () => {
    const rec = fold(
      "r",
      { kind: "event", event: { event: "phase.merged", phase: "5" } },
      { kind: "event", event: { event: "phase.attempt.start", phase: "5", engine: "codex" } },
    );
    expect(rec.phases["5"].rank).toBe("merged");
  });

  it("lets a blocked state move back to running (blocked is not a rank)", () => {
    const rec = fold(
      "r",
      stateWith({ "2": { slug: "cart", status: "running" } }),
      stateWith({ "2": { slug: "cart", status: "blocked" } }),
    );
    expect(effectivePhaseStatus(rec, "2")).toBe("blocked");
    const rec2 = reduceLoop(rec, stateWith({ "2": { slug: "cart", status: "running" } }));
    expect(effectivePhaseStatus(rec2, "2")).toBe("running");
  });
});

describe("reduceLoop — timeline dedup & idempotency", () => {
  it("dedupes identical events by ts|event|phase|detail", () => {
    const ev: Ingest = {
      kind: "event",
      event: { event: "spawned", phase: "1", detail: "codex", ts: "2026-08-03T09:00:00Z" },
    };
    const rec = fold("r", ev, ev, ev);
    expect(rec.events.filter((e) => e.event === "spawned")).toHaveLength(1);
  });

  it("re-folding the same eventsFile is idempotent", () => {
    const jsonl =
      `{"ts":"2026-08-03T09:00:00Z","event":"scheduled","phase":"","detail":"start"}\n` +
      `{"ts":"2026-08-03T09:22:00Z","event":"merged","phase":"1","detail":"lane->int"}\n`;
    const once = fold("r", { kind: "eventsFile", text: jsonl });
    const twice = reduceLoop(once, { kind: "eventsFile", text: jsonl });
    expect(twice.events).toHaveLength(2);
    expect(once.events).toHaveLength(2);
  });

  it("an eventsFile 'merged' line still drives promotion (legacy keyword fallback)", () => {
    const jsonl = `{"ts":"2026-08-03T09:22:00Z","event":"merged","phase":"1","detail":"lane->int"}\n`;
    const rec = fold("r", { kind: "eventsFile", text: jsonl });
    expect(rec.phases["1"].rank).toBe("merged");
  });

  it("a free-form/unknown event name is kept on the timeline but promotes nothing", () => {
    const rec = fold("r", {
      kind: "event",
      event: { event: "quantum-flux", phase: "3", detail: "unknown", ts: "2026-08-03T10:05:00Z" },
    });
    expect(rec.events).toHaveLength(1);
    expect(rec.phases["3"]?.rank ?? "todo").toBe("todo");
  });
});

describe("reduceLoop — HIL raise/resolve", () => {
  it("raises then resolves a phase's HIL flag", () => {
    const raised = fold("r", { kind: "event", event: { event: "hil.raise", phase: "2" } });
    expect(raised.phases["2"].hilOpen).toBe(true);
    expect(raised.status).toBe("paused");
    const resolved = reduceLoop(raised, { kind: "event", event: { event: "hil.resolve", phase: "2" } });
    expect(resolved.phases["2"].hilOpen).toBe(false);
    expect(resolved.status).toBe("active");
  });
});

describe("reduceLoop — sub-orchestrator recycles & occupancy", () => {
  it("folds sub.saturation then sub.recycle into occupancy + subRecycles", () => {
    const rec = fold(
      "r",
      { kind: "event", event: { event: "sub.saturation", tokens: 120000, percent: 60 } },
      { kind: "event", event: { event: "sub.recycle", tokens: 151000, recycleIndex: 1 } },
    );
    expect(rec.subRecycles).toBe(1);
    expect(rec.occupancy).toEqual({ tokens: 151000, percent: null });
  });

  it("is idempotent under re-fold — subRecycles stays at the max, does not keep growing", () => {
    const jsonl =
      `{"ts":"2026-08-03T09:00:00Z","event":"sub.saturation","phase":"","detail":""}\n` +
      `{"ts":"2026-08-03T09:10:00Z","event":"sub.recycle","phase":"","detail":""}\n`;
    // Direct event fold with the full body first (the live daemon POST path).
    const live = fold(
      "r",
      { kind: "event", event: { event: "sub.recycle", tokens: 151000, recycleIndex: 2, ts: "2026-08-03T09:10:00Z" } },
    );
    expect(live.subRecycles).toBe(2);
    // Re-folding the same event twice never multiplies the count.
    const twice = reduceLoop(live, { kind: "event", event: { event: "sub.recycle", tokens: 151000, recycleIndex: 2, ts: "2026-08-03T09:10:00Z" } });
    expect(twice.subRecycles).toBe(2);
    // A re-folded events.jsonl (tokens/recycleIndex dropped by rawToEventInfo) is a no-op:
    // it PRESERVES occupancy/subRecycles rather than resetting them.
    const refolded = reduceLoop(live, { kind: "eventsFile", text: jsonl });
    expect(refolded.subRecycles).toBe(2);
    expect(refolded.occupancy).toEqual({ tokens: 151000, percent: null });
  });

  it("last-write occupancy can drop, but subRecycles never decreases", () => {
    const rec = fold(
      "r",
      { kind: "event", event: { event: "sub.recycle", tokens: 160000, recycleIndex: 3, percent: 80 } },
      // a stale recycle with a lower index must NOT lower the count (max-fold)…
      { kind: "event", event: { event: "sub.recycle", tokens: 40000, recycleIndex: 1 } },
      // …and a later heartbeat with FEWER tokens still updates occupancy (last-write).
      { kind: "event", event: { event: "sub.saturation", tokens: 40000, percent: 20 } },
    );
    expect(rec.occupancy).toEqual({ tokens: 40000, percent: 20 });
    expect(rec.subRecycles).toBe(3);
  });
});

describe("reduceLoop — loop.finish", () => {
  it("marks the loop finished and captures the review + prUrl", () => {
    const rec = fold(
      "r",
      { kind: "register", info: { runId: "r" } },
      {
        kind: "finish",
        info: {
          status: "integrating",
          finishedAt: "2026-08-03T11:00:00Z",
          prUrl: "https://github.com/acme/shop/pull/128",
          review: {
            outcome: "done",
            summary: "LGTM, ship it",
            reportPath: "runs/review-a1/report.md",
            commentUrl: "https://github.com/acme/shop/pull/128#issuecomment-1",
          },
        },
      },
    );
    expect(rec.status).toBe("finished");
    expect(rec.finishedAt).toBe("2026-08-03T11:00:00Z");
    expect(rec.prUrl).toBe("https://github.com/acme/shop/pull/128");
    expect(rec.review?.outcome).toBe("done");
    expect(rec.review?.reportPath).toBe("runs/review-a1/report.md");
  });

  it("keeps the loop finished even if a stray late state push arrives after finish", () => {
    const rec = fold(
      "r",
      { kind: "finish", info: { finishedAt: "2026-08-03T11:00:00Z" } },
      stateWith({ "1": { slug: "pg", status: "running" } }),
    );
    expect(rec.status).toBe("finished");
  });
});

describe("reduceLoop — purity", () => {
  it("does not mutate the previous record", () => {
    const prev = emptyRecord("r");
    const next = reduceLoop(prev, { kind: "event", event: { event: "spawned", phase: "1" } });
    expect(prev.events).toHaveLength(0);
    expect(next.events).toHaveLength(1);
    expect(next).not.toBe(prev);
  });
});
