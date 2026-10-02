import { describe, it, expect } from "vitest";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { planReviewRuns, reviewStageRuns } from "./review-stages.ts";
import type { ReviewRun } from "./types.ts";

const LOOP_REVIEW = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../loop-review.sh");

function run(over: Partial<ReviewRun> & { k: number }): ReviewRun {
  return { runDir: `runs/review-p7-acme--api-a${over.k}`, status: null, phase: "7", repository: "acme--api", ...over };
}

describe("planReviewRuns", () => {
  it.each([
    ["shallow", 1], ["shallow", 2], ["shallow", 3],
    ["medium", 1], ["medium", 3],
    ["max", 2],
  ] as const)("matches loop-review.sh stages for %s ×%i", (tier, rounds) => {
    const tsv = execFileSync("zsh", [LOOP_REVIEW, "stages", "--tier", tier, "--rounds", String(rounds), "--models-conf", "/dev/null"], { encoding: "utf8" });
    const expected = tsv.trim().split("\n").map((line) => {
      const [k, stage, chain] = line.split("\t");
      return { k: Number(k.slice(1)), stage, chain };
    });
    expect(planReviewRuns(tier, rounds)).toEqual(expected);
  });

  it("falls back to the tier's default round count", () => {
    expect(planReviewRuns("medium", null)).toHaveLength(9);
    expect(planReviewRuns("shallow", null).map((r) => r.stage)).toEqual(["final"]);
  });
});

describe("reviewStageRuns", () => {
  const config = { tier: "shallow" as const, rounds: 2 };

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
