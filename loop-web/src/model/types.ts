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
  slug?: string;
  taskId?: string;
  lane?: string;
  branch?: string;
  worktree?: string | null;
  status?: PhaseStateStatus | string;
  attempt?: number;
  runDir?: string;
  pid?: number;
  sessionId?: string;
  questionRounds?: number;
}

export type PhaseStateStatus =
  | "todo"
  | "claimed"
  | "running"
  | "merged"
  | "blocked"
  | "done";

export interface StateJson {
  runId?: string;
  effort?: string;
  projectId?: string;
  workContextId?: string;
  integrationBranch?: string;
  integrationWorktree?: string;
  prUrl?: string | null;
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
  ts?: string;
  event?: string;
  phase?: string;
  detail?: string;
}

// ---------------------------------------------------------------------------------------
// Parsed plan (from the plan markdown)
// ---------------------------------------------------------------------------------------

export type PlanPhaseStatus = "todo" | "in-progress" | "blocked" | "done";

export interface LoopConfig {
  integrationBranch: string | null;
  verify: string | null;
  pr: string | null;
  concurrency: number | null;
}

export interface PlanPhase {
  /** Phase number as string ("1".."N"), from the `Phase N` marker. */
  phase: string;
  /** Descriptive title with the bracket markers stripped. */
  title: string;
  lane: string;
  status: PlanPhaseStatus;
  /** Phase numbers this phase depends on (from "Depends on:"). */
  dependsOn: string[];
  suggestedBranch: string | null;
  touches: string | null;
  doneWhen: string | null;
  verify: string | null;
  taskUrl: string | null;
  notes: string | null;
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
  phases: PlanPhase[];
  prose: PlanProse;
  warnings: string[];
}

/** One phase row of the whole-plan drawer summary. */
export interface PlanPhaseSummary {
  phase: string;
  title: string;
  lane: string;
  status: PlanPhaseStatus;
  dependsOn: string[];
}

/** The plan digest carried on every Snapshot — feeds the plan node badges + the plan drawer. */
export interface PlanOverview {
  name: string;
  status: string | null;
  updatedAt: string | null;
  loopConfig: LoopConfig | null;
  phaseSummary: PlanPhaseSummary[];
  laneCount: number;
  prose: PlanProse;
}

// ---------------------------------------------------------------------------------------
// Parsed runtime (from .loop/)
// ---------------------------------------------------------------------------------------

/** One attempt dir: runs/<slug>-a<K>. Files are read by the server and handed in as content. */
export interface Attempt {
  k: number;
  runDir: string;
  meta: MetaJson | null;
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
}

export interface ReviewRun {
  k: number;
  runDir: string;
  status: StatusJson | null;
}

export interface Runtime {
  present: boolean;
  state: StateJson | null;
  phases: Record<string, PhaseRuntime>;
  events: RawEvent[];
  reviewRuns: ReviewRun[];
  /** Reserved notes/pr-review.md steering note. */
  reviewNote: string | null;
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
  | "problem";

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
  runId: string | null;
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
}
