// Pure serialize/parse for the per-loop store file (`~/.loop/loops/<runId>.json`). The
// I/O (atomic write, debounce, loadAll) lives in `server/store.mjs`; this is the tested,
// filesystem-free boundary so corrupt-file tolerance and the event cap are verifiable.

import type { LoopRecord } from "./store-types.ts";
import type { Snapshot } from "./types.ts";
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
  const legacy = obj.schemaVersion == null || obj.schemaVersion < 2;
  const repositories = obj.repositories && typeof obj.repositories === "object"
    ? obj.repositories
    : legacy && (obj.integrationBranch || obj.prUrl || obj.review)
      ? { primary: {
          sourceRoot: null,
          defaultBranch: null,
          baseSha: null,
          integrationBranch: obj.integrationBranch ?? null,
          integrationWorktree: null,
          prUrl: obj.prUrl ?? null,
          review: obj.review ?? null,
        } }
      : {};
  const phases = obj.phases && typeof obj.phases === "object"
    ? Object.fromEntries(Object.entries(obj.phases).map(([phase, overlay]) => [
        phase,
        { ...overlay, repository: overlay.repository ?? "primary" },
      ]))
    : {};
  const lastSnapshot = obj.lastSnapshot ? migrateSnapshot(obj.lastSnapshot) : null;
  return {
    ...base,
    ...obj,
    schemaVersion: STORE_SCHEMA_VERSION,
    phases,
    repositories,
    lastSnapshot,
    events: Array.isArray(obj.events) ? obj.events : [],
    status: obj.status ?? base.status,
  };
}

function migrateSnapshot(snapshot: Snapshot): Snapshot {
  const effort = {
    ...snapshot.effort,
    repositories: Array.isArray(snapshot.effort?.repositories) ? snapshot.effort.repositories : [],
  };
  const plan = snapshot.plan
    ? {
        ...snapshot.plan,
        repositories: Array.isArray(snapshot.plan.repositories) ? snapshot.plan.repositories : [],
        phaseSummary: (snapshot.plan.phaseSummary ?? []).map((phase) => ({
          ...phase,
          repository: phase.repository ?? "primary",
        })),
      }
    : snapshot.plan;
  const graph = snapshot.graph
    ? {
        ...snapshot.graph,
        nodes: (snapshot.graph.nodes ?? []).map((node) => ({
          ...node,
          repository: node.repository ?? null,
        })),
      }
    : snapshot.graph;
  return { ...snapshot, effort, plan, graph };
}
