// The shared data contract for the Loop Observatory.
//
// Everything downstream — the Node server, the SSE stream, and the React UI — agrees on
// `Snapshot`. It is derived by pure functions in this directory from two sources of truth:
//   1. the plan markdown (grammar: multiphase-plan/references/plan-format.md)
//   2. the runtime dir `.loop/` written by ~/dotfile/loop-*.sh
//
// The runtime schemas are grounded in the real scripts, NOT the idealized docs:
//   meta.json    — loop-runner.sh write_meta():  { engine, model, sessionId, headBefore,
//                  headAfter, engineExit, timedOut }.  engineExit ∈ {0, 40, 124} (the value
//                  passed to write_meta, not the raw last-leg rc); its mere existence means
//                  "attempt ended".
//   status.json  — runner-written: { outcome: "done"|"question"|"blocked", summary,
//                  question?, details? }.  Absent/invalid ⇒ crash.
//   state.json   — orchestrator bookkeeping via loop-state.sh; EVERY field optional.
//   events.jsonl — one row per line: { ts, event, phase, detail }; `event` is an open string.

// ---------------------------------------------------------------------------------------
// Raw file shapes (defensive — every field optional; the scripts guarantee nothing)
// ---------------------------------------------------------------------------------------

export interface MetaJson {
  engine?: string;
  model?: string;
  sessionId?: string;
  headBefore?: string;
  headAfter?: string;
  engineExit?: number;
  timedOut?: boolean;
  proposedProfile?: string | null;
  actualProfile?: string | null;
  routeFallbackReason?: string | null;
}

export type RunnerOutcome = "done" | "question" | "blocked";

export interface StatusJson {
  outcome?: RunnerOutcome | string;
  summary?: string;
  question?: string;
  details?: string;
}

/** state.json phase entry — schema per loop-protocol.md, every field optional. */
export interface StatePhase {
  kind?: PlanPhaseKind;
  slug?: string;
  taskId?: string;
  lane?: string;
  repository?: string;
  branch?: string;
  worktree?: string | null;
  status?: PhaseStateStatus | string;
  attempt?: number;
  runDir?: string;
  pid?: number;
  sessionId?: string;
  questionRounds?: number;
}

export interface StateRepository {
  sourceRoot?: string | null;
  defaultBranch?: string | null;
  baseSha?: string | null;
  integrationBranch?: string | null;
  integrationWorktree?: string | null;
  prUrl?: string | null;
  review?: {
    outcome?: string | null;
    summary?: string | null;
    reportPath?: string | null;
    commentUrl?: string | null;
  } | null;
}

export type PhaseStateStatus =
  | "todo"
  | "claimed"
  | "running"
  | "merged"
  | "blocked"
  | "done"
  | "paused";

export interface StateJson {
  runId?: string;
  effort?: string;
  projectId?: string;
  workContextId?: string;
  integrationBranch?: string;
  integrationWorktree?: string;
  prUrl?: string | null;
  repositories?: Record<string, StateRepository>;
  phases?: Record<string, StatePhase>;
  linkedPrTasks?: string[];
  /** PR-review verdict promoted by the orchestrator (`loop-state.sh set`). Structural (no
   * cross-import) so the daemon reducer can fold it; the drawer renders the full report. */
  review?: {
    outcome?: string;
    summary?: string;
    reportPath?: string;
    commentUrl?: string;
  } | null;
}

/** One row of events.jsonl (loop-state.sh log). */
export interface RawEvent {
  runId?: string;
  ts?: string;
  event?: string;
  phase?: string;
  repository?: string;
  detail?: string;
  attempt?: number;
  stage?: string;
  mode?: JevMode;
  candidate?: string | null;
  confidence?: number | null;
  probabilities?: Record<string, number>;
  appliedAction?: string | null;
  fallbackReason?: string | null;
  resolvedModel?: string | null;
  evidenceChecked?: boolean;
  evidenceSources?: string[];
  questionRound?: number | null;
  head?: string | null;
  requiredGates?: string[];
  completedGates?: string[];
  remainingGates?: string[];
  focus?: string[];
}

export type JevMode = "off" | "shadow" | "active";

export interface JevDecision {
  runId: string;
  phase: string;
  attempt: number;
  stage: string;
  mode: JevMode;
  candidate: string | null;
  confidence: number | null;
  probabilities: Record<string, number>;
  appliedAction: string | null;
  fallbackReason: string | null;
  resolvedModel: string | null;
  evidenceChecked?: boolean;
  evidenceSources?: string[];
  questionRound?: number | null;
  head?: string | null;
  requiredGates?: string[];
  completedGates?: string[];
  remainingGates?: string[];
  focus?: string[];
  ts: string | null;
}

export interface JevStatus {
  mode: JevMode;
  count: number;
  fallbackCount: number;
}

// ---------------------------------------------------------------------------------------
// Parsed plan (from the plan markdown)
// ---------------------------------------------------------------------------------------

export type PlanPhaseStatus = "todo" | "in-progress" | "blocked" | "done";
export type PlanPhaseKind = "work" | "pr-review";

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
  /** Descriptive title with the bracket markers stripped. */
  title: string;
  kind: PlanPhaseKind;
  lane: string;
  status: PlanPhaseStatus;
  repository: string;
  /** Phase numbers this phase depends on (from "Depends on:"). */
  dependsOn: string[];
  suggestedBranch: string | null;
  touches: string | null;
  doneWhen: string | null;
  verify: string | null;
  taskUrl: string | null;
  notes: string | null;
  /** PR-review phases only: `[review: tier]` (default medium) + `[rounds: N]` (null = tier default). */
  review: ReviewConfig | null;
}

export type ReviewTier = "shallow" | "medium" | "max";

export interface ReviewConfig {
  tier: ReviewTier;
  rounds: number | null;
}

/** Free-form prose sections of the plan markdown (each nullable). Rendered in the drawer. */
export interface PlanProse {
  goal: string | null;
  approach: string | null;
  parallelGuide: string | null;
  progressLog: string | null;
}

export interface Plan {
  name: string;
  status: string | null;
  updatedAt: string | null;
  loopConfig: LoopConfig | null;
  repositories: PlanRepository[];
  phases: PlanPhase[];
  prose: PlanProse;
  warnings: string[];
}

/** One phase row of the whole-plan drawer summary. */
export interface PlanPhaseSummary {
  phase: string;
  title: string;
  kind: PlanPhaseKind;
  lane: string;
  status: PlanPhaseStatus;
  repository: string;
  dependsOn: string[];
}

/** The plan digest carried on every Snapshot — feeds the plan node badges + the plan drawer. */
export interface PlanOverview {
  name: string;
  status: string | null;
  updatedAt: string | null;
  loopConfig: LoopConfig | null;
  repositories: PlanRepository[];
  phaseSummary: PlanPhaseSummary[];
  laneCount: number;
  prose: PlanProse;
}

// ---------------------------------------------------------------------------------------
// Parsed runtime (from .loop/)
// ---------------------------------------------------------------------------------------

/** One attempt dir: runs/<slug>-a<K>. Files are read by the server and handed in as content. */
/** `<runDir>/leg.json`: the leg the runner is running now (written at each leg launch). */
export interface LegJson {
  engine: string | null;
  model: string | null;
  effort?: string | null;
  startedAt?: string | null;
}

export interface Attempt {
  k: number;
  runDir: string;
  meta: MetaJson | null;
  /** Running leg — the only engine/model source while the attempt is in flight (no meta). */
  leg?: LegJson | null;
  status: StatusJson | null;
  /**
   * Tail of spawn.log (the orchestrator's `nohup loop-runner.sh > spawn.log 2>&1`).
   * The ONLY file that carries the exit-12 marker "claimed done but verify failed",
   * which is how verify-fail is distinguished from a passing verify (both write verify.log).
   */
  spawnLog: string | null;
  /** transcript.jsonl mtime (epoch ms) — the heartbeat. */
  transcriptMtime: number | null;
  /** meta.json mtime (epoch ms) — when the attempt ended. */
  endedAt: number | null;
}

export interface HilState {
  open: boolean;
  markdown: string | null;
}

export interface PhaseRuntime {
  phase: string;
  state: StatePhase | null;
  attempts: Attempt[];
  hil: HilState | null;
  /** User-authored steering note for this phase, read from notes/<phase>.md. */
  note: string | null;
  /** `control/pause-<phase>` present (user paused this phase). */
  paused: boolean;
  /** `control/model-<phase>` leg (`engine:model[+fallback]`), or null for the default chain. */
  modelOverride: string | null;
}

/** Out-of-band user controls read from `.loop/control/`. */
export interface LoopControl {
  /** `control/pause` present — the whole loop is paused. */
  paused: boolean;
  pausedPhases: string[];
  /** phase → leg override. */
  models: Record<string, string>;
}

export interface ReviewRun {
  k: number;
  runDir: string;
  status: StatusJson | null;
  phase: string | null;
  repository: string | null;
  /** Present once the run ended (same contract as Attempt.meta). */
  meta?: MetaJson | null;
  /** The leg running right now — engine/model while in flight. */
  leg?: LegJson | null;
}

/** loop-review.sh chain names — one run of a review pipeline uses exactly one. */
export type ReviewChain = "review-adv-a" | "review-adv-b" | "review-fix" | "review-final";

export type ReviewRunState = "todo" | "running" | "done" | "question" | "blocked" | "failed";

/** One run (`a<k>`) of an explicit review phase's pipeline for one repository: the planned
 * stage from tier + rounds, folded with its run dir when one exists. */
export interface ReviewStageRun {
  repository: string;
  k: number;
  /** `round<r>` | `fix<r>` | `final`. */
  stage: string;
  chain: ReviewChain;
  state: ReviewRunState;
  engine: string | null;
  model: string | null;
  summary: string | null;
}

export interface Runtime {
  present: boolean;
  state: StateJson | null;
  phases: Record<string, PhaseRuntime>;
  events: RawEvent[];
  reviewRuns: ReviewRun[];
  reviewRunsByRepository: Record<string, ReviewRun[]>;
  /** Legacy reserved notes/pr-review.md steering note. */
  reviewNote: string | null;
  reviewNotes: Record<string, string>;
  control: LoopControl;
}

// ---------------------------------------------------------------------------------------
// Derived snapshot (server → browser)
// ---------------------------------------------------------------------------------------

/** A derived problem class. `question`/`blocked` come from status.json; the rest from the
 * exit-code paths of loop-runner.sh. `chain-exhausted` IS observable (meta.engineExit===40). */
export type ProblemClass =
  | "timeout"
  | "verify-fail"
  | "stall"
  | "crash"
  | "chain-exhausted"
  | "blocked"
  | "question";

/** One resolved UI state per node — the single token the UI maps to a colour. */
export type NodeUiState =
  | "todo"
  | "running"
  | "blocked"
  | "done"
  | "awaiting"
  | "problem"
  | "paused";

export type Liveness = "live" | "flatline";

export interface AttemptSummary {
  k: number;
  engine: string | null;
  model: string | null;
  outcome: string | null;
  problem: ProblemClass | null;
  endedAt: number | null;
  ended: boolean;
}

export interface NodeRuntime {
  attempt: number | null;
  branch: string | null;
  /** "last worked by <engine>:<model>" — the last leg only; intra-chain fallback is invisible. */
  engine: string | null;
  model: string | null;
  lastHeartbeatAgeSec: number | null;
  problem: ProblemClass | null;
  awaiting: boolean;
  hilOpen: boolean;
  /** Full HIL prose (rendered verbatim in the drawer) when a request is open. */
  hilMarkdown: string | null;
  attempts: AttemptSummary[];
  runDir: string | null;
  slug: string | null;
}

export type NodeKind = "plan" | "phase" | "integration" | "pr-review";

export interface GraphNode {
  id: string;
  kind: NodeKind;
  title: string;
  phase: string | null;
  lane: string | null;
  repository: string | null;
  /** Lifecycle status, normalized across the plan and state.json enums. */
  status: PhaseStateStatus;
  /** The one token the UI colours from (folds status + problem + awaiting). */
  ui: NodeUiState;
  /** Heartbeat animation cue: live/flatline while running, else null. */
  pulse: Liveness | null;
  runtime: NodeRuntime | null;
  /** Derived from note presence and incomplete lifecycle; never trusts file deletion. */
  notePending: boolean;
  noteMarkdown: string | null;
  /** Jev observations correlated to this phase. Optional on pre-Jev archived snapshots. */
  decisions?: JevDecision[];
  /** User paused this phase (control/pause-<N> or state `paused`). Absent on older snapshots. */
  paused?: boolean;
  /** User-chosen leg for the next attempt (control/model-<N>). Absent on older snapshots. */
  modelOverride?: string | null;
  /** PR-review nodes: tier + rounds from the plan tags. Optional on older snapshots. */
  review?: ReviewConfig | null;
  /** Explicit PR-review nodes: every pipeline run per repository, in order. Optional on older snapshots. */
  reviewStages?: ReviewStageRun[];
}

export type EdgeKind = "plan-to-lane" | "depends" | "to-review";

export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  kind: EdgeKind;
  /** Dep not yet satisfied — dashed amber, animated toward the waiting node. */
  blocking: boolean;
}

export interface Graph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  lanes: string[];
}

export type EventTone = "hil" | "verify" | "crash" | null;

export interface TimelineEvent {
  ts: string | null;
  event: string;
  phase: string;
  repository: string;
  detail: string;
  glyph: string;
  label: string;
  known: boolean;
  /** Warning treatment for the timeline row, derived once with the glyph (no re-scanning). */
  tone: EventTone;
}

export interface Problem {
  id: string;
  nodeId: string;
  phase: string | null;
  title: string;
  class: ProblemClass | "hil";
  severity: number;
  detail: string | null;
  /** Full HIL prose (rendered verbatim in the drawer) when class === "hil". */
  hilMarkdown: string | null;
}

export interface PrInfo {
  url: string | null;
  /** Structured review outcome (done|question|blocked) — prefers state.review, else the run. */
  outcome: string | null;
  /** Human-readable review summary for display — never used to derive pass/fail. */
  verdict: string | null;
  reviewPresent: boolean;
  /** `runs/review-a<K>/report.md` (from state.review) — the full report the drawer fetches. */
  reportPath: string | null;
  /** URL of the GitHub PR comment the review was posted to (from state.review). */
  commentUrl: string | null;
  /** Slug + attempt of the latest review run (for the per-attempt detail fetch), if any. */
  reviewSlug: string | null;
  reviewAttempt: number | null;
}

export interface EffortInfo {
  name: string;
  status: string | null;
  integrationBranch: string | null;
  updatedAt: string | null;
  pr: PrInfo | null;
  repositories: RepositoryInfo[];
  runId: string | null;
}

export interface RepositoryInfo {
  slug: string;
  integrationBranch: string | null;
  verify: string | null;
  pr: PrInfo | null;
}

/** Sub-orchestrator context health: recycle count + latest context-window occupancy. */
export interface SubOrchInfo {
  recycles: number;
  contextTokens: number | null;
  contextPct: number | null;
}

export interface Snapshot {
  effort: EffortInfo;
  plan: PlanOverview;
  graph: Graph;
  problems: Problem[];
  events: TimelineEvent[];
  loopActive: boolean;
  generatedAt: string | null;
  warnings: string[];
  /** Sub-orchestrator context health, or null when no sub activity has been observed. */
  subOrch: SubOrchInfo | null;
  /** Count of phases awaiting a human decision (open HIL). */
  pendingHil: number;
  /** Typed Jev observations. Absent only on pre-Jev archived snapshots. */
  decisions?: JevDecision[];
  /** Latest mode plus aggregate counts. Null/absent for pre-Jev loops. */
  jev?: JevStatus | null;
  /** Whole loop paused (control/pause). Absent on older snapshots. */
  paused?: boolean;
}
