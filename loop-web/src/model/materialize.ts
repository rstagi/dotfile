// Render a daemon `LoopRecord` into the browser-facing `Snapshot`, reusing the tested pure
// layer UNCHANGED: it builds an *effective* state.json (overlay ranks → phases[n].status),
// serializes the merged/deduped timeline, and feeds both — plus the live runs/hil/mtimes —
// into `parseLoop` → `buildSnapshot`. A live loop keeps its runs for the heartbeat; an
// archived loop (live === null) short-circuits to the frozen `record.lastSnapshot`.

import type { LoopRecord, LoopSummary, PhaseCounts } from "./store-types.ts";
import type { Snapshot, StateJson, StatePhase, RawEvent } from "./types.ts";
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
  // or rebuild a runless one from the record when nothing was ever stored.
  if (live == null) return record.lastSnapshot ?? render(record, EMPTY_LIVE, opts);
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
  return buildSnapshot(plan, runtime, { now, nowIso: opts.nowIso ?? null });
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
