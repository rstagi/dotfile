import { appendFile, mkdir, readFile, readdir, unlink } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import type { CockpitEvent, RunState } from "./events.ts";
import { reduceEvent } from "./state.ts";

export interface StoreOptions {
  home?: string;
  now?: () => number;
}

export const RETENTION_MS = 30 * 24 * 60 * 60 * 1000;
const RUN_ID = /^[a-zA-Z0-9][a-zA-Z0-9._-]*$/;
const PAYLOAD_KEYS = {
  action: "action", "intent.upsert": "intent", "group.upsert": "group",
  "phase.start": "phase", "phase.end": "phase", drift: "drift", ask: "ask",
  media: "media", checkpoint: "checkpoint", "run.meta": "meta",
} as const;

export class EventStore {
  readonly home: string;
  private readonly runs = new Map<string, RunState>();
  private pending: Promise<void> = Promise.resolve();
  private readonly now: () => number;

  private constructor(options: StoreOptions) {
    this.home = options.home ?? process.env.COCKPIT_HOME ?? join(homedir(), ".cockpit");
    this.now = options.now ?? Date.now;
  }

  static async open(options: StoreOptions = {}): Promise<EventStore> {
    const store = new EventStore(options);
    await mkdir(join(store.home, "runs"), { recursive: true, mode: 0o700 });
    for (const entry of (await readdir(join(store.home, "runs"), { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      if (!entry.isFile() || !entry.name.endsWith(".jsonl")) continue;
      const runId = entry.name.slice(0, -6);
      assertRunId(runId);
      for (const event of await store.readEvents(runId)) store.apply(event);
    }
    await store.prune();
    return store;
  }

  append(event: CockpitEvent): Promise<void> {
    const captured = structuredClone(event);
    return this.enqueue(async () => {
      assertEvent(captured);
      await appendFile(this.path(captured.runId), `${JSON.stringify(captured)}\n`, { mode: 0o600 });
      this.apply(captured);
    });
  }

  getRun(runId: string): RunState | undefined {
    return structuredClone(this.runs.get(runId));
  }

  snapshot(): RunState[] {
    return structuredClone([...this.runs.values()].sort((a, b) => a.id.localeCompare(b.id)));
  }

  async events(runId: string): Promise<CockpitEvent[]> {
    await this.pending;
    return this.readEvents(runId);
  }

  prune(now = this.now()): Promise<string[]> {
    return this.enqueue(async () => {
      const removed: string[] = [];
      for (const run of this.runs.values()) {
        if (Date.parse(run.updatedAt) >= now - RETENTION_MS) continue;
        await unlink(this.path(run.id));
        this.runs.delete(run.id);
        removed.push(run.id);
      }
      return removed.sort();
    });
  }

  private async readEvents(runId: string): Promise<CockpitEvent[]> {
    let content: string;
    try {
      content = await readFile(this.path(runId), "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
      throw error;
    }
    const events: CockpitEvent[] = [];
    for (const [index, line] of content.split("\n").entries()) {
      if (!line.trim()) continue;
      try {
        const event: unknown = JSON.parse(line);
        assertEvent(event);
        if (event.runId !== runId) throw new Error("The event run ID does not match the file.");
        events.push(event);
      } catch {
        throw new Error(`The event log for ${runId} has an invalid event at line ${index + 1}.`);
      }
    }
    return events;
  }

  private apply(event: CockpitEvent): void {
    this.runs.set(event.runId, reduceEvent(this.runs.get(event.runId), event));
  }

  private path(runId: string): string {
    assertRunId(runId);
    return join(this.home, "runs", `${runId}.jsonl`);
  }

  private enqueue<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.pending.then(operation);
    this.pending = result.then(() => undefined, () => undefined);
    return result;
  }
}

function assertRunId(runId: string): void {
  if (!RUN_ID.test(runId)) throw new Error("The run ID must contain only letters, digits, dots, underscores, or hyphens and start with a letter or digit.");
}

function assertEvent(value: unknown): asserts value is CockpitEvent {
  if (!value || typeof value !== "object") throw new Error("The event must be an object.");
  const event = value as Record<string, unknown>;
  if (typeof event.runId !== "string") throw new Error("The event must have a run ID.");
  assertRunId(event.runId);
  if (typeof event.id !== "string" || !event.id) throw new Error("The event must have an ID.");
  if (typeof event.timestamp !== "string" || !Number.isFinite(Date.parse(event.timestamp))) {
    throw new Error("The event must have a valid timestamp.");
  }
  if (typeof event.type !== "string" || !Object.hasOwn(PAYLOAD_KEYS, event.type)) {
    throw new Error("The event type is unknown.");
  }
  const key = PAYLOAD_KEYS[event.type as keyof typeof PAYLOAD_KEYS];
  const payload = event[key];
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) throw new Error("The event payload must be an object.");
  if (key !== "meta" && (typeof (payload as Record<string, unknown>).id !== "string" || !(payload as Record<string, unknown>).id)) {
    throw new Error("The event payload must have an ID.");
  }
}
