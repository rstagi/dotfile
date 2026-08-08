import { describe, it, expect } from "vitest";
import { buildGraph } from "./build-graph.ts";
import { parsePlan } from "./parse-plan.ts";
import type { Runtime, PhaseRuntime, Attempt, Graph, PrInfo } from "./types.ts";

/** Minimal PrInfo for the review-node tests — fills the review-detail fields with nulls. */
function pr(over: Partial<PrInfo>): PrInfo {
  return {
    url: null,
    outcome: null,
    verdict: null,
    reviewPresent: false,
    reportPath: null,
    commentUrl: null,
    reviewSlug: null,
    reviewAttempt: null,
    ...over,
  };
}

const SINGLE_LANE = `# Ship it — Multi-Phase Plan

**Status:** in progress

## Phases

### Phase 1 — Lay the foundation \`[lane: A]\` \`[status: done]\`
- **Depends on:** none

### Phase 2 — Build the feature \`[lane: A]\` \`[status: in-progress]\`
- **Depends on:** Phase 1

### Phase 3 — Polish \`[lane: A]\` \`[status: todo]\`
- **Depends on:** Phase 2
`;

const TWO_LANE = `# Two lanes — Multi-Phase Plan

**Status:** in progress

## Phases

### Phase 1 — Lane A work \`[lane: A]\` \`[status: done]\`
- **Depends on:** none

### Phase 2 — Lane B work \`[lane: B]\` \`[status: todo]\`
- **Depends on:** none

### Phase 3 — Integrate \`[lane: integration]\` \`[status: todo]\`
- **Depends on:** Phase 1, Phase 2
`;

function node(g: Graph, id: string) {
  const n = g.nodes.find((x) => x.id === id);
  if (!n) throw new Error(`no node ${id} (have: ${g.nodes.map((x) => x.id).join(",")})`);
  return n;
}
function hasEdge(g: Graph, source: string, target: string) {
  return g.edges.some((e) => e.source === source && e.target === target);
}
function attempt(over: Partial<Attempt> & { k: number }): Attempt {
  return {
    runDir: `.kestral/loop/runs/slug-a${over.k}`,
    meta: null,
    status: null,
    spawnLog: null,
    transcriptMtime: null,
    endedAt: null,
    ...over,
  };
}
function runtime(phases: Record<string, Partial<PhaseRuntime>>): Runtime {
  const out: Record<string, PhaseRuntime> = {};
  for (const [k, v] of Object.entries(phases)) {
    out[k] = { phase: k, state: null, attempts: [], hil: null, note: null, ...v };
  }
  return { present: true, state: null, phases: out, events: [], reviewRuns: [], reviewNote: null };
}

describe("buildGraph — static shape (no runtime)", () => {
  const g = buildGraph(parsePlan(SINGLE_LANE), null);

  it("has a synthetic Plan root and a synthetic PR Review terminal", () => {
    expect(node(g, "plan").kind).toBe("plan");
    expect(node(g, "pr-review").kind).toBe("pr-review");
  });
  it("emits a phase node per plan phase, in order, with lanes", () => {
    const phaseNodes = g.nodes.filter((n) => n.kind === "phase" || n.kind === "integration");
    expect(phaseNodes.map((n) => n.phase)).toEqual(["1", "2", "3"]);
    expect(g.lanes).toEqual(["A"]);
  });
  it("wires Plan → lane-root (dependency-less) phase only", () => {
    expect(hasEdge(g, "plan", "1")).toBe(true);
    expect(hasEdge(g, "plan", "2")).toBe(false);
  });
  it("wires Depends-on edges between phases", () => {
    expect(hasEdge(g, "1", "2")).toBe(true);
    expect(hasEdge(g, "2", "3")).toBe(true);
  });
  it("flows the terminal phase into PR Review", () => {
    expect(hasEdge(g, "3", "pr-review")).toBe(true);
  });
  it("reads a single lane as Plan → phases → PR Review on one band", () => {
    expect(node(g, "1").status).toBe("done");
    expect(node(g, "2").status).toBe("running"); // in-progress → running
    expect(node(g, "3").ui).toBe("todo");
  });
});

describe("buildGraph — lanes & integration", () => {
  const g = buildGraph(parsePlan(TWO_LANE), null);
  it("orders lanes with integration last", () => {
    expect(g.lanes).toEqual(["A", "B", "integration"]);
  });
  it("marks the integration phase as an integration node", () => {
    expect(node(g, "3").kind).toBe("integration");
  });
  it("wires Plan → both lane roots", () => {
    expect(hasEdge(g, "plan", "1")).toBe(true);
    expect(hasEdge(g, "plan", "2")).toBe(true);
  });
  it("routes the integration node into PR Review (not the lane tails directly)", () => {
    expect(hasEdge(g, "3", "pr-review")).toBe(true);
    expect(hasEdge(g, "1", "pr-review")).toBe(false);
    expect(hasEdge(g, "2", "pr-review")).toBe(false);
  });
  it("marks a dependency edge as blocking while its source is not done", () => {
    // Phase 3 depends on Phase 2 (todo) → that edge blocks; on Phase 1 (done) → satisfied.
    const eFrom2 = g.edges.find((e) => e.source === "2" && e.target === "3")!;
    const eFrom1 = g.edges.find((e) => e.source === "1" && e.target === "3")!;
    expect(eFrom2.blocking).toBe(true);
    expect(eFrom1.blocking).toBe(false);
  });
});

describe("buildGraph — runtime overlay", () => {
  const now = 1_000_000_000_000;

  it("overlays state.json status + branch + attempt + last-worked-by model", () => {
    const rt = runtime({
      "2": {
        state: { status: "running", branch: "feat/build", attempt: 2 },
        attempts: [
          attempt({ k: 1, meta: { engine: "codex", model: "gpt-5.6-sol", engineExit: 0 } }),
          attempt({ k: 2, transcriptMtime: now - 1000 }), // in flight
        ],
      },
    });
    const g = buildGraph(parsePlan(SINGLE_LANE), rt, { now });
    const n = node(g, "2");
    expect(n.status).toBe("running");
    expect(n.runtime?.branch).toBe("feat/build");
    expect(n.runtime?.attempt).toBe(2);
    expect(n.runtime?.model).toBe("gpt-5.6-sol"); // last ended leg
    expect(n.pulse).toBe("live"); // fresh in-flight transcript
  });

  it("flatlines a running node whose in-flight transcript has gone stale", () => {
    const rt = runtime({
      "2": {
        state: { status: "running" },
        attempts: [attempt({ k: 1, transcriptMtime: now - 26 * 60 * 1000 })],
      },
    });
    const g = buildGraph(parsePlan(SINGLE_LANE), rt, { now });
    expect(node(g, "2").pulse).toBe("flatline");
    expect(node(g, "2").ui).toBe("running");
  });

  it("raises ui='problem' for a crashed latest attempt even if state still says running (skew)", () => {
    const rt = runtime({
      "2": {
        state: { status: "running" }, // stale skew — never an error
        attempts: [attempt({ k: 1, meta: { engineExit: 0 }, status: null })], // crash
      },
    });
    const g = buildGraph(parsePlan(SINGLE_LANE), rt, { now });
    const n = node(g, "2");
    expect(n.runtime?.problem).toBe("crash");
    expect(n.ui).toBe("problem");
    expect(n.pulse).toBeNull(); // ended attempt ⇒ no heartbeat
  });

  it("raises ui='awaiting' when a HIL request is open", () => {
    const rt = runtime({
      "2": {
        state: { status: "blocked" },
        hil: { open: true, markdown: "## Need a decision\nA or B?" },
      },
    });
    const g = buildGraph(parsePlan(SINGLE_LANE), rt, { now });
    const n = node(g, "2");
    expect(n.ui).toBe("awaiting");
    expect(n.runtime?.hilOpen).toBe(true);
  });

  it("raises ui='awaiting' when the latest attempt asked a question", () => {
    const rt = runtime({
      "2": { attempts: [attempt({ k: 1, meta: { engineExit: 0 }, status: { outcome: "question" } })] },
    });
    const n = node(buildGraph(parsePlan(SINGLE_LANE), rt, { now }), "2");
    expect(n.runtime?.awaiting).toBe(true);
    expect(n.ui).toBe("awaiting");
  });

  it("shows a pending note until the phase reaches done or merged", () => {
    const pending = runtime({ "2": { state: { status: "running" }, note: "rebase first" } });
    expect(node(buildGraph(parsePlan(SINGLE_LANE), pending, { now }), "2")).toMatchObject({
      notePending: true,
      noteMarkdown: "rebase first",
    });

    for (const status of ["done", "merged"] as const) {
      const complete = runtime({ "2": { state: { status }, note: "stale file" } });
      expect(node(buildGraph(parsePlan(SINGLE_LANE), complete, { now }), "2").notePending).toBe(false);
    }
  });
});

describe("buildGraph — PR Review node", () => {
  it("shows PR Review as done when the review OUTCOME is done", () => {
    const g = buildGraph(parsePlan(SINGLE_LANE), null, {
      pr: pr({ url: "https://gh/pr/1", outcome: "done", verdict: "approved", reviewPresent: true }),
    });
    expect(node(g, "pr-review").ui).toBe("done");
  });
  it("does NOT flag a passing review whose prose contains 'changes'/'blocking'", () => {
    // The verdict is display-only prose; the terminal must key off the structured outcome.
    const g = buildGraph(parsePlan(SINGLE_LANE), null, {
      pr: pr({
        url: "https://gh/pr/1",
        outcome: "done",
        verdict: "Reviewed the changes; no blocking issues. Approving.",
        reviewPresent: true,
      }),
    });
    expect(node(g, "pr-review").ui).toBe("done");
  });
  it("flags PR Review as a problem when the review outcome is blocked/question", () => {
    const g = buildGraph(parsePlan(SINGLE_LANE), null, {
      pr: pr({ url: "https://gh/pr/1", outcome: "blocked", verdict: "needs work", reviewPresent: true }),
    });
    expect(node(g, "pr-review").ui).toBe("problem");
  });
  it("shows PR Review as running when a PR exists but no review yet", () => {
    const g = buildGraph(parsePlan(SINGLE_LANE), null, {
      pr: pr({ url: "https://gh/pr/1", outcome: null, verdict: null, reviewPresent: false }),
    });
    expect(node(g, "pr-review").ui).toBe("running");
  });
  it("shows PR Review as todo with no PR at all", () => {
    expect(node(buildGraph(parsePlan(SINGLE_LANE), null), "pr-review").ui).toBe("todo");
  });
  it("shows a review note only until review finishes", () => {
    const rt = runtime({});
    rt.reviewNote = "rebase integration first";
    const pending = node(buildGraph(parsePlan(SINGLE_LANE), rt), "pr-review");
    expect(pending.notePending).toBe(true);
    expect(pending.noteMarkdown).toBe("rebase integration first");

    const running = node(buildGraph(parsePlan(SINGLE_LANE), rt, {
      pr: pr({ url: "https://gh/pr/1", outcome: null, reviewPresent: true }),
    }), "pr-review");
    expect(running.notePending).toBe(true);

    const finished = node(buildGraph(parsePlan(SINGLE_LANE), rt, {
      pr: pr({ outcome: "done", reviewPresent: true }),
    }), "pr-review");
    expect(finished.notePending).toBe(false);
  });
});
