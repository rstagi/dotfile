import { describe, it, expect } from "vitest";
import { parseLoop } from "./parse-loop.ts";
import type { LoopInput, RawRunDir } from "./parse-loop.ts";

function run(over: Partial<RawRunDir> & { name: string }): RawRunDir {
  return {
    meta: null,
    status: null,
    spawnLog: null,
    transcriptMtime: null,
    metaMtime: null,
    ...over,
  };
}

const STATE = JSON.stringify({
  runId: "loop-widget-2026-08-01",
  integrationBranch: "feat/widget-revamp",
  prUrl: null,
  phases: {
    "1": { slug: "widget-core", lane: "A", branch: "feat/widget-core", status: "merged", attempt: 1 },
    "2": { slug: "build-feature", lane: "A", branch: "feat/build", status: "running", attempt: 2 },
  },
});

function input(over: Partial<LoopInput> = {}): LoopInput {
  return {
    present: true,
    state: STATE,
    events: null,
    runs: [],
    hil: [],
    notes: [],
    ...over,
  };
}

describe("parseLoop — presence & defensiveness", () => {
  it("returns an empty runtime when the loop dir is absent", () => {
    const rt = parseLoop(input({ present: false }));
    expect(rt.present).toBe(false);
    expect(rt.phases).toEqual({});
    expect(rt.events).toEqual([]);
  });
  it("does not throw on malformed state.json — state becomes null", () => {
    const rt = parseLoop(input({ state: "{not json" }));
    expect(rt.state).toBeNull();
    expect(rt.phases).toEqual({}); // no mapping without state
  });
  it("survives an entirely empty input", () => {
    expect(() => parseLoop(input({ state: null }))).not.toThrow();
  });
});

describe("parseLoop — state & phase mapping", () => {
  it("parses state.json and creates a phase runtime per state phase", () => {
    const rt = parseLoop(input());
    expect(rt.state?.integrationBranch).toBe("feat/widget-revamp");
    expect(Object.keys(rt.phases).sort()).toEqual(["1", "2"]);
    expect(rt.phases["2"].state?.status).toBe("running");
  });

  it("groups run dirs to their phase by the slug prefix of runs/<slug>-a<K>", () => {
    const rt = parseLoop(
      input({
        runs: [
          run({ name: "widget-core-a1", meta: JSON.stringify({ engine: "codex", model: "gpt-5.6-sol", engineExit: 0 }), status: JSON.stringify({ outcome: "done" }) }),
          run({ name: "build-feature-a1", meta: JSON.stringify({ engineExit: 40 }) }),
          run({ name: "build-feature-a2", transcriptMtime: 123 }),
        ],
      }),
    );
    expect(rt.phases["1"].attempts.map((a) => a.k)).toEqual([1]);
    expect(rt.phases["2"].attempts.map((a) => a.k)).toEqual([1, 2]);
    expect(rt.phases["1"].attempts[0].meta?.model).toBe("gpt-5.6-sol");
    expect(rt.phases["2"].attempts[1].transcriptMtime).toBe(123);
  });

  it("separates review runs (runs/review-a<K>) from phase attempts", () => {
    const rt = parseLoop(
      input({
        runs: [
          run({ name: "review-a1", status: JSON.stringify({ outcome: "done", summary: "LGTM" }) }),
          run({ name: "widget-core-a1" }),
        ],
      }),
    );
    expect(rt.reviewRuns.map((r) => r.k)).toEqual([1]);
    expect(rt.reviewRuns[0].status?.summary).toBe("LGTM");
    // review dir must NOT leak into a phase
    expect(Object.values(rt.phases).every((p) => p.attempts.every((a) => !a.runDir.includes("review")))).toBe(true);
  });

  it("groups repository review runs independently", () => {
    const rt = parseLoop(input({ runs: [
      run({ name: "review-acme--api-a1", status: JSON.stringify({ outcome: "done" }) }),
      run({ name: "review-acme--web-a2", status: JSON.stringify({ outcome: "blocked" }) }),
    ] }));
    expect(rt.reviewRunsByRepository["acme--api"][0].status?.outcome).toBe("done");
    expect(rt.reviewRunsByRepository["acme--web"][0].k).toBe(2);
    expect(rt.reviewRuns).toEqual([]);
  });
});

describe("parseLoop — events.jsonl", () => {
  it("parses one event per line and skips malformed lines", () => {
    const events = [
      JSON.stringify({ ts: "T1", event: "spawned", phase: "2", detail: "" }),
      "{ broken",
      "",
      JSON.stringify({ ts: "T2", event: "merged", phase: "1" }),
    ].join("\n");
    const rt = parseLoop(input({ events }));
    expect(rt.events.map((e) => e.event)).toEqual(["spawned", "merged"]);
  });
});

describe("parseLoop — HIL", () => {
  it("marks a HIL open when the slug has no answer, and maps it to its phase", () => {
    const rt = parseLoop(
      input({ hil: [{ slug: "build-feature", markdown: "## Decide\nA or B?", answered: false }] }),
    );
    expect(rt.phases["2"].hil).toEqual({ open: true, markdown: "## Decide\nA or B?" });
  });
  it("marks a HIL closed once the answer file exists", () => {
    const rt = parseLoop(
      input({ hil: [{ slug: "build-feature", markdown: "q", answered: true }] }),
    );
    expect(rt.phases["2"].hil?.open).toBe(false);
  });
});

describe("parseLoop — steering notes", () => {
  it("routes numeric notes to their phase and the reserved review note separately", () => {
    const rt = parseLoop(
      input({
        notes: [
          { key: "2", markdown: "rebase before merge" },
          { key: "pr-review", markdown: "check the migration" },
        ],
      } as Partial<LoopInput>),
    );
    expect(rt.phases["2"].note).toBe("rebase before merge");
    expect(rt.reviewNote).toBe("check the migration");
  });

  it("creates a phase runtime for a numeric note even before state appears", () => {
    const rt = parseLoop(
      input({ state: null, notes: [{ key: "7", markdown: "start here" }] } as Partial<LoopInput>),
    );
    expect(rt.phases["7"].note).toBe("start here");
  });

  it("routes repository review notes independently", () => {
    const rt = parseLoop(input({ notes: [
      { key: "pr-review.acme--api", markdown: "focus API" },
      { key: "pr-review.acme--web", markdown: "focus UI" },
    ] }));
    expect(rt.reviewNotes).toEqual({ "acme--api": "focus API", "acme--web": "focus UI" });
    expect(rt.reviewNote).toBeNull();
  });
});
