import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { planReviewRuns, reviewStageRuns } from "./review-stages.ts";
import { parseLoop } from "./parse-loop.ts";
import type { ReviewRun } from "./types.ts";

const LEGACY_STAGES = readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), "legacy-stages.tsv"), "utf8");

function pinned(config: object) {
  return parseLoop({ present: true, state: null, events: null, runs: [], hil: [], notes: [],
    reviewConfigs: [{ name: "review-p7-acme--api.ledger.config.json", content: JSON.stringify(config) }],
  }).reviewConfigs!;
}

function ledger(...lines: string[]) {
  return parseLoop({ present: true, state: null, events: null, runs: [], hil: [], notes: [],
    reviewLedgers: [{ name: "review-p7-acme--api.ledger.tsv", content: lines.join("\n") }],
  }).reviewLedger!;
}

describe("AR reviewStageRuns", () => {
  const config = { tier: "standard" as const, rounds: 2, inner: 2 };

  it("plans the maximum remaining inner exchanges with rotating providers", () => {
    const out = reviewStageRuns(config, "7", ["acme/api"], {});
    expect(out.map((r) => [r.stage, r.role, r.provider])).toEqual([
      ["round1", "review", "gpt"], ["round1", "critic", "claude"], ["round1", "confirm", "grok"],
      ["round1", "respond", "gpt"], ["round1", "critic", "grok"], ["disputed", "disputed", "-"], ["fix1", "fix", "claude"],
      ["round2", "review", "claude"], ["round2", "critic", "grok"], ["round2", "confirm", "gpt"],
      ["round2", "respond", "claude"], ["round2", "critic", "gpt"], ["disputed", "disputed", "-"], ["final", "final", "claude"],
    ]);
    expect(out.every((r) => r.state === "planned" && r.verdict === null)).toBe(true);
  });

  it("infers skipped rounds after clean confirm while the actual final a4 is running", () => {
    const rows = ledger(
      "a1\tround1\treview\tgpt\tAPPROVE\truns/review-p7-acme--api-a1",
      "a2\tround1\tcritic\tclaude\tAGREE\truns/review-p7-acme--api-a2",
      "a3\tround1\tconfirm\tgrok\tAGREE\truns/review-p7-acme--api-a3",
    );
    const out = reviewStageRuns(config, "7", ["acme/api"], {
      "acme--api": [run({ k: 4, leg: { engine: "claude", model: "opus" } })],
    }, rows);
    expect(out.slice(0, 3).map((r) => [r.role, r.provider, r.verdict, r.state])).toEqual([
      ["review", "gpt", "APPROVE", "done"], ["critic", "claude", "AGREE", "done"], ["confirm", "grok", "AGREE", "done"],
    ]);
    expect(out.filter((r) => r.stage === "round2").every((r) => r.state === "skipped")).toBe(true);
    expect(out.filter((r) => r.stage === "round2")).not.toHaveLength(0);
    expect(out.filter((r) => r.state === "running")).toMatchObject([
      { k: 4, stage: "final", role: "final", provider: "claude", model: "opus" },
    ]);
    expect(out.at(-1)).toMatchObject({ k: 4, stage: "final", state: "running" });
    expect(out.some((r) => r.state === "planned")).toBe(false);
    expect(new Set(out.map((r) => r.k)).size).toBe(out.length);
  });

  it("keeps skipped rounds before a completed final without inventing more work", () => {
    const rows = ledger(
      "a1\tround1\treview\tgpt\tAPPROVE\truns/review-p7-acme--api-a1",
      "a2\tround1\tcritic\tclaude\tAGREE\truns/review-p7-acme--api-a2",
      "a3\tround1\tconfirm\tgrok\tAGREE\truns/review-p7-acme--api-a3",
      "a4\tfinal\tfinal\tclaude\tAPPROVE\truns/review-p7-acme--api-a4",
    );
    const out = reviewStageRuns(config, "7", ["acme/api"], {}, rows);
    expect(out.at(-1)).toMatchObject({ k: 4, role: "final", verdict: "APPROVE", state: "done" });
    expect(out.every((r) => r.state === "done" || r.state === "skipped")).toBe(true);
  });

  it("folds a disputed marker as completed and schedules remediation, never clean skips", () => {
    const rows = ledger(
      "a1\tround1\treview\tgpt\tAPPROVE\truns/review-p7-acme--api-a1",
      "a2\tround1\tcritic\tclaude\tDISAGREE_CONCERN\truns/review-p7-acme--api-a2",
      "a3\tround1\trespond\tgpt\tAPPROVE\truns/review-p7-acme--api-a3",
      "a4\tround1\tcritic\tclaude\tDISAGREE_EVIDENCE\truns/review-p7-acme--api-a4",
      "a5\tdisputed\tdisputed\t-\tDISPUTED\truns",
    );
    const out = reviewStageRuns(config, "7", ["acme/api"], {
      "acme--api": [run({ k: 6, leg: { engine: "claude", model: "opus" } })],
    }, rows);
    expect(out[4]).toMatchObject({ k: 5, stage: "disputed", role: "disputed", provider: "-", verdict: "DISPUTED", state: "done", engine: null, runDir: "runs" });
    expect(out[5]).toMatchObject({ k: 6, stage: "fix1", role: "fix", state: "running" });
    expect(out[6]).toMatchObject({ stage: "round2", role: "review", provider: "claude", state: "planned" });
    expect(out.some((r) => r.state === "skipped")).toBe(false);
  });
});

describe("reviewStageRuns protocol format", () => {
  it("renders a historical max review without a ledger or pinned config in its original shape", () => {
    const done = Array.from({ length: 12 }, (_, i) => run({ k: i + 1, meta: { engineExit: 0 }, status: { outcome: "done" } }));
    const out = reviewStageRuns({ tier: "max", rounds: 3, inner: 2 }, "7", ["acme/api"], { "acme--api": done });
    expect(out.map((r) => [r.k, r.stage, r.chain, r.state])).toEqual(
      planReviewRuns("max", 3).map((p) => [p.k, p.stage, p.chain, "done"]));
    expect(out.some((r) => r.role)).toBe(false);
  });

  it("renders a started pipeline from its pinned limits, not the plan's omitted defaults", () => {
    const rows = ledger(
      "a1\tround1\treview\tgpt\tNEEDS_CHANGES\truns/review-p7-acme--api-a1",
      "a2\tround1\tcritic\tclaude\tAGREE\truns/review-p7-acme--api-a2",
      "a3\tfix1\tfix\tclaude\tPUSHED\truns/review-p7-acme--api-a3",
      "a4\tround2\treview\tclaude\tNEEDS_CHANGES\truns/review-p7-acme--api-a4",
      "a5\tround2\tcritic\tgrok\tAGREE\truns/review-p7-acme--api-a5",
    );
    const out = reviewStageRuns({ tier: "standard", rounds: null, inner: null }, "7", ["acme/api"], {}, rows,
      pinned({ tier: "standard", rounds: 3, inner: 2 }));
    expect(out.find((r) => r.k === 6)).toMatchObject({ stage: "fix2", role: "fix" });
  });

  it("treats a pinned config as AR even before its first ledger row", () => {
    const out = reviewStageRuns({ tier: "max", rounds: null, inner: null }, "7", ["acme/api"],
      { "acme--api": [run({ k: 1, leg: { engine: "codex", model: "gpt-6-astra" } })] }, [], pinned({ tier: "max", rounds: 1, inner: 1 }));
    expect(out[0]).toMatchObject({ k: 1, role: "review", provider: "gpt", state: "running" });
    expect(out.at(-1)).toMatchObject({ role: "final" });
  });
});

function run(over: Partial<ReviewRun> & { k: number }): ReviewRun {
  return { runDir: `runs/review-p7-acme--api-a${over.k}`, status: null, phase: "7", repository: "acme--api", ...over };
}

describe("planReviewRuns", () => {
  it.each([
    ["shallow", 1], ["shallow", 2], ["shallow", 3],
    ["medium", 1], ["medium", 3],
    ["max", 2],
  ] as const)("matches legacy stages fixture for %s ×%i", (tier, rounds) => {
    const expected = LEGACY_STAGES.trim().split("\n")
      .filter((line) => line.startsWith(`${tier}\t${rounds}\t`)).map((line) => {
      const [, , k, stage, chain] = line.split("\t");
      return { k: Number(k.slice(1)), stage, chain };
    });
    expect(planReviewRuns(tier, rounds)).toEqual(expected);
  });

  it("falls back to the tier's default round count", () => {
    expect(planReviewRuns("medium", null)).toHaveLength(12);
    expect(planReviewRuns("shallow", null).map((r) => r.stage)).toEqual(["final"]);
  });
});

describe("reviewStageRuns", () => {
  const config = { tier: "shallow" as const, rounds: 2, inner: 2 };

  it("lists every planned run as todo before anything ran", () => {
    expect(reviewStageRuns(config, "7", ["acme/api"], {})).toEqual([
      { repository: "acme/api", k: 1, stage: "round1", chain: "review-adv-a", state: "todo", engine: null, model: null, summary: null },
      { repository: "acme/api", k: 2, stage: "fix1", chain: "review-fix", state: "todo", engine: null, model: null, summary: null },
      { repository: "acme/api", k: 3, stage: "final", chain: "review-final", state: "todo", engine: null, model: null, summary: null },
    ]);
  });

  it("folds run dirs of this phase into done / running / failed", () => {
    const runs = {
      "acme--api": [
        run({ k: 1, meta: { engine: "claude", model: "opus", engineExit: 0 }, status: { outcome: "done", summary: "2 majors" } }),
        run({ k: 2, leg: { engine: "claude", model: "sonnet" } }),
        run({ k: 1, phase: "3", status: { outcome: "blocked" } }),
      ],
    };
    const out = reviewStageRuns(config, "7", ["acme/api"], runs);
    expect(out.map((r) => [r.stage, r.state, r.engine, r.model, r.summary])).toEqual([
      ["round1", "done", "claude", "opus", "2 majors"],
      ["fix1", "running", "claude", "sonnet", null],
      ["final", "todo", null, null, null],
    ]);
    const crashed = reviewStageRuns(config, "7", ["acme/api"], { "acme--api": [run({ k: 1, meta: { engineExit: 0 } })] });
    expect(crashed[0].state).toBe("failed");
    const asked = reviewStageRuns(config, "7", ["acme/api"], { "acme--api": [run({ k: 1, meta: {}, status: { outcome: "question" } })] });
    expect(asked[0].state).toBe("question");
  });

  it("uses the repositories that actually ran when the plan only knows `primary`", () => {
    const runs = { "acme--api": [run({ k: 1, status: { outcome: "done" }, meta: {} })] };
    const out = reviewStageRuns(config, "7", ["primary"], runs);
    expect(new Set(out.map((r) => r.repository))).toEqual(new Set(["acme/api"]));
  });
});
