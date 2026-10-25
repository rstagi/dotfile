// These types define the retained plan parser contract.

export type PlanPhaseStatus = "todo" | "in-progress" | "blocked" | "done";
export type PlanPhaseKind = "work" | "pr-review";
/** What a work phase produces. Non-PR outputs live under `.loop/outputs/p<N>/`. */
export type PlanPhaseOutput = "pr" | "file" | "doc" | "output.md";

export interface LoopConfig {
  integrationBranch: string | null;
  verify: string | null;
  pr: string | null;
  concurrency: number | null;
}

export interface PlanRepository {
  /** GitHub owner/repo slug. Legacy plans use the synthetic `primary` slug. */
  slug: string;
  verify: string | null;
  integrationBranch: string | null;
  pr: string | null;
}

export interface PlanPhase {
  /** Phase number as string ("1".."N"), from the `Phase N` marker. */
  phase: string;
  /** Containing milestone number; legacy `## Phases` belongs to milestone 1. */
  milestone: number;
  /** Descriptive title with the bracket markers stripped. */
  title: string;
  kind: PlanPhaseKind;
  /** Human barrier: `[gate: merge]` (PR outputs) or `[gate: accept]` (file, doc and output.md outputs). */
  gate: "merge" | "accept" | null;
  /** Work phases only (default `pr`); null on review phases. */
  output: PlanPhaseOutput | null;
  lane: string;
  status: PlanPhaseStatus;
  repository: string;
  /** Phase numbers this phase depends on (from "Depends on:"). */
  dependsOn: string[];
  suggestedBranch: string | null;
  touches: string | null;
  doneWhen: string | null;
  verify: string | null;
  notes: string | null;
  /** PR-review phases only: defaults to standard with two outer rounds. */
  review: ReviewConfig | null;
  /** True for a phase listed by an archived milestone stub: it exists, it is `done`, and its
   * block lives in the archive file. Archived phases carry no body fields; their `dependsOn`
   * is a display-only chain in stub order. */
  archived?: boolean;
}

/** Historical tiers remain readable; execution only accepts standard|max. */
export type LegacyReviewTier = "shallow" | "medium";
export type ReviewTier = "standard" | "max" | LegacyReviewTier;

export interface ReviewConfig {
  tier: ReviewTier;
  /** Null = untagged: execution resolves LOOP_REVIEW_ROUNDS_DEFAULT (legacy tiers: their historical default). */
  rounds: number | null;
  /** Maximum reviewer/critic exchanges per outer round; null = LOOP_REVIEW_INNER_DEFAULT. */
  inner: number | null;
}

/** Free-form prose sections of the plan markdown (each nullable). Rendered in the drawer. */
export interface PlanProse {
  goal: string | null;
  approach: string | null;
  parallelGuide: string | null;
  progressLog: string | null;
}

export interface PlanMilestone {
  number: number;
  title: string;
  gatePhase: string | null;
  integrationBranches: Record<string, string | null>;
  prs: Record<string, string | null>;
  /** Repositories assigned to this milestone's work phases, in document order. */
  participants: string[];
  /** Present only for a shipped milestone archived to a stub (`[archived]` heading). */
  archive?: MilestoneArchive;
}

export interface MilestoneArchive {
  /** Archive file path relative to plan.md, e.g. `milestones/m1.md`. */
  file: string | null;
  outcome: string | null;
}

export interface Plan {
  name: string;
  status: string | null;
  updatedAt: string | null;
  loopConfig: LoopConfig | null;
  repositories: PlanRepository[];
  milestones: PlanMilestone[];
  /** True only when the plan declares `## Milestone N` sections (legacy `## Phases` = false). */
  explicitMilestones: boolean;
  phases: PlanPhase[];
  prose: PlanProse;
  warnings: string[];
}

