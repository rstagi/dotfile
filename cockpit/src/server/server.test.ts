import { createServer } from 'node:http';
import { mkdtemp, mkdir, writeFile, symlink, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { createObserver } from '../observer/index.ts';
import { startServer } from './index.ts';

const cleanups: (() => Promise<unknown>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.reverse()) await cleanup(); cleanups.length = 0; });
it('serves snapshots, replay events and SSE, and refuses media outside allowed folders', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cockpit-http-'));
  cleanups.push(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'ui'));
  await mkdir(join(root, 'proof'));
  await writeFile(join(root, 'ui/index.html'), '<h1>Cockpit</h1>');
  await writeFile(join(root, 'proof/ok.png'), 'image');
  await writeFile(join(root, 'outside.png'), 'private');
  await symlink(join(root, 'outside.png'), join(root, 'proof/link.png'));
  const observer = await createObserver({ home: join(root, 'store'), userHome: root, model: async () => '{}' });
  const event = { id: 'meta', runId: 'demo', timestamp: new Date().toISOString(), type: 'run.meta' as const, meta: { chatId: 'chat-demo', title: 'Demo' } };
  await observer.store.append(event);
  for (const [id, path] of [['ok', 'proof/ok.png'], ['outside', 'outside.png'], ['link', 'proof/link.png']]) {
    await observer.store.append({ ...event, id, type: 'media', media: { id, path: join(root, path), mimeType: 'image/png' } });
  }
  const server = await startServer({ observer, port: 0, uiDirectory: join(root, 'ui'), allowedFolders: () => [join(root, 'proof')] });
  cleanups.push(server.close);
  expect(await (await fetch(server.url)).text()).toContain('Cockpit');
  expect((await readJson(`${server.url}/api/state?view=all`)).runs).toHaveLength(1);
  expect((await readJson(`${server.url}/api/state?view=session&chat=chat-demo`)).runs).toHaveLength(1);
  expect((await readJson(`${server.url}/api/state?view=session&chat=other`)).runs).toHaveLength(0);
  expect((await readJson(`${server.url}/api/events?run=demo`)).events).toHaveLength(4);
  expect(await (await fetch(`${server.url}/media/ok`)).text()).toBe('image');
  expect((await fetch(`${server.url}/media/outside`)).status).toBe(403);
  expect((await fetch(`${server.url}/media/link`)).status).toBe(403);
  const controller = new AbortController();
  const response = await fetch(`${server.url}/api/stream`, { signal: controller.signal });
  const stream = response.body!.getReader();
  expect(new TextDecoder().decode((await stream.read()).value)).toContain(': connected');
  await mkdir(join(root, '.claude/projects/demo'), { recursive: true });
  await writeFile(join(root, '.claude/projects/demo/new.jsonl'), JSON.stringify({ type: 'user', sessionId: 'new', cwd: root, timestamp: new Date().toISOString(), message: { role: 'user', content: 'Inspect the source' } }) + '\n');
  await observer.pass();
  expect(new TextDecoder().decode((await stream.read()).value)).toContain('event: cockpit');
  controller.abort();
}, 30_000);
it('uses the next free port when the requested port is occupied', async () => {
  const blocker = createServer();
  await new Promise<void>(resolve => blocker.listen(0, '127.0.0.1', resolve));
  cleanups.push(() => new Promise<void>(resolve => blocker.close(() => resolve())));
  const address = blocker.address();
  if (!address || typeof address === 'string') throw new Error('The port is unavailable.');
  const root = await mkdtemp(join(tmpdir(), 'cockpit-port-'));
  cleanups.push(() => rm(root, { recursive: true, force: true }));
  const observer = await createObserver({ home: root, userHome: root });
  const server = await startServer({ observer, port: address.port });
  cleanups.push(server.close);
  expect(new URL(server.url).hostname).toBe('127.0.0.1');
  expect(Number(new URL(server.url).port)).toBeGreaterThan(address.port);
}, 30_000);

async function readJson(url: string): Promise<{ runs: unknown[]; events: unknown[] }> {
  return (await fetch(url)).json() as Promise<{ runs: unknown[]; events: unknown[] }>;
}
