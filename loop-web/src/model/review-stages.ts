// AR pipelines replay completed ledger rows, then show the maximum remaining path.
// Pre-AR pipelines (no ledger rows or pinned config, but run dirs) keep their fixed positions.
import type { ReviewChain, ReviewConfig, ReviewLedgerEntry, ReviewPipelineConfig, ReviewProvider, ReviewRole, ReviewRun, ReviewRunState, ReviewStageRun, ReviewTier } from "./types.ts";

export interface PlannedReviewRun {
  k: number;
  stage: string;
  chain: ReviewChain;
}

interface ArStep {
  stage: string;
  role: ReviewRole;
  provider: ReviewProvider;
}

export const DEFAULT_REVIEW_ROUNDS: Record<ReviewTier, number> = { standard: 2, shallow: 1, medium: 3, max: 2 };
const DEFAULT_REVIEW_INNER = 2;
const PROVIDERS = ["gpt", "claude", "grok"] as const;

/** Fold each repository separately; `primary` yields to repository keys from live data. */
export function reviewStageRuns(
  config: ReviewConfig,
  phase: string,
  planRepositories: string[],
  runsByRepository: Record<string, ReviewRun[]>,
  ledger: ReviewLedgerEntry[] = [],
  pinned: ReviewPipelineConfig[] = [],
): ReviewStageRun[] {
  const phaseLedger = ledger.filter((row) => row.phase === phase);
  const ranKeys = [...new Set([
    ...Object.keys(runsByRepository).filter((key) => runsByRepository[key].some((r) => r.phase === phase)),
    ...phaseLedger.map((row) => row.repository),
    ...pinned.filter((c) => c.phase === phase).map((c) => c.repository),
  ])];
  const keys = planRepositories.length === 1 && planRepositories[0] === "primary" && ranKeys.length
    ? ranKeys : planRepositories.map((slug) => slug.replace("/", "--"));
  return keys.flatMap((key) => {
    const runs = new Map((runsByRepository[key] ?? []).filter((r) => r.phase === phase).map((r) => [r.k, r]));
    const repository = key.replace("--", "/");
    const rows = phaseLedger.filter((row) => row.repository === key);
    const started = pinned.find((c) => c.phase === phase && c.repository === key);
    if (started) return arStageRuns(started, repository, rows, runs);
    if (config.tier === "shallow" || config.tier === "medium" || (!rows.length && runs.size)) {
      return planReviewRuns(config.tier, config.rounds).map((p) => foldRun(repository, p, runs.get(p.k)));
    }
    return arStageRuns(config, repository, rows, runs);
  });
}

/** Historical static shape, retained for fixture-backed rendering of legacy tiers. */
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
      add(`round${r}`, "review-adv-c");
    }
    if (r < total) add(`fix${r}`, "review-fix");
  }
  add("final", "review-final");
  return out;
}

function arStageRuns(config: ReviewConfig, repository: string, ledger: ReviewLedgerEntry[], runs: Map<number, ReviewRun>): ReviewStageRun[] {
  const rounds = config.rounds ?? DEFAULT_REVIEW_ROUNDS[config.tier];
  const inner = config.inner ?? DEFAULT_REVIEW_INNER;
  const out: ReviewStageRun[] = ledger.map((row) => ({
    ...foldArRun(repository, row.k, row, runs.get(row.k)),
    state: "done" as const, verdict: row.verdict, runDir: row.runDir,
  }));
  const replay = [...ledger];
  let k = Math.max(0, ...ledger.map((row) => row.k));
  let step: ArStep | null;
  while ((step = nextArStep(replay, rounds, inner))) {
    k++;
    out.push(foldArRun(repository, k, step, runs.get(k)));
    replay.push(hypotheticalRow(step, k, replay));
  }
  // A clean confirmation skips all later outer rounds. These have no actual attempt IDs;
  // negative display keys keep the live final's a<K> intact and React row keys unique.
  const clean = ledger.find((row) => row.role === "confirm" && row.verdict === "AGREE");
  if (clean) {
    const round = Number(clean.stage.slice(5));
    let skippedK = 0;
    const skipped: ReviewStageRun[] = [];
    if (round < rounds) skipped.push({
      ...foldArRun(repository, --skippedK, { stage: `fix${round}`, role: "fix", provider: "claude" }), state: "skipped",
    });
    const skippedReplay: ReviewLedgerEntry[] = [];
    while ((step = nextArStep(skippedReplay, rounds, inner, round + 1)) && step.role !== "final") {
      skipped.push({ ...foldArRun(repository, --skippedK, step), state: "skipped" });
      skippedReplay.push(hypotheticalRow(step, -skippedK, skippedReplay));
    }
    const finalIndex = out.findIndex((row) => row.role === "final");
    out.splice(finalIndex < 0 ? out.length : finalIndex, 0, ...skipped);
  }
  return out;
}

/** The next run follows loop-ar.sh; hypothetical verdicts below expand only unobserved work. */
function nextArStep(ledger: ReviewLedgerEntry[], rounds: number, inner: number, start = 1): ArStep | null {
  if (ledger.some((row) => row.role === "final")) return null;
  for (let round = start; round <= rounds; round++) {
    let current = 0;
    const rows = ledger.filter((row) => {
      if (row.stage.startsWith("round")) current = Number(row.stage.slice(5));
      if (row.stage.startsWith("fix")) current = Number(row.stage.slice(3));
      return current === round;
    });
    if (rows.some((row) => row.role === "fix")) continue;
    const reviewer = PROVIDERS[(round - 1) % 3];
    const third = PROVIDERS[(round + 1) % 3];
    const confirm = rows.find((row) => row.role === "confirm");
    const critic = confirm && confirm.verdict !== "AGREE" ? third : PROVIDERS[round % 3];
    const reviews = rows.filter((row) => row.role === "review" || row.role === "respond");
    const last = rows.at(-1);
    const step = (role: ReviewRole, provider: ReviewProvider, stage = `round${round}`): ArStep => ({ stage, role, provider });
    if (!last) return step("review", reviewer);
    if (last.role === "review" || last.role === "respond") return step("critic", critic);
    if (last.role === "confirm" && last.verdict === "AGREE") return step("final", "claude", "final");
    if (last.role === "critic" && last.verdict === "AGREE" && reviews.length === 1
      && reviews[0].verdict === "APPROVE" && !confirm) return step("confirm", third);
    if ((last.role === "critic" || last.role === "confirm") && last.verdict !== "AGREE") {
      return reviews.length < inner ? step("respond", reviewer) : step("disputed", "-", "disputed");
    }
    if (round < rounds && (reviews.at(-1)?.verdict === "NEEDS_CHANGES" || last.role === "disputed")) {
      return step("fix", "claude", `fix${round}`);
    }
  }
  return { stage: "final", role: "final", provider: "claude" };
}

/** Choose the longest possible continuation: first approval → confirmation veto → cap. */
function hypotheticalRow(step: ArStep, k: number, ledger: ReviewLedgerEntry[]): ReviewLedgerEntry {
  const verdict = step.role === "review" ? "APPROVE"
    : step.role === "critic" ? (ledger.at(-1)?.role === "review" && ledger.at(-1)?.verdict === "APPROVE" ? "AGREE" : "DISAGREE_EVIDENCE")
    : step.role === "confirm" ? "DISAGREE_EVIDENCE"
    : step.role === "respond" ? "NEEDS_CHANGES"
    : step.role === "disputed" ? "DISPUTED"
    : step.role === "fix" ? "NOOP" : "APPROVE";
  return { ...step, k, phase: "", repository: "", verdict, runDir: "" };
}

function foldArRun(repository: string, k: number, step: ArStep, run?: ReviewRun): ReviewStageRun {
  const chain: ReviewChain = step.role === "fix" ? "review-fix" : step.role === "final" ? "review-final"
    : step.role === "disputed" ? "review-disputed" : `review-${step.provider as "gpt" | "claude" | "grok"}`;
  return { ...foldRun(repository, { k, stage: step.stage, chain }, run),
    role: step.role, provider: step.provider, verdict: null, runDir: run?.runDir ?? null,
    state: run ? runState(run) : "planned" };
}

function foldRun(repository: string, planned: PlannedReviewRun, run: ReviewRun | undefined): ReviewStageRun {
  return {
    repository, ...planned, state: run ? runState(run) : "todo",
    engine: run?.meta?.engine || run?.leg?.engine || null,
    model: run?.meta?.model || run?.leg?.model || null,
    summary: run?.status?.summary ?? null,
  };
}

function runState(run: ReviewRun): ReviewRunState {
  const outcome = run.status?.outcome;
  if (outcome === "done" || outcome === "question" || outcome === "blocked") return outcome;
  return run.meta ? "failed" : "running";
}
