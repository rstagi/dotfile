import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, it } from 'vitest';
import { createObserver } from '../observer/index.ts';
import { startServer } from './index.ts';
import type { RunState } from '../core/events.ts';

it('replays a snapshot from the event log before a later action', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cockpit-replay-'));
  const observer = await createObserver({ home: root, userHome: root });
  const now = Date.now();
  const timestamp = new Date(now - 10_000).toISOString();
  await observer.store.append({ id: 'meta', runId: 'run', timestamp, type: 'run.meta', meta: { title: 'Replay', sessions: [{ id: 'runner', chatId: 'chat-runner' }] } });
  await observer.store.append({ id: 'a1', runId: 'run', timestamp: new Date(now).toISOString(), type: 'action', action: { id: 'a1', tool: 'Read', summary: 'Read the source.' } });
  const server = await startServer({ observer, port: 0 });
  try {
    const snapshot = await (await fetch(`${server.url}/api/state?view=session&chat=chat-runner&at=${encodeURIComponent(timestamp)}`)).json() as { runs: RunState[]; sessions: { chatId?: string }[] };
    expect(snapshot.runs).toHaveLength(1);
    expect(snapshot.runs[0].actions).toEqual([]);
    expect(snapshot.sessions[0].chatId).toBe('chat-runner');
    expect((await fetch(`${server.url}/api/state?at=invalid`)).status).toBe(400);
    expect((await fetch(`${server.url}/api/state`, { method: 'POST' })).status).toBe(405);
    expect((await fetch(`${server.url}/api/state`, { headers: { Origin: 'https://example.com' } })).status).toBe(403);
  } finally { await server.close(); await rm(root, { recursive: true, force: true }); }
}, 30_000);
