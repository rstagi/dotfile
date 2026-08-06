import { describe, it, expect } from "vitest";
import { materialize, summarize } from "./materialize.ts";
import { emptyRecord, reduceLoop } from "./reduce-loop.ts";
import type { Ingest, LoopRecord } from "./store-types.ts";
import type { LoopInput } from "./parse-loop.ts";
import type { StateJson } from "./types.ts";

const PLAN = `# Checkout revamp — Multi-Phase Plan

## Loop config
- **Integration branch:** \`feat/checkout\`
- **Verify:** \`npm test\`

## Goal
Rebuild checkout.

## Phases

### Phase 1 — Payments gateway \`[lane: A]\` \`[status: todo]\`
- **Depends on:** none

### Phase 2 — Cart state machine \`[lane: B]\` \`[status: todo]\`
- **Depends on:** none

### Phase 3 — Wire checkout \`[lane: A]\` \`[status: todo]\`
- **Depends on:** Phase 1

### Phase 4 — Integrate \`[lane: integration]\` \`[status: todo]\`
- **Depends on:** Phase 2, Phase 3
`;

function fold(runId: string, ...msgs: Ingest[]): LoopRecord {
  return msgs.reduce((rec, m) => reduceLoop(rec, m), emptyRecord(runId));
}

const register: Ingest = {
  kind: "register",
  info: { runId: "r", effort: "Checkout revamp", integrationBranch: "feat/checkout", planText: PLAN },
};

const stateAll: Ingest = {
  kind: "state",
  state: {
    runId: "r",
    effort: "Checkout revamp",
    phases: {
      "1": { slug: "payments-gateway", status: "merged" },
      "2": { slug: "cart-state-machine", status: "blocked" },
      "3": { slug: "wire-checkout", status: "running" },
      "4": { slug: "integrate", status: "todo" },
    } as StateJson["phases"],
  },
};

const emptyLive: LoopInput = { present: true, state: null, events: null, runs: [], hil: [] };

describe("materialize — overlay drives the node status through the real buildSnapshot", () => {
  it("promotes phase 3 to done in the graph even though state.json still says running", () => {
    const rec = fold("r", register, stateAll, {
      kind: "event",
      event: { event: "phase.attempt.finish", phase: "3", outcome: "done", exitCode: 0 },
    });
    const snap = materialize(rec, emptyLive, { now: 1_000_000, nowIso: "2026-08-03T10:00:00Z" });
    const node3 = snap.graph.nodes.find((n) => n.id === "3")!;
    expect(node3.status).toBe("done");
    expect(node3.ui).toBe("done");
    // the un-promoted, blocked phase still reads blocked
    const node2 = snap.graph.nodes.find((n) => n.id === "2")!;
    expect(node2.status).toBe("blocked");
  });
});

describe("materialize — heartbeat comes from live mtimes, independent of the overlay", () => {
  it("reads the running phase's pulse from the live transcript mtime", () => {
    const now = 10_000_000;
    const rec = fold("r", register, {
      kind: "state",
      state: { phases: { "3": { slug: "wire-checkout", status: "running" } } } as StateJson,
    });
    const live: LoopInput = {
      present: true,
      state: null,
      events: null,
      runs: [
        {
          name: "wire-checkout-a1",
          meta: null, // in flight — no meta.json yet
          status: null,
          spawnLog: null,
          transcriptMtime: now - 5_000, // 5s ago → live
          metaMtime: null,
        },
      ],
      hil: [],
    };
    const snap = materialize(rec, live, { now, nowIso: "2026-08-03T10:00:00Z" });
    const node3 = snap.graph.nodes.find((n) => n.id === "3")!;
    expect(node3.pulse).toBe("live");
    expect(node3.runtime?.lastHeartbeatAgeSec).toBe(5);
  });
});

describe("materialize — archived loop (live = null)", () => {
  it("short-circuits to the stored lastSnapshot", () => {
    const rec = fold("r", register, stateAll);
    const snap = materialize(rec, emptyLive, { now: 1, nowIso: "2026-08-03T10:00:00Z" });
    const archived = materialize({ ...rec, lastSnapshot: snap }, null, {});
    expect(archived).toBe(snap);
  });

  it("still returns a valid snapshot when no lastSnapshot was stored", () => {
    const rec = fold("r", register, stateAll);
    const snap = materialize(rec, null, { now: 1, nowIso: "2026-08-03T10:00:00Z" });
    expect(snap.graph.nodes.length).toBeGreaterThan(0);
    expect(snap.effort.name).toBe("Checkout revamp");
  });
});

describe("materialize — timeline is deduped", () => {
  it("does not double-render an event pushed twice", () => {
    const ev: Ingest = {
      kind: "event",
      event: { event: "spawned", phase: "1", detail: "codex", ts: "2026-08-03T09:00:00Z" },
    };
    const rec = fold("r", register, ev, ev);
    const snap = materialize(rec, emptyLive, { now: 1, nowIso: "2026-08-03T10:00:00Z" });
    expect(snap.events.filter((e) => e.event === "spawned")).toHaveLength(1);
  });
});

describe("materialize — sub-orchestrator health + pendingHil on the snapshot", () => {
  it("has null subOrch and pendingHil 0 with no sub/HIL activity", () => {
    const rec = fold("r", register, stateAll);
    const snap = materialize(rec, emptyLive, { now: 1, nowIso: "2026-08-03T10:00:00Z" });
    expect(snap.subOrch).toBeNull();
    expect(snap.pendingHil).toBe(0);
  });

  it("populates subOrch after a sub.recycle fold", () => {
    const rec = fold("r", register, stateAll, {
      kind: "event",
      event: { event: "sub.recycle", tokens: 151000, recycleIndex: 2, percent: 75 },
    });
    const snap = materialize(rec, emptyLive, { now: 1, nowIso: "2026-08-03T10:00:00Z" });
    expect(snap.subOrch).toEqual({ recycles: 2, contextTokens: 151000, contextPct: 75 });
  });

  it("counts pendingHil from open phase HILs (snapshot + summarize agree)", () => {
    const rec = fold("r", register, stateAll, { kind: "event", event: { event: "hil.raise", phase: "2" } });
    const snap = materialize(rec, emptyLive, { now: 1, nowIso: "2026-08-03T10:00:00Z" });
    expect(snap.pendingHil).toBe(1);
    expect(summarize(rec).pendingHil).toBe(1);
  });
});

describe("summarize — phase counts + lifecycle", () => {
  it("counts effective phase statuses (promotion included)", () => {
    const rec = fold("r", register, stateAll, {
      kind: "event",
      event: { event: "phase.attempt.finish", phase: "3", outcome: "done", exitCode: 0 },
    });
    const sum = summarize(rec);
    expect(sum.phaseCounts).toEqual({ total: 4, todo: 1, running: 0, blocked: 1, done: 1, merged: 1 });
    expect(sum.effort).toBe("Checkout revamp");
    expect(sum.integrationBranch).toBe("feat/checkout");
    expect(sum.status).toBe("active");
  });

  it("reports paused when a HIL is open and finished after loop.finish", () => {
    const paused = fold("r", register, { kind: "event", event: { event: "hil.raise", phase: "2" } });
    expect(summarize(paused).status).toBe("paused");
    const done = reduceLoop(paused, { kind: "finish", info: { finishedAt: "2026-08-03T11:00:00Z" } });
    expect(summarize(done).status).toBe("finished");
  });
});
