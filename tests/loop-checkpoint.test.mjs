import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { checkpoint } from '../loop-checkpoint.mjs';

const script = new URL('../loop-checkpoint.mjs', import.meta.url).pathname;
const PLAN = fs.readFileSync(new URL('./fixtures/milestones/plan.md', import.meta.url), 'utf8');

// A loop dir in milestone 2: phase 4 merged after two attempts, phase 10 runs, phase 5 waits.
function loopDir() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'loop-cp-'));
  fs.writeFileSync(path.join(dir, 'plan.md'), PLAN);
  const phase = (status, slug, attempt) => ({ status, slug, attempt, lane: 'A' });
  fs.writeFileSync(path.join(dir, 'state.json'), JSON.stringify({
    version: 2, milestone: 2,
    phases: {
      1: phase('done', 'p1-core-api', 1), 2: phase('done', 'p2-web-shell', 1), 9: phase('done', 'p9-warning', 1), 3: phase('done', 'p3-review', 1),
      4: phase('merged', 'p4-call-api', 2), 10: phase('running', 'p10-banner', 1), 5: phase('todo', 'p5-review', 0),
      6: phase('todo', 'p6-limits', 0), 7: phase('todo', 'p7-review', 0),
    },
  }));
  const run = (name, start, end, files = []) => {
    const d = path.join(dir, 'runs', name);
    fs.mkdirSync(path.join(d, 'proof'), { recursive: true });
    fs.writeFileSync(path.join(d, 'prompt.md'), 'x');
    fs.utimesSync(path.join(d, 'prompt.md'), new Date(start), new Date(start));
    if (end) {
      fs.writeFileSync(path.join(d, 'meta.json'), '{}');
      fs.utimesSync(path.join(d, 'meta.json'), new Date(end), new Date(end));
    }
    for (const [file, at] of files) {
      fs.writeFileSync(path.join(d, 'proof', file), 'img');
      fs.utimesSync(path.join(d, 'proof', file), new Date(at), new Date(at));
    }
  };
  run('p4-call-api-a1', '2026-11-01T10:00:00Z', '2026-11-01T10:20:00Z', [['01-old.png', '2026-11-01T10:19:00Z']]);
  run('p4-call-api-a2', '2026-11-01T10:21:00Z', '2026-11-01T10:40:00Z', [['02-health.png', '2026-11-01T10:39:00Z'], ['03-flow.gif', '2026-11-01T10:39:30Z']]);
  run('p10-banner-a1', '2026-11-01T10:41:00Z', null);
  return dir;
}

test('the checkpoint gives stats, a timeline, a graph of the current milestone and new media', () => {
  const dir = loopDir();
  try {
    const cp = checkpoint({ dir, since: '2026-11-01T10:30:00Z', now: '2026-11-01T11:00:00Z' });
    assert.equal(cp.milestone, 2);
    assert.deepEqual(cp.stats.plan, { done: 5, total: 9 });
    assert.deepEqual(cp.stats.milestone, { done: 1, total: 3 });
    assert.deepEqual(cp.stats.running, ['10']);
    assert.equal(cp.stats.attempts, 3);
    assert.equal(cp.stats.retries, 1);

    const p4 = cp.timeline.find((r) => r.phase === '4');
    assert.deepEqual([p4.start, p4.end, p4.minutes, p4.attempts, p4.status], ['2026-11-01T10:00:00.000Z', '2026-11-01T10:40:00.000Z', 40, 2, 'done']);
    const p10 = cp.timeline.find((r) => r.phase === '10');
    assert.deepEqual([p10.end, p10.minutes, p10.status], [null, 19, 'active']);
    assert.equal(cp.timeline.some((r) => r.phase === '5'), false);

    const ids = cp.graph.nodes.map((n) => n.id);
    assert.ok(ids.includes('p4') && ids.includes('p10') && ids.includes('p5'));
    assert.ok(ids.includes('m1') && ids.includes('m3'));
    assert.equal(ids.includes('p1'), false);
    assert.equal(cp.graph.nodes.find((n) => n.id === 'p10').status, 'active');
    assert.equal(cp.graph.nodes.find((n) => n.id === 'p10').emphasis, true);
    assert.equal(cp.graph.nodes.find((n) => n.id === 'm1').status, 'done');
    assert.deepEqual(cp.graph.edges.find((e) => e.to === 'p10'), { from: 'p4', to: 'p10', kind: 'depends' });
    assert.equal(cp.graph.edges.some((e) => e.from === 'p3'), false);

    assert.deepEqual(cp.media.map((m) => [m.phase, path.basename(m.path), m.kind]), [['4', '02-health.png', 'image'], ['4', '03-flow.gif', 'image']]);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('the CLI prints the checkpoint as JSON and rejects a missing loop dir', () => {
  const dir = loopDir();
  try {
    let result = spawnSync(process.execPath, [script, '--dir', dir, '--all'], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    const cp = JSON.parse(result.stdout);
    assert.equal(cp.graph.nodes.filter((n) => n.type === 'task').length, 9);
    assert.equal(cp.media.length, 3);
    result = spawnSync(process.execPath, [script, '--dir', path.join(dir, 'missing')], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    fs.rmSync(path.join(dir, 'state.json'));
    fs.rmSync(path.join(dir, 'runs'), { recursive: true });
    result = spawnSync(process.execPath, [script, '--dir', dir, '--all'], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    const before = JSON.parse(result.stdout);
    assert.equal(before.milestone, 1);
    assert.deepEqual(before.stats.plan, { done: 4, total: 9 });
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
