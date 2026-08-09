// Render a daemon `LoopRecord` into the browser-facing `Snapshot`, reusing the tested pure
// layer UNCHANGED: it builds an *effective* state.json (overlay ranks → phases[n].status),
// serializes the merged/deduped timeline, and feeds both — plus the live runs/hil/mtimes —
// into `parseLoop` → `buildSnapshot`. A live loop keeps its runs for the heartbeat; an
// archived loop (live === null) short-circuits to the frozen `record.lastSnapshot`.

import type { LoopRecord, LoopSummary, PhaseCounts } from "./store-types.ts";
import type { Snapshot, StateJson, StatePhase, RawEvent, SubOrchInfo } from "./types.ts";
import type { LoopInput } from "./parse-loop.ts";
import { parsePlan } from "./parse-plan.ts";
import { parseLoop } from "./parse-loop.ts";
import { buildSnapshot } from "./snapshot.ts";
import { effectivePhaseStatus } from "./reduce-loop.ts";

interface MaterializeOpts {
  now?: number;
  nowIso?: string | null;
}

const FALLBACK_PLAN = "# No plan found — Multi-Phase Plan\n";
const EMPTY_LIVE: LoopInput = { present: true, state: null, events: null, runs: [], hil: [] };

export function materialize(record: LoopRecord, live: LoopInput | null, opts: MaterializeOpts = {}): Snapshot {
  // Archived: the worktree is gone (no runs/hil to read) — return the frozen last snapshot,
  // or rebuild a runless one from the record when nothing was ever stored. Spread defaults so a
  // snapshot frozen by a pre-subOrch build still satisfies the Snapshot shape (never undefined).
  if (live == null) {
    const frozen = record.lastSnapshot;
    if (!frozen) return render(record, EMPTY_LIVE, opts);
    // Backfill defaults ONLY for a snapshot frozen by a pre-subOrch build (keep the same
    // reference otherwise, so a current snapshot short-circuits untouched).
    const current = Object.prototype.hasOwnProperty.call(frozen, "subOrch");
    return current ? frozen : { ...frozen, subOrch: null, pendingHil: 0 };
  }
  return render(record, live, opts);
}

export function summarize(record: LoopRecord): LoopSummary {
  return {
    runId: record.runId,
    effort: record.effort,
    status: record.status,
    integrationBranch: record.integrationBranch,
    startedAt: record.startedAt,
    finishedAt: record.finishedAt,
    updatedAt: record.updatedAt,
    prUrl: record.prUrl,
    reviewOutcome: record.review?.outcome ?? null,
    phaseCounts: countPhases(record),
    pendingHil: countHilOpen(record),
  };
}

// ---------------------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------------------

function render(record: LoopRecord, live: LoopInput, opts: MaterializeOpts): Snapshot {
  const plan = parsePlan(record.planText ?? FALLBACK_PLAN);
  const loopInput: LoopInput = {
    present: true,
    state: JSON.stringify(effectiveState(record)),
    events: eventsToJsonl(record.events),
    runs: live.runs,
    hil: live.hil,
  };
  const runtime = parseLoop(loopInput);
  const now = opts.now ?? Date.now();
  const snap = buildSnapshot(plan, runtime, { now, nowIso: opts.nowIso ?? null });
  return { ...snap, subOrch: buildSubOrch(record), pendingHil: countHilOpen(record) };
}

/** Count phases with an open HIL — the single source for pendingHil (drift-free, idempotent). */
function countHilOpen(record: LoopRecord): number {
  return Object.values(record.phases).filter((o) => o.hilOpen).length;
}

/** Fold the sub-orchestrator overlay into the snapshot shape, or null when no sub activity. */
function buildSubOrch(record: LoopRecord): SubOrchInfo | null {
  if (record.subRecycles === 0 && record.occupancy == null) return null;
  return {
    recycles: record.subRecycles,
    contextTokens: record.occupancy?.tokens ?? null,
    contextPct: record.occupancy?.percent ?? null,
  };
}

/** state.json with each phase's status replaced by the overlay-resolved effective status;
 * every other field (slug/lane/branch) is preserved so parseLoop still maps attempts. */
function effectiveState(record: LoopRecord): StateJson {
  const base = record.lastState ?? {};
  const phases: Record<string, StatePhase> = {};
  for (const num of phaseNums(record)) {
    phases[num] = { ...(base.phases?.[num] ?? {}), status: effectivePhaseStatus(record, num) };
  }
  return { ...base, phases };
}

function eventsToJsonl(events: RawEvent[]): string {
  return events.map((e) => JSON.stringify(e)).join("\n");
}

// ---------------------------------------------------------------------------------------
// summary
// ---------------------------------------------------------------------------------------

function countPhases(record: LoopRecord): PhaseCounts {
  const counts: PhaseCounts = { total: 0, todo: 0, running: 0, blocked: 0, done: 0, merged: 0 };
  for (const num of phaseNums(record)) {
    counts.total++;
    switch (effectivePhaseStatus(record, num)) {
      case "merged":
        counts.merged++;
        break;
      case "done":
        counts.done++;
        break;
      case "running":
      case "claimed":
        counts.running++;
        break;
      case "blocked":
        counts.blocked++;
        break;
      default:
        counts.todo++;
        break;
    }
  }
  return counts;
}

function phaseNums(record: LoopRecord): string[] {
  return [...new Set([...Object.keys(record.lastState?.phases ?? {}), ...Object.keys(record.phases)])];
}
