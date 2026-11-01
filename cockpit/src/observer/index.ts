import { createHash } from 'node:crypto';
import { homedir } from 'node:os';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { CockpitEvent } from '../core/events.ts';
import { EventStore } from '../core/store.ts';
import { SessionReader } from '../adapters/sessions/reader.ts';
import { discoverLoopDirectories, matchLoopSession, readLoop } from '../adapters/loop/index.ts';
import type { LoopState } from '../adapters/loop/index.ts';
import { processIntents } from '../intents/processor.ts';
import type { ModelCaller } from '../intents/model.ts';

export interface ObserverOptions {
  home?: string;
  userHome?: string;
  loopDirectories?: string[];
  model?: ModelCaller;
  intervalMs?: number;
  onError?: (error: unknown) => void;
}

export async function createObserver(options: ObserverOptions = {}) {
  const store = await EventStore.open({ home: options.home });
  const reader = await SessionReader.open({ home: store.home, userHome: options.userHome });
  const seen = new Set<string>();
  const latest = new Map<string, string>();
  const times = new Map<string, Record<string, Record<string, string>>>();
  const listeners = new Set<(event: CockpitEvent) => void>();
  const directories = new Set(options.loopDirectories);
  for (const run of store.snapshot()) for (const event of await store.events(run.id)) remember(event);
  let pending: Promise<void> | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;
  let lastPass: string | null = null;
  let lastError: string | null = null;

  async function append(event: CockpitEvent) {
    const key = observationKey(event);
    const fingerprint = payload(event);
    if (isObservation(event) ? latest.get(key) === fingerprint : seen.has(event.id)) return;
    await store.append(event);
    remember(event);
    for (const listener of listeners) listener(event);
  }
  function remember(event: CockpitEvent) {
    seen.add(event.id);
    latest.set(observationKey(event), payload(event));
    const value = Object.values(event).find(value => value && typeof value === 'object' && 'id' in value);
    if (value?.id) {
      const runTimes = times.get(event.runId) ?? {};
      (runTimes[event.type] ??= {})[value.id] = event.timestamp;
      times.set(event.runId, runTimes);
    }
  }
  async function observe() {
    const incoming = await reader.poll();
    for (const run of [...store.snapshot().map(run => run.meta), ...reader.snapshot().map(session => session.meta)]) {
      if (run.cwd) for (const dir of discoverLoopDirectories(run.cwd)) directories.add(dir);
    }
    const sources: { state: LoopState; runId: string }[] = [];
    const batches = new Map<string, CockpitEvent[]>();
    const add = (event: CockpitEvent) => {
      if (isObservation(event) ? latest.get(observationKey(event)) === payload(event) : seen.has(event.id)) return;
      const batch = batches.get(event.runId) ?? [];
      batch.push(event);
      batches.set(event.runId, batch);
    };
    for (const directory of directories) {
      try {
        const events = await readLoop(directory);
        for (const event of events) add(event);
        const state = JSON.parse(await readFile(join(directory, 'state.json'), 'utf8')) as LoopState;
        if (events[0]) sources.push({ state: { ...state, runId: events[0].runId }, runId: events[0].runId });
      } catch (error) { options.onError?.(error); }
    }
    const sessionStates = reader.snapshot();
    const mappings = new Map<string, { runId: string; phaseId: string; laneId?: string }>();
    for (const session of sessionStates) {
      const match = session.meta.cwd && sources.map(source => matchLoopSession(source.state, session.meta.cwd!)).find(Boolean);
      if (match) mappings.set(session.runId, match);
    }
    for (const source of sources) {
      const sessions = sessionStates.filter(session => mappings.get(session.runId)?.runId === source.runId).map(session => ({
        id: session.meta.sessionId ?? session.runId, chatId: session.meta.chatId, title: session.meta.title,
      }));
      if (sessions.length) add({ runId: source.runId, timestamp: new Date().toISOString(), type: 'run.meta',
        id: `sessions-${createHash('sha256').update(JSON.stringify([source.runId, sessions])).digest('hex')}`, meta: { sessions } });
    }
    for (const event of incoming) {
      const match = mappings.get(event.runId);
      if (!match) { add(event); continue; }
      if (event.type === 'run.meta') continue;
      const mapped = { ...event, ...match };
      if (mapped.type === 'action') mapped.action = { ...mapped.action, phaseId: match.phaseId, laneId: match.laneId };
      add(mapped);
    }
    for (const [runId, batch] of batches) {
      const previous = store.getRun(runId);
      for (const event of batch) await append(event);
      for (const event of await processIntents(previous, batch, options.model)) await append(event);
    }
    await reader.save();
    await store.prune();
    lastPass = new Date().toISOString();
    lastError = null;
  }
  function pass(): Promise<void> {
    if (pending) return pending;
    pending = observe().catch(error => {
      lastError = 'The observer pass failed.';
      throw error;
    }).finally(() => { pending = undefined; });
    return pending;
  }
  return {
    store, pass,
    eventTimes(runId: string) { return times.get(runId) ?? {}; },
    mediaFolders() {
      return [...directories].map(dir => join(dir, 'runs')).concat(reader.snapshot().flatMap(session => [
        ...(session.meta.cwd ? [session.meta.cwd] : []),
        ...(session.meta.chatId ? [join(options.userHome ?? homedir(), '.beluga/chats', session.meta.chatId, '.beluga/scratch')] : []),
      ]));
    },
    get status() { return { lastPass, lastError, intervalMs: options.intervalMs ?? 30_000 }; },
    subscribe(listener: (event: CockpitEvent) => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    start() {
      if (timer) return;
      void pass().catch(error => options.onError?.(error));
      timer = setInterval(() => { void pass().catch(error => options.onError?.(error)); }, options.intervalMs ?? 30_000);
    },
    async stop() { if (timer) clearInterval(timer); timer = undefined; await pending; },
  };
}

function observationKey(event: CockpitEvent): string {
  const value = Object.values(event).find(value => value && typeof value === 'object' && 'id' in value);
  return JSON.stringify([event.runId, event.type, value?.id ?? (event.type === 'run.meta' && event.meta.sessions ? 'sessions' : '')]);
}
function payload(event: CockpitEvent): string {
  const { id: _id, timestamp: _timestamp, ...value } = event;
  return JSON.stringify(value);
}

function isObservation(event: CockpitEvent): boolean {
  return ['run.meta', 'drift', 'ask', 'media', 'checkpoint'].includes(event.type);
}
