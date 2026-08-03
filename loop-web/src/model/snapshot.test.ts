import { describe, it, expect } from "vitest";
import { buildSnapshot } from "./snapshot.ts";
import { parsePlan } from "./parse-plan.ts";
import { parseLoop } from "./parse-loop.ts";
import type { LoopInput, RawRunDir } from "./parse-loop.ts";

const PLAN = `# Widget revamp — Multi-Phase Plan

**Status:** in progress
**Last updated:** 2026-08-01 by Codex (feat/widget-core)

## Loop config

- **Integration branch:** \`feat/widget-revamp\`
- **Verify:** \`npm test\`
- **PR:** _none yet_
- **Concurrency:** 3

## Phases

### Phase 1 — Widget core \`[lane: A]\` \`[status: done]\`
- **Depends on:** none

### Phase 2 — Build feature \`[lane: A]\` \`[status: in-progress]\`
- **Depends on:** Phase 1
`;

function run(over: Partial<RawRunDir> & { name: string }): RawRunDir {
  return { meta: null, status: null, spawnLog: null, transcriptMtime: null, metaMtime: null, ...over };
}
function loop(over: Partial<LoopInput> = {}): LoopInput {
  return { present: true, state: null, events: null, runs: [], hil: [], ...over };
}

const NOW = 1_000_000_000_000;
const ISO = "2026-08-03T10:00:00Z";

describe("buildSnapshot — effort info", () => {
  it("assembles name/status/updatedAt and prefers state.json for the integration branch & PR", () => {
    const state = JSON.stringify({
      runId: "loop-widget-2026",
      integrationBranch: "feat/widget-from-state",
      prUrl: "https://github.com/acme/widgets/pull/9",
      phases: { "2": { slug: "build-feature", status: "running" } },
    });
    const snap = buildSnapshot(parsePlan(PLAN), parseLoop(loop({ state })), { now: NOW, nowIso: ISO });
    expect(snap.effort.name).toBe("Widget revamp");
    expect(snap.effort.status).toBe("in progress");
    expect(snap.effort.integrationBranch).toBe("feat/widget-from-state");
    expect(snap.effort.pr?.url).toBe("https://github.com/acme/widgets/pull/9");
    expect(snap.effort.runId).toBe("loop-widget-2026");
    expect(snap.generatedAt).toBe(ISO);
  });

  it("falls back to the plan's loop config when state has no branch/PR", () => {
    const snap = buildSnapshot(parsePlan(PLAN), null, { now: NOW });
    expect(snap.effort.integrationBranch).toBe("feat/widget-revamp");
    expect(snap.effort.pr).toBeNull(); // "_none yet_"
    expect(snap.loopActive).toBe(false);
  });
});

describe("buildSnapshot — problems rail", () => {
  it("lists HIL loudest, then other problems, ranked by severity", () => {
    const state = JSON.stringify({
      phases: {
        "1": { slug: "widget-core", status: "running" },
        "2": { slug: "build-feature", status: "running" },
      },
    });
    const snap = buildSnapshot(
      parsePlan(PLAN),
      parseLoop(
        loop({
          state,
          runs: [run({ name: "widget-core-a1", meta: JSON.stringify({ engineExit: 124, timedOut: true }) })],
          hil: [{ slug: "build-feature", markdown: "## Decide\nA or B?", answered: false }],
        }),
      ),
      { now: NOW, nowIso: ISO },
    );
    expect(snap.problems[0].class).toBe("hil");
    expect(snap.problems[0].hilMarkdown).toContain("Decide");
    expect(snap.problems.some((p) => p.class === "timeout")).toBe(true);
    // HIL severity strictly greater than the timeout's
    expect(snap.problems[0].severity).toBeGreaterThan(
      snap.problems.find((p) => p.class === "timeout")!.severity,
    );
  });

  it("has an empty problems rail in the clean static case", () => {
    expect(buildSnapshot(parsePlan(PLAN), null, { now: NOW }).problems).toEqual([]);
  });

  it("adds no phantom problem for a passing review whose prose says 'blocking'/'changes'", () => {
    const state = JSON.stringify({ phases: { "1": { slug: "widget-core", status: "merged" } } });
    const snap = buildSnapshot(
      parsePlan(PLAN),
      parseLoop(
        loop({
          state,
          runs: [run({ name: "review-a1", status: JSON.stringify({ outcome: "done", summary: "Reviewed the changes; no blocking issues. Approving." }) })],
        }),
      ),
      { now: NOW, nowIso: ISO },
    );
    expect(snap.effort.pr?.outcome).toBe("done");
    expect(snap.problems.filter((p) => p.nodeId === "pr-review")).toEqual([]);
  });
});

describe("buildSnapshot — timeline & loopActive", () => {
  it("normalizes events and sorts them newest-first", () => {
    const events = [
      JSON.stringify({ ts: "2026-08-01T10:00:00Z", event: "spawned", phase: "1" }),
      JSON.stringify({ ts: "2026-08-01T12:00:00Z", event: "merged", phase: "1" }),
    ].join("\n");
    const snap = buildSnapshot(parsePlan(PLAN), parseLoop(loop({ events })), { now: NOW });
    expect(snap.events.map((e) => e.event)).toEqual(["merged", "spawned"]);
    expect(snap.events[0].glyph).not.toBe("");
  });

  it("reports loopActive when a phase is running with a fresh in-flight attempt", () => {
    const state = JSON.stringify({ phases: { "2": { slug: "build-feature", status: "running" } } });
    const snap = buildSnapshot(
      parsePlan(PLAN),
      parseLoop(loop({ state, runs: [run({ name: "build-feature-a1", transcriptMtime: NOW - 1000 })] })),
      { now: NOW },
    );
    expect(snap.loopActive).toBe(true);
  });
});
