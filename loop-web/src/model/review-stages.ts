// PR-review pipeline sub-stages for an explicit review phase: the run list loop-review.sh
// generates from tier + rounds, folded with the phase's `runs/review-p<N>-<owner--repo>-a<k>`
// dirs so every run carries its live state. planReviewRuns mirrors `loop-review.sh stages`
// (a parity test keeps them in sync).
import type { ReviewChain, ReviewConfig, ReviewRun, ReviewRunState, ReviewStageRun, ReviewTier } from "./types.ts";

export interface PlannedReviewRun {
  k: number;
  stage: string;
  chain: ReviewChain;
}

/** LOOP_REVIEW_ROUNDS_<TIER> defaults in loop-models.conf. */
export const DEFAULT_REVIEW_ROUNDS: Record<ReviewTier, number> = { shallow: 1, medium: 3, max: 3 };

/** Every run of one repository's pipeline, planned runs first, each folded with its run dir.
 * Repositories come from the plan, except a lone `primary` (single-repo plans), which yields to
 * whatever repository keys this phase's run dirs actually carry. */
export function reviewStageRuns(
  config: ReviewConfig,
  phase: string,
  planRepositories: string[],
  runsByRepository: Record<string, ReviewRun[]>,
): ReviewStageRun[] {
  const planned = planReviewRuns(config.tier, config.rounds);
  const ranKeys = Object.keys(runsByRepository).filter((key) => runsByRepository[key].some((r) => r.phase === phase));
  const keys = planRepositories.length === 1 && planRepositories[0] === "primary" && ranKeys.length
    ? ranKeys
    : planRepositories.map((slug) => slug.replace("/", "--"));
  return keys.flatMap((key) => {
    const runs = new Map((runsByRepository[key] ?? []).filter((r) => r.phase === phase).map((r) => [r.k, r]));
    return planned.map((p) => foldRun(key.replace("--", "/"), p, runs.get(p.k)));
  });
}

/** loop-review.sh stages: medium/max run the adversary pair each round; shallow runs one
 * reviewer and its last round is `final` alone. Every round but the last is followed by a fix. */
export function planReviewRuns(tier: ReviewTier, rounds: number | null): PlannedReviewRun[] {
  const total = rounds ?? DEFAULT_REVIEW_ROUNDS[tier];
  const out: PlannedReviewRun[] = [];
  const add = (stage: string, chain: ReviewChain) => out.push({ k: out.length + 1, stage, chain });
  for (let r = 1; r <= total; r++) {
    if (tier === "shallow") {
      if (r === total) break;
      add(`round${r}`, "review-adv-a");
    } else {
      add(`round${r}`, "review-adv-a");
      add(`round${r}`, "review-adv-b");
    }
    if (r < total) add(`fix${r}`, "review-fix");
  }
  add("final", "review-final");
  return out;
}

function foldRun(repository: string, planned: PlannedReviewRun, run: ReviewRun | undefined): ReviewStageRun {
  return {
    repository,
    ...planned,
    state: run ? runState(run) : "todo",
    engine: run?.meta?.engine || run?.leg?.engine || null,
    model: run?.meta?.model || run?.leg?.model || null,
    summary: run?.status?.summary ?? null,
  };
}

/** A valid status.json outcome wins; otherwise ended (meta.json) without one = failed, and a
 * run dir with neither is still in flight. */
function runState(run: ReviewRun): ReviewRunState {
  const outcome = run.status?.outcome;
  if (outcome === "done" || outcome === "question" || outcome === "blocked") return outcome;
  return run.meta ? "failed" : "running";
}
