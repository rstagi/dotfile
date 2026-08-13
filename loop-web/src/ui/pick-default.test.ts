import { describe, it, expect } from "vitest";
import { pickDefaultRunId } from "./pick-default.ts";
import type { LoopListEntry } from "../model/store-types.ts";

function entry(over: Partial<LoopListEntry> & { runId: string }): LoopListEntry {
  return {
    effort: over.runId,
    status: "active",
    integrationBranch: null,
    startedAt: null,
    finishedAt: null,
    updatedAt: null,
    prUrl: null,
    reviewOutcome: null,
    repositories: [],
    phaseCounts: { total: 0, todo: 0, running: 0, blocked: 0, done: 0, merged: 0 },
    pendingHil: 0,
    ...over,
  };
}

describe("pickDefaultRunId", () => {
  it("returns null for an empty list", () => {
    expect(pickDefaultRunId([], "x")).toBeNull();
  });
  it("keeps the current runId when it is still present", () => {
    const loops = [entry({ runId: "a" }), entry({ runId: "b" })];
    expect(pickDefaultRunId(loops, "b")).toBe("b");
  });
  it("picks the most-recent active loop when current is absent", () => {
    const loops = [
      entry({ runId: "old", status: "active", updatedAt: "2026-08-01T00:00:00Z" }),
      entry({ runId: "new", status: "active", updatedAt: "2026-08-03T00:00:00Z" }),
      entry({ runId: "fin", status: "finished", updatedAt: "2026-08-04T00:00:00Z" }),
    ];
    expect(pickDefaultRunId(loops, "gone")).toBe("new");
  });
  it("prefers a planned loop over a finished one when none are active", () => {
    const loops = [
      entry({ runId: "fin", status: "finished", updatedAt: "2026-08-05T00:00:00Z" }),
      entry({ runId: "plan", status: "planned", updatedAt: "2026-08-02T00:00:00Z" }),
    ];
    expect(pickDefaultRunId(loops, null)).toBe("plan");
  });
  it("still prefers an active loop over a planned one", () => {
    const loops = [
      entry({ runId: "plan", status: "planned", updatedAt: "2026-08-09T00:00:00Z" }),
      entry({ runId: "act", status: "active", updatedAt: "2026-08-01T00:00:00Z" }),
    ];
    expect(pickDefaultRunId(loops, null)).toBe("act");
  });
  it("falls back to the most-recent finished loop when none are active or planned", () => {
    const loops = [
      entry({ runId: "f1", status: "finished", updatedAt: "2026-08-01T00:00:00Z" }),
      entry({ runId: "f2", status: "finished", updatedAt: "2026-08-02T00:00:00Z" }),
      entry({ runId: "arc", status: "archived", updatedAt: "2026-08-09T00:00:00Z" }),
    ];
    expect(pickDefaultRunId(loops, null)).toBe("f2");
  });
  it("falls back to the first entry when nothing is active or finished", () => {
    const loops = [entry({ runId: "p", status: "paused" }), entry({ runId: "a", status: "archived" })];
    expect(pickDefaultRunId(loops, null)).toBe("p");
  });
});
