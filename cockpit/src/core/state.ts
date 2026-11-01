import type { CockpitEvent, RunState } from "./events.ts";

export function reduceEvent(previous: RunState | undefined, event: CockpitEvent): RunState {
  const run: RunState = previous ? { ...previous } : {
    id: event.runId,
    createdAt: event.timestamp,
    updatedAt: event.timestamp,
    meta: {},
    actions: [], intents: [], groups: [], phases: [], drifts: [], asks: [], media: [], checkpoints: [],
  };
  if (Date.parse(event.timestamp) < Date.parse(run.createdAt)) run.createdAt = event.timestamp;
  if (Date.parse(event.timestamp) > Date.parse(run.updatedAt)) run.updatedAt = event.timestamp;
  switch (event.type) {
    case "run.meta": run.meta = { ...run.meta, ...event.meta }; break;
    case "action": run.actions = [...run.actions, event.action]; break;
    case "intent.upsert": run.intents = upsert(run.intents, event.intent); break;
    case "group.upsert": run.groups = upsert(run.groups, event.group); break;
    case "phase.start":
      run.phases = upsert(run.phases, { ...event.phase, status: "running", startedAt: event.timestamp });
      break;
    case "phase.end": {
      const phase = run.phases.find((phase) => phase.id === event.phase.id);
      run.phases = upsert(run.phases, {
        title: phase?.title ?? event.phase.id, ...phase, ...event.phase,
        status: event.phase.status ?? "done", endedAt: event.timestamp,
      });
      break;
    }
    case "drift": run.drifts = upsert(run.drifts, event.drift); break;
    case "ask": run.asks = upsert(run.asks, event.ask); break;
    case "media": run.media = upsert(run.media, event.media); break;
    case "checkpoint": run.checkpoints = upsert(run.checkpoints, event.checkpoint); break;
  }
  return run;
}

function upsert<T extends { id: string }>(items: readonly T[], item: T): T[] {
  const index = items.findIndex((existing) => existing.id === item.id);
  if (index < 0) return [...items, item];
  return items.map((existing, i) => i === index ? item : existing);
}
