import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync, execFileSync } from 'node:child_process';
import { checkpoint, renderSheet } from '../loop-checkpoint.mjs';
import { publishProof } from '../loop-proof.mjs';

const proofScript = new URL('../loop-proof.mjs', import.meta.url).pathname;
const checkpointScript = new URL('../loop-checkpoint.mjs', import.meta.url).pathname;
const PLAN = fs.readFileSync(new URL('./fixtures/milestones/plan.md', import.meta.url), 'utf8')
  .replace('- **Done when:** the shell shows the API health', '- **Touches:** folder `web/src/api/`\n- **Done when:** the shell shows the API health');
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==', 'base64');

// Milestone 2 runs. Phase 4 merged after a retry, took 40 min (estimate 10–20), and changed one
// file outside its Touches folder. Phase 10 runs. A git repo holds the phase 4 change.
function loopDir() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'loop-proof-'));
  const dir = path.join(root, '.loop');
  const repo = path.join(root, 'repo');
  fs.mkdirSync(dir);
  fs.mkdirSync(path.join(repo, 'web', 'src', 'api'), { recursive: true });
  const git = (...args) => execFileSync('git', ['-C', repo, ...args], { encoding: 'utf8' }).trim();
  git('init', '-q');
  git('config', 'user.email', 't@example.com');
  git('config', 'user.name', 'T');
  fs.writeFileSync(path.join(repo, 'web', 'package.json'), '{}\n');
  git('add', '.');
  git('commit', '-q', '-m', 'base');
  const before = git('rev-parse', 'HEAD');
  fs.writeFileSync(path.join(repo, 'web', 'src', 'api', 'health.ts'), 'export const ok = 1;\nexport const two = 2;\n');
  fs.writeFileSync(path.join(repo, 'web', 'package.json'), '{"x":1}\n');
  git('add', '.');
  git('commit', '-q', '-m', 'phase 4');
  const after = git('rev-parse', 'HEAD');

  fs.writeFileSync(path.join(dir, 'plan.md'), PLAN);
  const phase = (status, slug, attempt, extra = {}) => ({ status, slug, attempt, lane: 'A', ...extra });
  fs.writeFileSync(path.join(dir, 'state.json'), JSON.stringify({
    version: 2, milestone: 2,
    phases: {
      1: phase('done', 'p1-core-api', 1), 2: phase('done', 'p2-web-shell', 1), 9: phase('done', 'p9-warning', 1), 3: phase('done', 'p3-review', 1),
      4: phase('merged', 'p4-call-api', 2, { worktree: repo }), 10: phase('running', 'p10-banner', 1), 5: phase('todo', 'p5-review', 0),
      6: phase('todo', 'p6-limits', 0), 7: phase('todo', 'p7-review', 0),
    },
  }));
  fs.writeFileSync(path.join(dir, 'events.jsonl'), `${JSON.stringify({ ts: '2026-11-01T10:20:00Z', event: 'phase.verify-fail', phase: '4', detail: 'a1' })}\n`);
  fs.writeFileSync(path.join(dir, 'emitted.jsonl'), `${JSON.stringify({ ts: '2026-11-01T10:39:00Z', runId: 'r', action: 'event', body: { event: 'merge.conflict', phase: '4' } })}\n`);
  const run = (name, start, end, meta, files = []) => {
    const d = path.join(dir, 'runs', name);
    fs.mkdirSync(path.join(d, 'proof'), { recursive: true });
    fs.writeFileSync(path.join(d, 'prompt.md'), 'x');
    fs.utimesSync(path.join(d, 'prompt.md'), new Date(start), new Date(start));
    if (end) {
      fs.writeFileSync(path.join(d, 'meta.json'), JSON.stringify(meta));
      fs.utimesSync(path.join(d, 'meta.json'), new Date(end), new Date(end));
    }
    for (const file of files) fs.writeFileSync(path.join(d, 'proof', file), PNG);
    return d;
  };
  run('p4-call-api-a1', '2026-11-01T10:00:00Z', '2026-11-01T10:20:00Z', { engine: 'codex', headBefore: before, headAfter: before });
  const last = run('p4-call-api-a2', '2026-11-01T10:21:00Z', '2026-11-01T10:40:00Z', { engine: 'claude', model: 'claude-opus-5-5', headBefore: before, headAfter: after }, ['01-shell-shows-health.png', '02-health-flow.gif']);
  fs.writeFileSync(path.join(last, 'verified.json'), '{"tree":"t","verify":"npm test"}');
  fs.writeFileSync(path.join(last, 'verify.log'), 'running\n# tests 12\n# pass 12\n# fail 0\n');
  run('p10-banner-a1', '2026-11-01T10:41:00Z', null, {});
  return { root, dir };
}

test('the checkpoint lists each phase with its estimate, its attempts and its drift from the plan', () => {
  const { root, dir } = loopDir();
  try {
    const cp = checkpoint({ dir, now: '2026-11-01T11:00:00Z' });
    const p4 = cp.phases.find((p) => p.phase === '4');
    assert.deepEqual(p4.estimate, { min: 10, max: 20 });
    assert.equal(p4.minutes, 40);
    assert.equal(p4.attempts, 2);
    assert.deepEqual(p4.diff, { files: 2, insertions: 3, deletions: 1 });
    assert.deepEqual(p4.drift.map((d) => d.kind).sort(), ['merge-conflict', 'over-time', 'retried', 'unplanned-files', 'verify-fail']);
    assert.match(p4.drift.find((d) => d.kind === 'unplanned-files').detail, /web\/package\.json/);
    const p10 = cp.phases.find((p) => p.phase === '10');
    assert.deepEqual(p10.drift, []);
    assert.equal(cp.stats.onPlan, 4);
    assert.equal(cp.stats.drifted, 1);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('the checkpoint graph shows the whole plan, the drift, the proof images, the checkpoints and a walkthrough', () => {
  const { root, dir } = loopDir();
  try {
    fs.writeFileSync(path.join(dir, 'checkpoints.jsonl'), [
      { n: 1, ts: '2026-11-01T10:30:00Z', done: 4, total: 9 },
      { n: 2, ts: '2026-11-01T10:50:00Z', done: 5, total: 9 },
    ].map((c) => JSON.stringify(c)).join('\n'));
    const { graph } = checkpoint({ dir, all: true, now: '2026-11-01T11:00:00Z' });
    const node = (id) => graph.nodes.find((n) => n.id === id);
    assert.equal(node('p4').status, 'done');
    assert.match(node('p4').sublabel, /⚠ 5 drift/);
    assert.match(node('p4').sublabel, /40m \/ 20m/);
    assert.match(node('p4').details, /over-time: 40 min \(plan 10–20\)/);
    assert.equal(node('p4').emphasis, true);
    assert.match(node('p1').sublabel, /on plan|lane/);
    const image = graph.nodes.find((n) => n.type === 'image');
    assert.match(image.image, /02-health-flow\.gif$/);
    assert.ok(graph.edges.some((e) => e.from === 'p4' && e.to === image.id && e.label === 'proof'));
    assert.deepEqual(['cp1', 'cp2'].map((id) => node(id)?.type), ['milestone', 'milestone']);
    assert.match(node('cp2').sublabel, /5\/9 done/);
    assert.ok(graph.steps.length >= 3 && graph.steps.length <= 7);
    assert.ok(graph.steps.some((s) => s.focus.includes('p4') && /drift/i.test(s.title)));
    const ids = new Set(graph.nodes.map((n) => n.id));
    for (const e of graph.edges) assert.ok(ids.has(e.from) && ids.has(e.to), `${e.from} -> ${e.to}`);
    for (const s of graph.steps) for (const f of s.focus) assert.ok(ids.has(f), f);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('the checkpoint sheet is one SVG with the whole plan, the drift and the proof images', () => {
  const { root, dir } = loopDir();
  try {
    const cp = checkpoint({ dir, all: true, now: '2026-11-01T11:00:00Z' });
    const svg = renderSheet(cp, { title: 'Archive & "fixture"', label: 'CP02' });
    assert.match(svg, /^<svg [^>]*xmlns="http:\/\/www.w3.org\/2000\/svg"/);
    assert.match(svg, /Archive &amp; &quot;fixture&quot;/);
    for (const n of ['P1', 'P4', 'P10', 'P7']) assert.match(svg, new RegExp(`>${n}<`));
    assert.match(svg, /5\/9 phases done/);
    assert.match(svg, /over-time/);
    assert.match(svg, /data:image\/png;base64,/);
    assert.match(svg, /01-shell-shows-health\.png/);
    const out = path.join(root, 'sheet.svg');
    const result = spawnSync(process.execPath, [checkpointScript, '--dir', dir, '--all', '--sheet', out, '--label', 'CP02'], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.match(fs.readFileSync(out, 'utf8'), /^<svg /);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('publish copies the proof of each merged phase with clear names, adds a proof card and runs one time', () => {
  const { root, dir } = loopDir();
  try {
    const out = path.join(root, 'scratch', 'proof');
    const first = publishProof({ dir, out, now: '2026-11-01T11:00:00Z' });
    const names = first.map((f) => path.basename(f)).sort();
    assert.deepEqual(names, [
      'M2-P04-call-the-api-from-the-web-shell--00-proof-card.svg',
      'M2-P04-call-the-api-from-the-web-shell--01-shell-shows-health.png',
      'M2-P04-call-the-api-from-the-web-shell--02-health-flow.gif',
    ]);
    const card = fs.readFileSync(path.join(out, names[0]), 'utf8');
    assert.match(card, /Call the API from the web shell/);
    assert.match(card, /the shell shows the API health/);
    assert.match(card, /Verify passed/);
    assert.match(card, /# pass 12/);
    assert.match(card, /\+3 −1 in 2 files/);
    assert.match(card, /web\/package\.json/);
    assert.match(card, /40 min \(plan 10–20\)/);
    assert.deepEqual(publishProof({ dir, out, now: '2026-11-01T11:00:00Z' }), []);
    const result = spawnSync(process.execPath, [proofScript, 'publish', '--dir', dir, '--out', path.join(root, 'other')], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(JSON.parse(result.stdout).length, 3);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
