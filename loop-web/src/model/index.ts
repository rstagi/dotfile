// Barrel for the pure model layer. The server imports from here; Node strips the types
// at runtime (erasable syntax only — no enums/namespaces), so keep every export erasable.
export * from "./types.ts";
export { parsePlan } from "./parse-plan.ts";
export { parseLoop } from "./parse-loop.ts";
export type { LoopInput, RawRunDir, RawHil } from "./parse-loop.ts";
export { buildGraph, PLAN_ID, REVIEW_ID } from "./build-graph.ts";
export { buildSnapshot } from "./snapshot.ts";
export {
  classifyAttempt,
  attemptEnded,
  deriveLiveness,
  normalizeEvent,
  lastWorkedBy,
  summarizeAttempt,
  problemSeverity,
  DEFAULT_STALE_MS,
} from "./derive.ts";
