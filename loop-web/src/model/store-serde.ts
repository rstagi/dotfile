// Pure serialize/parse for the per-loop store file (`~/.loop/loops/<runId>.json`). The
// I/O (atomic write, debounce, loadAll) lives in `server/store.mjs`; this is the tested,
// filesystem-free boundary so corrupt-file tolerance and the event cap are verifiable.

import type { LoopRecord } from "./store-types.ts";
import { STORE_SCHEMA_VERSION, EVENT_CAP } from "./store-types.ts";
import { emptyRecord } from "./reduce-loop.ts";

export function serializeRecord(record: LoopRecord): string {
  const events = record.events.length > EVENT_CAP ? record.events.slice(record.events.length - EVENT_CAP) : record.events;
  return JSON.stringify({ ...record, schemaVersion: STORE_SCHEMA_VERSION, events });
}

/** Parse a store file into a record, or null if it is unusable (corrupt / no runId). Missing
 * optional fields are backfilled from an empty record so an older file forward-migrates. */
export function parseStoreFile(text: string): LoopRecord | null {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return null;
  }
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const obj = raw as Partial<LoopRecord>;
  if (typeof obj.runId !== "string" || !obj.runId) return null;

  const base = emptyRecord(obj.runId);
  return {
    ...base,
    ...obj,
    schemaVersion: STORE_SCHEMA_VERSION,
    phases: obj.phases && typeof obj.phases === "object" ? obj.phases : {},
    events: Array.isArray(obj.events) ? obj.events : [],
    status: obj.status ?? base.status,
  };
}
