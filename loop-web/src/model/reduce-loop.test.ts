import { describe, it, expect } from "vitest";
import { emptyRecord, reduceLoop, effectivePhaseStatus } from "./reduce-loop.ts";
import { summarize } from "./materialize.ts";
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

const REVIEWED_PLAN = `# Editable — Multi-Phase Plan
## Phases
### Phase 1 — Initial work \`[lane: A]\` \`[status: done]\`
- **Depends on:** none
### Phase 2 — First review \`[lane: review]\` \`[status: done]\` \`[kind: pr-review]\`
- **Depends on:** Phase 1
`;

const APPENDED_PLAN = `${REVIEWED_PLAN}
### Phase 3 — Follow-up work \`[lane: A]\` \`[status: todo]\`
- **Depends on:** Phase 2
### Phase 4 — Final review \`[lane: review]\` \`[status: todo]\` \`[kind: pr-review]\`
- **Depends on:** Phase 3
`;

const MILESTONE_PLAN = `# Milestones — Multi-Phase Plan
## Loop config
- **Integration branch:** \`feat/global\`
## Repositories
### \`acme/api\`
- **PR:** https://github.com/acme/api/pull/99
### \`acme/web\`
## Milestone 1 — First release
- **Integration branch:** \`feat/first\`
- **PR (acme/api):** https://github.com/acme/api/pull/1
### Phase 1 — API \`[lane: A]\` \`[status: todo]\`
- **Repository:** \`acme/api\`
- **Depends on:** none
### Phase 2 — Web \`[lane: B]\` \`[status: todo]\`
- **Repository:** \`acme/web\`
- **Depends on:** none
### Phase 3 — Gate \`[lane: review]\` \`[status: todo]\` \`[kind: pr-review]\` \`[gate: merge]\`
- **Depends on:** Phase 1, Phase 2
## Milestone 2 — Second release
- **Integration branch:** \`feat/second\`
### Phase 4 — API follow-up \`[lane: A]\` \`[status: todo]\`
- **Repository:** \`acme/api\`
- **Depends on:** Phase 3
### Phase 5 — Final review \`[lane: review]\` \`[status: todo]\` \`[kind: pr-review]\`
- **Depends on:** Phase 4
`;

describe("reduceLoop — register", () => {
  it("seeds identity + planText and marks a register-only loop planned", () => {
    const rec = fold("run-1", {
      kind: "register",
      info: {
        runId: "run-1",
        effort: "Checkout revamp",
        loopDir: "/repo/.loop",
        planFile: "/repo/.loop/plan.md",
        integrationBranch: "feat/checkout",
        startedAt: "2026-08-03T09:00:00Z",
        planText: "# Checkout revamp — Multi-Phase Plan\n",
      },
    });
    expect(rec.runId).toBe("run-1");
    expect(rec.effort).toBe("Checkout revamp");
    expect(rec.integrationBranch).toBe("feat/checkout");
    expect(rec.planText).toMatch(/Checkout revamp/);
    // register-only ⇒ planned (plan on the daemon, no run yet)
    expect(rec.status).toBe("planned");
    expect(rec.startedAt).toBe("2026-08-03T09:00:00Z");
  });

  it("reopens a finished loop when an unfinished phase is appended", () => {
    const finished = fold(
      "r",
      { kind: "register", info: { runId: "r", planText: REVIEWED_PLAN } },
      stateWith({ "1": { status: "merged" }, "2": { status: "done" } }),
      { kind: "finish", info: { finishedAt: "2026-08-03T11:00:00Z" } },
    );
    const reopened = reduceLoop(finished, {
      kind: "register",
      info: { runId: "r", planText: APPENDED_PLAN },
    });
    expect(reopened.status).toBe("active");
    expect(reopened.finishedAt).toBeNull();
    expect(effectivePhaseStatus(reopened, "1")).toBe("merged");
    expect(effectivePhaseStatus(reopened, "2")).toBe("done");
    expect(effectivePhaseStatus(reopened, "3")).toBe("todo");
    expect(effectivePhaseStatus(reopened, "4")).toBe("todo");
    expect(summarize(reopened).phaseCounts).toEqual({
      total: 4,
      todo: 2,
      running: 0,
      blocked: 0,
      done: 1,
      merged: 1,
    });
  });
});

describe("reduceLoop — planned → active", () => {
  it("a register-only record with no state and no events is planned", () => {
    const rec = fold("r", { kind: "register", info: { runId: "r", effort: "Demo" } });
    expect(rec.status).toBe("planned");
  });

  it("the first state push flips planned → active", () => {
    const rec = fold(
      "r",
      { kind: "register", info: { runId: "r", effort: "Demo" } },
      stateWith({ "1": { slug: "pg", status: "todo" } }),
    );
    expect(rec.status).toBe("active");
  });

  it("the first lifecycle event flips planned → active", () => {
    const rec = fold(
      "r",
      { kind: "register", info: { runId: "r" } },
      { kind: "event", event: { event: "phase.attempt.start", phase: "1" } },
    );
    expect(rec.status).toBe("active");
  });

  it("keeps a pre-attempt Jev decision observational and idempotent", () => {
    const decision: Ingest = {
      kind: "event",
      event: {
        event: "jev.decision",
        phase: "1",
        attempt: 1,
        stage: "route",
        mode: "shadow",
        candidate: "light",
        confidence: 0.91,
        probabilities: { light: 0.91, default: 0.09 },
        appliedAction: "default",
        fallbackReason: "shadow-mode",
        resolvedModel: "typesafe-systemone",
        evidenceChecked: true,
        evidenceSources: ["question", "plan"],
        questionRound: 2,
        ts: "2026-09-27T10:00:00Z",
      },
    };
    const once = fold("r", { kind: "register", info: { runId: "r" } }, decision);
    const twice = reduceLoop(once, decision);

    expect(twice.decisions).toEqual([
      expect.objectContaining({ runId: "r", phase: "1", attempt: 1, stage: "route",
        evidenceChecked: true, evidenceSources: ["question", "plan"], questionRound: 2 }),
    ]);
    expect(twice.status).toBe("planned");
    expect(effectivePhaseStatus(twice, "1")).toBe("todo");
  });

  it("preserves typed Jev fields through idempotent JSONL replay", () => {
    const jsonl = JSON.stringify({
      event: "jev.decision", runId: "r", phase: "2", attempt: 2, stage: "merge-risk",
      mode: "active", candidate: "high", confidence: 0.73, probabilities: { high: 0.73 },
      appliedAction: "full-review", fallbackReason: null, resolvedModel: "systemone",
      ts: "2026-09-27T10:00:00Z",
    });
    const once = fold("r", { kind: "eventsFile", text: jsonl });
    const twice = reduceLoop(once, { kind: "eventsFile", text: jsonl });
    expect(twice.decisions).toEqual([expect.objectContaining({
      runId: "r", phase: "2", attempt: 2, stage: "merge-risk", candidate: "high",
      appliedAction: "full-review", confidence: 0.73,
    })]);
  });

  it("retains separate merge-risk decisions for changed heads with the same timestamp", () => {
    const first = {
      event: "jev.decision", phase: "2", attempt: 2, stage: "merge-risk", mode: "active" as const,
      appliedAction: "full-diff-skim", head: "abc123", ts: "2026-09-27T10:00:00Z",
    };
    const rec = fold("r",
      { kind: "event", event: first },
      { kind: "event", event: { ...first, head: "def456" } },
    );

    expect(rec.decisions.map((decision) => decision.head)).toEqual(["abc123", "def456"]);
    expect(rec.events.map((event) => event.head)).toEqual(["abc123", "def456"]);
  });

  it("keeps the latest update time while accepting delayed events after finish", () => {
    const decision: Ingest = {
      kind: "event",
      event: {
        event: "jev.decision", phase: "2", attempt: 1, stage: "merge-risk", mode: "active",
        head: "abc123", ts: "2026-09-27T10:00:00Z",
      },
    };
    const finished = fold("r", decision,
      { kind: "event", event: { event: "phase.attempt.finish", phase: "2", outcome: "done",
        exitCode: 0, ts: "2026-09-27T10:05:00Z" } },
      { kind: "finish", info: { finishedAt: "2026-09-27T10:10:00Z" } },
    );
    const replayed = reduceLoop(finished, decision);
    const delayed = reduceLoop(replayed, {
      kind: "event",
      event: { event: "review.finish", phase: "2", ts: "2026-09-27T10:02:00Z" },
    });

    expect(replayed.updatedAt).toBe("2026-09-27T10:10:00Z");
    expect(replayed.decisions).toHaveLength(1);
    expect(delayed.updatedAt).toBe("2026-09-27T10:10:00Z");
    expect(delayed.events.map((event) => event.event)).toContain("review.finish");
  });

  it("compares update times by instant when timestamps have different precision", () => {
    const recent = fold("r", {
      kind: "event", event: { event: "phase.attempt.finish", phase: "2",
        ts: "2026-09-27T10:00:00.001Z" },
    });
    const backfilled = reduceLoop(recent, {
      kind: "event", event: { event: "jev.decision", phase: "2", attempt: 1,
        stage: "route", mode: "active", ts: "2026-09-27T10:00:00Z" },
    });

    expect(backfilled.updatedAt).toBe("2026-09-27T10:00:00.001Z");
  });

  it("sorts out-of-order decisions chronologically with a deterministic timestamp tie", () => {
    const event = { event: "jev.decision", phase: "2", attempt: 1,
      mode: "active" as const };
    const rec = fold("r",
      { kind: "event", event: { ...event, stage: "route", ts: "2026-09-27T10:00:00.001Z" } },
      { kind: "event", event: { ...event, stage: "question", ts: "2026-09-27T10:00:00Z" } },
      { kind: "event", event: { ...event, stage: "merge-risk", head: "abc123",
        ts: "2026-09-27T10:00:00Z" } },
    );

    expect(rec.decisions.map((decision) => decision.stage)).toEqual(["merge-risk", "question", "route"]);
  });
});

describe("reduceLoop — promotion lattice (the staleness fix)", () => {
  it("keeps an event-only running promotion over a static todo plan marker", () => {
    const rec = fold(
      "r",
      { kind: "register", info: { runId: "r", planText: APPENDED_PLAN } },
      { kind: "event", event: { event: "phase.attempt.start", phase: "3" } },
    );
    expect(effectivePhaseStatus(rec, "3")).toBe("running");
  });

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

  it("keeps an explicit review phase running when one review-stage attempt finishes", () => {
    const rec = fold(
      "r",
      { kind: "register", info: { runId: "r", planText: APPENDED_PLAN } },
      stateWith({ "4": { kind: "pr-review", slug: "review-p4", status: "running" } }),
      {
        kind: "event",
        event: { event: "phase.attempt.finish", phase: "4", outcome: "done", exitCode: 0 },
      },
    );

    expect(rec.phases["4"].rank).toBe("running");
    expect(effectivePhaseStatus(rec, "4")).toBe("running");
    expect(rec.events).toEqual([
      expect.objectContaining({ event: "phase.attempt.finish", phase: "4" }),
    ]);
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

  it("keeps merge.verify-fail timeline-only on live ingest and JSONL replay", () => {
    const message = { event: "merge.verify-fail", phase: "2", repository: "acme/api", detail: "verify exit 1\nFAILED", ts: "2026-10-18T00:00:00Z" };
    const initial = fold("r", stateWith({ "2": { slug: "cart", status: "running" } }));
    const live = reduceLoop(initial, { kind: "event", event: message });
    const replayed = reduceLoop(live, { kind: "eventsFile", text: JSON.stringify(message) });
    expect(effectivePhaseStatus(replayed, "2")).toBe("running");
    expect(replayed.phases).toEqual(initial.phases);
    expect(replayed.events).toEqual([expect.objectContaining({ event: "merge.verify-fail", phase: "2" })]);
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

describe("reduceLoop — milestone lifecycle", () => {
  it("uses a repository-section PR only for a milestone-less plan", () => {
    const explicit = MILESTONE_PLAN.replace("- **PR (acme/api):** https://github.com/acme/api/pull/1\n", "");
    const implicit = `# Legacy
## Repositories
### \`acme/api\`
- **PR:** https://github.com/acme/api/pull/99
## Phases
### Phase 1 — Work \`[lane: A]\` \`[status: todo]\`
- **Repository:** \`acme/api\`
`;
    const milestone = fold("r", { kind: "register", info: { runId: "r", planText: explicit } });
    const legacy = fold("r", { kind: "register", info: { runId: "r", planText: implicit } });
    expect(milestone.repositories["acme/api"].prUrl).toBeNull();
    expect(legacy.repositories["acme/api"].prUrl).toBe("https://github.com/acme/api/pull/99");
  });

  it("preserves all declared repositories and register branch precedence for legacy plans", () => {
    const planText = `# Legacy
## Repositories
### \`acme/api\`
- **Integration branch:** \`feat/api\`
### \`acme/web\`
- **Integration branch:** \`feat/web\`
- **PR:** https://github.com/acme/web/pull/9
## Phases
### Phase 1 — API only \`[lane: A]\` \`[status: todo]\`
- **Repository:** \`acme/api\`
`;
    const rec = fold("r", { kind: "register", info: {
      runId: "r", planText, integrationBranch: "feat/registered",
    } });
    expect(Object.keys(rec.repositories)).toEqual(["acme/api", "acme/web"]);
    expect(rec.repositories["acme/web"]).toMatchObject({
      integrationBranch: "feat/web", prUrl: "https://github.com/acme/web/pull/9",
    });
    expect(rec.integrationBranch).toBe("feat/registered");
  });

  it.each<Ingest>([
    { kind: "finish", info: { finishedAt: "2026-10-15T00:00:00Z" } },
    { kind: "event", event: { event: "loop.finish", ts: "2026-10-15T00:00:00Z" } },
  ])("rejects $kind at an intermediate gate until the final phase is done", (finish) => {
    const gate = fold("r",
      { kind: "register", info: { runId: "r", planText: MILESTONE_PLAN } },
      stateWith({ "1": { status: "merged" }, "2": { status: "merged" }, "3": { status: "done" } }, {
        milestone: 1, awaitingMerge: { milestone: 1, prs: {} },
      }),
    );
    const rejected = reduceLoop(gate, finish);
    expect(rejected.status).toBe("awaiting-merge");
    expect(rejected.finishedAt).toBeNull();
    const final = reduceLoop(rejected, stateWith({ "4": { status: "merged" }, "5": { status: "done" } }, {
      milestone: 2, awaitingMerge: null,
    }));
    expect(reduceLoop(final, finish).status).toBe("finished");
  });

  it("reads branch/PR from the current milestone and keeps retired participants out on registration", () => {
    const initial = fold("r", { kind: "register", info: {
      runId: "r", planText: MILESTONE_PLAN, integrationBranch: "feat/global",
    } });
    expect(initial.integrationBranch).toBe("feat/first");
    expect(initial.repositories["acme/api"]).toMatchObject({
      integrationBranch: "feat/first", prUrl: "https://github.com/acme/api/pull/1",
    });

    const advanced = reduceLoop(initial, stateWith({}, {
      milestone: 2,
      repositories: { "acme/api": { integrationBranch: "feat/second", prUrl: null } },
    }));
    const registered = reduceLoop(advanced, { kind: "register", info: {
      runId: "r", planText: MILESTONE_PLAN, integrationBranch: "feat/global",
    } });
    expect(Object.keys(registered.repositories)).toEqual(["acme/api"]);
    expect(registered.repositories["acme/api"]).toMatchObject({ integrationBranch: "feat/second", prUrl: null });
    expect(registered.integrationBranch).toBe("feat/second");
    expect(registered.prUrl).toBeNull();
  });

  it("replaces current repository and scalar projections when M1 advances to M2", () => {
    const first = fold("r", stateWith({ "1": { status: "merged" }, "2": { status: "done" } }, {
      milestone: 1,
      integrationBranch: "feat/first",
      integrationWorktree: "/worktrees/first",
      prUrl: "https://github.com/acme/api/pull/1",
      review: { outcome: "done", summary: "M1 approved" },
      reviewPipeline: { phase: 2, stage: "final" },
      awaitingMerge: { milestone: 1, prs: { "acme/api": "https://github.com/acme/api/pull/1" } },
      repositories: {
        "acme/api": {
          integrationBranch: "feat/first", integrationWorktree: "/worktrees/first",
          prUrl: "https://github.com/acme/api/pull/1",
          review: { outcome: "done", summary: "M1 approved" },
          reviewPipeline: { phase: 2, stage: "final" },
        },
        "acme/web": { integrationBranch: "feat/web-first", prUrl: "https://github.com/acme/web/pull/1" },
      },
    }));
    const advanced = reduceLoop(first, stateWith({ "3": { status: "running" } }, {
      milestone: 2,
      awaitingMerge: null,
      milestones: { "1": { prs: { "acme/api": "https://github.com/acme/api/pull/1" } } },
      repositories: {
        "acme/api": {
          integrationBranch: "feat/second", integrationWorktree: "/worktrees/second",
          prUrl: null, review: null, reviewPipeline: null,
        },
      },
    }));

    expect(Object.keys(advanced.repositories)).toEqual(["acme/api"]);
    expect(advanced.repositories["acme/api"]).toMatchObject({
      integrationBranch: "feat/second", integrationWorktree: "/worktrees/second",
      prUrl: null, review: null, reviewPipeline: null,
    });
    expect(advanced).toMatchObject({
      integrationBranch: "feat/second", integrationWorktree: "/worktrees/second",
      prUrl: null, review: null, reviewPipeline: null, status: "active",
    });
    expect(advanced.lastState?.milestones?.["1"].prs["acme/api"]).toBe("https://github.com/acme/api/pull/1");
    expect(effectivePhaseStatus(advanced, "1")).toBe("merged");
    expect(effectivePhaseStatus(advanced, "2")).toBe("done");
    expect(effectivePhaseStatus(advanced, "3")).toBe("running");
    expect(first.status).toBe("awaiting-merge");
    expect(first.repositories["acme/api"].review?.summary).toBe("M1 approved");
  });

  it.each([undefined, 1])("retains legacy merge semantics without a milestone change (milestone=%s)", (milestone) => {
    const initial = fold("r", stateWith({}, {
      milestone: 1,
      integrationBranch: "feat/first", integrationWorktree: "/worktrees/first",
      prUrl: "https://github.com/acme/api/pull/1", review: { outcome: "done" },
      reviewPipeline: { phase: 2, stage: "final" },
      repositories: {
        "acme/api": {
          integrationBranch: "feat/first", integrationWorktree: "/worktrees/first",
          prUrl: "https://github.com/acme/api/pull/1", review: { outcome: "done" },
          reviewPipeline: { phase: 2, stage: "final" },
        },
      },
    }));
    const repeated = reduceLoop(initial, stateWith({}, {
      milestone, integrationBranch: null, integrationWorktree: null,
      prUrl: null, review: null, reviewPipeline: null,
      repositories: { "acme/api": { prUrl: null, review: null, reviewPipeline: null } },
    }));
    expect(repeated.repositories).toEqual(initial.repositories);
    expect(repeated).toMatchObject({
      integrationBranch: "feat/first", integrationWorktree: "/worktrees/first",
      prUrl: "https://github.com/acme/api/pull/1", review: { outcome: "done" },
      reviewPipeline: { phase: 2, stage: "final" },
    });
  });

  it("honours explicit scalar nulls on a switch even when the new repository has values", () => {
    const rec = fold("r", stateWith({}, { milestone: 1 }), stateWith({}, {
      milestone: 2,
      integrationBranch: null, integrationWorktree: null, prUrl: null, review: null, reviewPipeline: null,
      repositories: {
        "acme/api": {
          integrationBranch: "feat/second", integrationWorktree: "/worktrees/second",
          prUrl: "https://github.com/acme/api/pull/2", review: { outcome: "done" },
          reviewPipeline: { phase: 5, stage: "final" },
        },
      },
    }));
    expect(rec).toMatchObject({
      integrationBranch: null, integrationWorktree: null, prUrl: null, review: null, reviewPipeline: null,
    });
    expect(rec.repositories["acme/api"].reviewPipeline).toEqual({ phase: 5, stage: "final" });
  });

  it("clears omitted projections and participants when a new milestone has no repository state yet", () => {
    const rec = fold("r", stateWith({}, {
      milestone: 1, integrationBranch: "feat/first", integrationWorktree: "/worktrees/first",
      prUrl: "https://github.com/acme/api/pull/1", review: { outcome: "done" },
      reviewPipeline: { phase: 2, stage: "final" },
      repositories: { "acme/api": { integrationBranch: "feat/first", prUrl: "https://github.com/acme/api/pull/1" } },
    }), stateWith({}, { milestone: 2 }));
    expect(rec.repositories).toEqual({});
    expect(rec).toMatchObject({
      integrationBranch: null, integrationWorktree: null, prUrl: null, review: null, reviewPipeline: null,
    });
  });

  it.each(["milestone.awaiting-merge", "milestone.resumed"])("keeps %s observational on live ingest and JSONL replay", (event) => {
    const message = { event, phase: "2", ts: "2026-10-15T00:00:00Z" };
    const initial = fold("r", stateWith({ "2": { status: "running" } }));
    const live = reduceLoop(initial, { kind: "event", event: message });
    const replayed = reduceLoop(live, { kind: "eventsFile", text: JSON.stringify(message) });
    expect(effectivePhaseStatus(replayed, "2")).toBe("running");
    expect(replayed.phases).toEqual(initial.phases);
    expect(replayed.events).toHaveLength(1);
    expect(replayed.status).toBe("active");
  });

  it.each([
    { awaiting: false, hil: false, finished: false, expected: "active" },
    { awaiting: true, hil: false, finished: false, expected: "awaiting-merge" },
    { awaiting: false, hil: true, finished: false, expected: "paused" },
    { awaiting: true, hil: true, finished: false, expected: "paused" },
    { awaiting: false, hil: false, finished: true, expected: "finished" },
    { awaiting: true, hil: false, finished: true, expected: "finished" },
    { awaiting: false, hil: true, finished: true, expected: "finished" },
    { awaiting: true, hil: true, finished: true, expected: "finished" },
  ])("derives $expected (awaiting=$awaiting, HIL=$hil, finished=$finished)", ({ awaiting, hil, finished, expected }) => {
    let rec = fold("r", stateWith({}, {
      milestone: 1,
      awaitingMerge: awaiting ? { milestone: 1, prs: { "acme/api": "https://github.com/acme/api/pull/1" } } : null,
    }));
    if (hil) rec = reduceLoop(rec, { kind: "event", event: { event: "hil.raise", phase: "2" } });
    if (finished) rec = reduceLoop(rec, { kind: "finish", info: {} });
    // Re-derive after finish too: a later state push must preserve its precedence.
    rec = reduceLoop(rec, { kind: "state", state: rec.lastState! });
    expect(rec.status).toBe(expected);
  });

  it("resolves HIL back to the merge gate and becomes active only when state clears the gate", () => {
    const awaiting = fold("r", stateWith({}, { awaitingMerge: { milestone: 1, prs: {} } }));
    const paused = reduceLoop(awaiting, { kind: "event", event: { event: "hil.raise", phase: "2" } });
    const resolved = reduceLoop(paused, { kind: "event", event: { event: "hil.resolve", phase: "2" } });
    expect(resolved.status).toBe("awaiting-merge");
    expect(reduceLoop(resolved, stateWith({}, { awaitingMerge: null })).status).toBe("active");
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

  it("does not finish an explicit flow until its terminal PR-review phase is done", () => {
    const notReviewed = fold(
      "r",
      { kind: "register", info: { runId: "r", planText: APPENDED_PLAN } },
      stateWith({
        "1": { status: "merged" },
        "2": { status: "done" },
        "3": { status: "merged" },
        "4": { status: "todo" },
      }),
      { kind: "finish", info: { finishedAt: "2026-08-03T12:00:00Z" } },
    );
    expect(notReviewed.status).toBe("active");
    expect(notReviewed.finishedAt).toBeNull();

    const reviewed = reduceLoop(
      reduceLoop(notReviewed, stateWith({
        "1": { status: "merged" },
        "2": { status: "done" },
        "3": { status: "merged" },
        "4": { status: "done" },
      })),
      { kind: "finish", info: { finishedAt: "2026-08-03T12:05:00Z" } },
    );
    expect(reviewed.status).toBe("finished");
  });

  it("keeps the loop finished even if a stray late state push arrives after finish", () => {
    const rec = fold(
      "r",
      { kind: "finish", info: { finishedAt: "2026-08-03T11:00:00Z" } },
      stateWith({ "1": { slug: "pg", status: "running" } }),
    );
    expect(rec.status).toBe("finished");
  });

  it("folds plural repository PR/review results while retaining the primary projection", () => {
    const rec = fold(
      "r",
      {
        kind: "state",
        state: {
          repositories: {
            "acme/api": { integrationBranch: "feat/api", prUrl: "https://github.com/acme/api/pull/1" },
            "acme/web": { integrationBranch: "feat/web", prUrl: null },
          },
          phases: { "1": { repository: "acme/api", status: "running" } },
        },
      },
      {
        kind: "finish",
        info: {
          repositories: {
            "acme/api": { prUrl: "https://github.com/acme/api/pull/1", review: { outcome: "done", summary: "ok", reportPath: null, commentUrl: null } },
            "acme/web": { prUrl: "https://github.com/acme/web/pull/2", review: { outcome: "blocked", summary: "fix", reportPath: "runs/review-acme--web-a1/report.md", commentUrl: null } },
          },
        },
      },
    );
    expect(rec.repositories["acme/api"].review?.outcome).toBe("done");
    expect(rec.repositories["acme/web"].prUrl).toContain("/pull/2");
    expect(rec.phases["1"].repository).toBe("acme/api");
    expect(rec.prUrl).toBe("https://github.com/acme/api/pull/1");
    expect(rec.review?.outcome).toBe("done");
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
