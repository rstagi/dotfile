import type { LoopListEntry } from "../model/store-types.ts";

/**
 * Choose which loop the observatory should show by default: keep `current` if it is still in the
 * list, else the most-recent `active` loop, else the most-recent `finished` one, else the first.
 * Pure so it can be unit-tested; "most recent" is by updatedAt (falling back to finished/started).
 */
export function pickDefaultRunId(loops: LoopListEntry[], current: string | null): string | null {
  if (loops.length === 0) return null;
  if (current && loops.some((l) => l.runId === current)) return current;
  return mostRecent(loops, "active") ?? mostRecent(loops, "finished") ?? loops[0].runId;
}

function mostRecent(loops: LoopListEntry[], status: LoopListEntry["status"]): string | null {
  const matching = loops.filter((l) => l.status === status).sort((a, b) => tsOf(b) - tsOf(a));
  return matching.length ? matching[0].runId : null;
}

function tsOf(l: LoopListEntry): number {
  const raw = l.updatedAt ?? l.finishedAt ?? l.startedAt;
  const n = raw ? Date.parse(raw) : NaN;
  return Number.isFinite(n) ? n : 0;
}
