import { mkdtemp, mkdir, copyFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { createObserver } from './index.ts';

const folders: string[] = [];
afterEach(async () => { await Promise.all(folders.map(path => rm(path, { recursive: true, force: true }))); });
it('appends session actions and intents once across observer passes and restarts', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cockpit-observer-'));
  folders.push(root);
  await mkdir(join(root, '.claude/projects/demo'), { recursive: true });
  await copyFile(new URL('../../fixtures/claude-session.jsonl', import.meta.url), join(root, '.claude/projects/demo/session.jsonl'));
  const options = { home: join(root, 'store'), userHome: root, model: async () => { throw new Error('Use rule names.'); } };
  const observer = await createObserver(options);
  const events: string[] = [];
  observer.subscribe(event => events.push(event.type));
  await observer.pass();
  expect(events).toContain('action');
  expect(events).toContain('intent.upsert');
  const actions = observer.store.snapshot().flatMap(run => run.actions);
  expect(actions.length).toBeGreaterThan(0);
  await observer.pass();
  expect(observer.store.snapshot().flatMap(run => run.actions)).toEqual(actions);
  const restarted = await createObserver(options);
  await restarted.pass();
  expect(restarted.store.snapshot().flatMap(run => run.actions)).toEqual(actions);
}, 30_000);

it('keeps later session actions in their Loop phase after a restart', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cockpit-mapping-'));
  folders.push(root);
  const loop = join(root, 'repo/.loop');
  const cwd = join(root, 'repo/worktree');
  await mkdir(loop, { recursive: true });
  await mkdir(cwd, { recursive: true });
  await mkdir(join(root, '.claude/projects/demo'), { recursive: true });
  const { writeFile, appendFile } = await import('node:fs/promises');
  await writeFile(join(loop, 'plan.md'), '# Example\n\n### Phase 1 — Implement the feature `[lane: A]` `[status: in-progress]`\n- **Depends on:** none\n');
  await writeFile(join(loop, 'state.json'), JSON.stringify({ runId: 'example-loop', phases: { '1': { worktree: cwd, lane: 'A', status: 'running' } } }));
  const path = join(root, '.claude/projects/demo/session.jsonl');
  const timestamp = new Date().toISOString();
  await writeFile(path, JSON.stringify({ type: 'user', sessionId: 'runner', cwd, timestamp, message: { role: 'user', content: 'Implement the feature' } }) + '\n');
  const options = { home: join(root, 'store'), userHome: root, model: async () => { throw new Error('Use rule names.'); } };
  const observer = await createObserver(options);
  await observer.pass();
  const restarted = await createObserver(options);
  await appendFile(path, JSON.stringify({ type: 'assistant', timestamp, message: { role: 'assistant', content: [{ type: 'tool_use', id: 'read-later', name: 'Read', input: { file_path: 'source.ts' } }] } }) + '\n');
  await restarted.pass();
  expect(restarted.store.getRun('example-loop')?.actions).toMatchObject([{ id: 'read-later', phaseId: '1', laneId: 'A' }]);
  expect(restarted.store.getRun('example-loop')?.meta.sessions).toMatchObject([{ id: 'runner' }]);
  expect(restarted.store.snapshot()).toHaveLength(1);
}, 30_000);

it('prefers the live runtime over a stale worktree copy of the same plan', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cockpit-sources-'));
  folders.push(root);
  const { writeFile } = await import('node:fs/promises');
  const runtime = join(root, 'runtime/.loop');
  const stale = join(root, 'worktree/.loop');
  const plan = '<!-- loop-plan\nplanId: shared-loop\n-->\n# Example\n\n### Phase 1 — Implement the feature `[lane: A]` `[status: in-progress]`\n- **Depends on:** none\n';
  for (const dir of [runtime, stale]) { await mkdir(dir, { recursive: true }); await writeFile(join(dir, 'plan.md'), plan); }
  await writeFile(join(runtime, 'state.json'), JSON.stringify({ runId: 'shared-loop', phases: {} }));
  await writeFile(join(runtime, 'lock'), String(process.pid));
  const observer = await createObserver({ home: join(root, 'store'), userHome: root, loopDirectories: [runtime, stale], model: async () => '{}' });
  await observer.pass();
  expect(observer.store.getRun('shared-loop')?.meta.status).toBe('active');
}, 30_000);
