// The daemon's per-loop record contract — the state the central Loop Observatory daemon
// keeps for EACH loop it observes, folded from `register` / `state` / `event` / `finish`
// ingests by the pure reducer (`reduce-loop.ts`) and rendered to a `Snapshot` by
// `materialize.ts`. Persisted verbatim to `~/.kestral/loops/<runId>.json` by the store.
//
// The load-bearing idea is the **promotion lattice**: a phase's rank can only ever advance
// (`todo < claimed < running < done < merged`). Both state.json pushes AND script events
// join into the same rank via a monotone max, so a finished-attempt event promotes a phase
// to `done` even when the orchestrator's state.json bookkeeping still says `running` — the
// exact gap behind "done phases stuck at running/todo". Ranks never regress (v1 accepts
// "a phase never un-completes in the UI"); sub-`done` status still tracks the live state
// (so `blocked → running` after a HIL resolution is honoured — blocked is not a rank).

import type { StateJson, RawEvent, Snapshot, PhaseStateStatus } from "./types.ts";

// ---------------------------------------------------------------------------------------
// Lattice
// ---------------------------------------------------------------------------------------

/** The monotone promotion lattice. `blocked` is deliberately NOT a rank (it is a live
 * sub-`done` status, so it can move back to `running`); it maps to rank 0 in `rankOf`. */
export type PhaseRank = "todo" | "claimed" | "running" | "done" | "merged";

export interface PhaseOverlay {
  /** Highest rank reached from any state push or script event (monotone max). */
  rank: PhaseRank;
  /** True while an unanswered HIL request is open for the phase. */
  hilOpen: boolean;
  /** Latest event-driven problem hint (e.g. "verify-fail", "merge-conflict"); non-promoting. */
  problem: string | null;
}

// ---------------------------------------------------------------------------------------
// Loop lifecycle
// ---------------------------------------------------------------------------------------

/** `active` = running · `paused` = a HIL request is open · `finished` = loop.finish seen ·
 * `archived` = finished AND its loop dir is gone (set by the server, not the reducer). */
export type LoopStatus = "active" | "paused" | "finished" | "archived";

// ---------------------------------------------------------------------------------------
// Ingests — the messages the reducer folds
// ---------------------------------------------------------------------------------------

export interface RegisterInfo {
  runId: string;
  effort?: string | null;
  projectId?: string | null;
  /** Absolute path to `.kestral/loop` (may no longer exist once the worktree is gone). */
  loopDir?: string | null;
  /** Absolute path to the plan markdown. */
  planFile?: string | null;
  integrationBranch?: string | null;
  startedAt?: string | null;
  /** The whole plan markdown, read once by the server at register time. */
  planText?: string | null;
}

export interface ReviewInfo {
  outcome: string | null;
  summary: string | null;
  /** `runs/review-a<K>/report.md` — the full stored PR-review report. */
  reportPath: string | null;
  /** URL of the GitHub PR comment the report was posted to. */
  commentUrl: string | null;
}

export interface FinishInfo {
  /** Final plan status label (e.g. "integrating"); free-form. */
  status?: string | null;
  finishedAt?: string | null;
  prUrl?: string | null;
  review?: ReviewInfo | null;
}

/** One event push. Carries the typed vocabulary fields (`outcome`/`exitCode`/…) that the
 * on-disk `events.jsonl` line (a bare `RawEvent`) lacks — the runner emits these straight
 * to the daemon, bypassing state.json, which is how promotion beats the bookkeeping lag. */
export interface EventInfo {
  event: string;
  phase?: string | null;
  detail?: string | null;
  ts?: string | null;
  outcome?: string | null;
  exitCode?: number | null;
  engine?: string | null;
  model?: string | null;
  prUrl?: string | null;
}

export type Ingest =
  | { kind: "register"; info: RegisterInfo }
  | { kind: "state"; state: StateJson; planText?: string | null }
  | { kind: "event"; event: EventInfo }
  | { kind: "eventsFile"; text: string }
  | { kind: "finish"; info: FinishInfo };

// ---------------------------------------------------------------------------------------
// The record (persisted per loop) & derived summary
// ---------------------------------------------------------------------------------------

export const STORE_SCHEMA_VERSION = 1;

/** Per-loop event cap — drop oldest to bound disk (Resolved decision #2). */
export const EVENT_CAP = 5000;

export interface LoopRecord {
  schemaVersion: number;
  runId: string;
  effort: string | null;
  projectId: string | null;
  loopDir: string | null;
  planFile: string | null;
  integrationBranch: string | null;
  startedAt: string | null;
  finishedAt: string | null;
  status: LoopStatus;
  /** Most recent full state.json push (last-write-wins). */
  lastState: StateJson | null;
  /** The plan markdown (for the whole-plan drawer + archived materialize). */
  planText: string | null;
  /** Per-phase promotion overlay, keyed by phase number ("1".."N"). */
  phases: Record<string, PhaseOverlay>;
  /** Merged, deduped, capped event timeline (raw rows; display shape derived downstream). */
  events: RawEvent[];
  review: ReviewInfo | null;
  prUrl: string | null;
  /** Last materialized snapshot — the archived short-circuit reads this. */
  lastSnapshot: Snapshot | null;
  updatedAt: string | null;
}

export interface PhaseCounts {
  total: number;
  todo: number;
  running: number;
  blocked: number;
  done: number;
  merged: number;
}

export interface LoopSummary {
  runId: string;
  effort: string | null;
  status: LoopStatus;
  integrationBranch: string | null;
  startedAt: string | null;
  finishedAt: string | null;
  updatedAt: string | null;
  prUrl: string | null;
  reviewOutcome: string | null;
  phaseCounts: PhaseCounts;
}

/** The GET /api/loops row the selector renders. */
export type LoopListEntry = LoopSummary;

/** The effective phase status the overlay resolves to (rank override ?? live state). */
export type EffectivePhaseStatus = PhaseStateStatus;
