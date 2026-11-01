import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { parsePlan } from '../loop-plan-model/parse-plan.ts';
import { splitPlan, exportPlan } from '../loop-board.mjs';

const script = new URL('../loop-board.mjs', import.meta.url).pathname;
const fixtures = new URL('./fixtures/milestones/', import.meta.url).pathname;
const PLAN = fs.readFileSync(path.join(fixtures, 'plan.md'), 'utf8');
const ARCHIVED = fs.readFileSync(path.join(fixtures, 'archived-m1/plan.md'), 'utf8');

const BOARD_STATUS = { todo: 'todo', 'in-progress': 'in_progress', blocked: 'in_progress', done: 'done' };

// Build a board.json-shaped object from split output, as the coordinator does with task_create.
function boardFromSplit(spec, epicKey = 'EX-1') {
  const issues = [];
  let n = Number(epicKey.split('-')[1]);
  const add = (fields) => {
    const issue = { id: `i_${n}`, key: `EX-${n}`, status: 'todo', blockedBy: [], ...fields };
    n += 1;
    issues.push(issue);
    return issue;
  };
  const epic = add({ title: spec.epic.title, brief: { goal: spec.epic.goal, decisions: spec.epic.decisions, context: spec.epic.context } });
  const parents = new Map();
  for (const m of spec.milestones) {
    const issue = add({ title: m.title, parent: epic.id, status: m.archived ? 'done' : 'todo', brief: { goal: m.goal, context: m.context, acceptance: m.acceptance } });
    parents.set(m.number, issue.id);
  }
  const byPhase = new Map();
  for (const p of spec.phases) {
    const issue = add({
      title: p.title,
      parent: parents.get(p.milestone) ?? epic.id,
      status: BOARD_STATUS[p.status] ?? 'todo',
      brief: { goal: p.goal, context: p.context, acceptance: p.acceptance },
    });
    byPhase.set(p.number, issue);
  }
  for (const p of spec.phases) byPhase.get(p.number).blockedBy = p.dependsOn.map((d) => byPhase.get(d)?.id).filter(Boolean);
  return { issues, byPhase, epic };
}

const comparable = (markdown) => {
  const plan = parsePlan(markdown);
  delete plan.updatedAt;
  return plan;
};

test('split then export keeps the parsed plan of a milestone plan', () => {
  const spec = splitPlan(PLAN);
  assert.equal(spec.planId, 'loop-archive-fixture-2026-10-01-000000');
  assert.equal(spec.phases.length, 9);
  assert.deepEqual(spec.phases.find((p) => p.number === '3').dependsOn, ['1', '9']);
  const board = boardFromSplit(spec);
  const out = exportPlan(board, 'EX-1');
  assert.match(out, /^<!-- loop-plan\nplanId: loop-archive-fixture-2026-10-01-000000\nboard: EX-1\n-->/);
  assert.doesNotMatch(out, /daemon:/);
  assert.deepEqual(comparable(out), comparable(PLAN));
});

test('split then export keeps an archived milestone stub', () => {
  const spec = splitPlan(ARCHIVED);
  const archived = spec.milestones.find((m) => m.number === 1);
  assert.equal(archived.archived, true);
  assert.equal(spec.phases.some((p) => p.milestone === 1), false);
  const out = exportPlan(boardFromSplit(spec), 'EX-1');
  assert.deepEqual(comparable(out), comparable(ARCHIVED));
});

test('board edits reach the plan: criteria, dependencies, new phase and canceled phase', () => {
  const spec = splitPlan(PLAN);
  const board = boardFromSplit(spec);
  const p6 = board.byPhase.get('6');
  p6.brief.acceptance = '- [ ] the API returns 429 over the limit\n- [ ] the limit is configurable';
  p6.blockedBy = [board.byPhase.get('5').id, board.byPhase.get('4').id];
  board.byPhase.get('10').status = 'canceled';
  const extra = splitPlan(PLAN).phases.find((p) => p.number === '6');
  board.issues.push({
    id: 'i_new', key: 'EX-99', status: 'todo', parent: p6.parent, blockedBy: [p6.id],
    title: 'P11 · Document the limits',
    brief: { context: extra.context.replace('phase: 6', 'phase: 11').replace(/order: [0-9.]+/, 'order: 99'), acceptance: '- [ ] the README lists the limits' },
  });
  const plan = comparable(exportPlan(board, 'EX-1'));
  const phase = (n) => plan.phases.find((p) => p.phase === n);
  assert.match(phase('6').doneWhen, /the API returns 429 over the limit.*the limit is configurable/s);
  assert.deepEqual(phase('6').dependsOn.sort(), ['4', '5']);
  assert.equal(phase('10'), undefined);
  assert.equal(phase('11').title, 'Document the limits');
  assert.deepEqual(phase('11').dependsOn, ['6']);
});

test('a Done when list becomes one criterion per item and comes back as a list', () => {
  const plan = PLAN.replace('- **Done when:** the shell renders', '- **Done when:**\n  - the shell renders\n  - the shell has a title');
  const spec = splitPlan(plan);
  const p2 = spec.phases.find((p) => p.number === '2');
  assert.equal(p2.acceptance, '- [ ] the shell renders\n- [ ] the shell has a title');
  const board = boardFromSplit(spec);
  board.byPhase.get('2').brief.acceptance += '\n- [ ] the shell has a footer';
  const out = exportPlan(board, 'EX-1');
  assert.match(out, /- \*\*Done when:\*\*\n {2}- the shell renders\n {2}- the shell has a title\n {2}- the shell has a footer\n- \*\*Estimate:\*\*/);
  assert.deepEqual(comparable(exportPlan(boardFromSplit(spec), 'EX-1')), comparable(plan));
});

test('export with a base keeps the fields that Loop owns', () => {
  const spec = splitPlan(PLAN);
  const board = boardFromSplit(spec);
  for (const issue of board.byPhase.values()) issue.status = 'todo';
  const base = ARCHIVED
    .replace('**Status:** awaiting merge', '**Status:** in progress')
    .replace('- **PR (acme/web):** _none yet_', '- **PR (acme/web):** https://github.com/acme/web/pull/7')
    .replace('- 2026-10-04 — Phase 4 claimed (lane A, codex)', '- 2026-10-05 — Phase 4 merged');
  const out = exportPlan(board, 'EX-1', base);
  const plan = comparable(out);
  assert.match(out, /\*\*Status:\*\* in progress/);
  assert.match(out, /Milestone 1 — Core API and web shell ship `\[archived\]`/);
  assert.equal(plan.phases.find((p) => p.phase === '4').status, 'in-progress');
  assert.equal(plan.milestones.find((m) => m.number === 2).prs['acme/web'], 'https://github.com/acme/web/pull/7');
  assert.match(out, /2026-10-05 — Phase 4 merged/);
  assert.equal(plan.phases.some((p) => p.milestone === 1 && !p.archived), false);
});

test('the CLI exports from a board file and splits a plan file', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'loop-board-'));
  try {
    const spec = splitPlan(PLAN);
    const { issues } = boardFromSplit(spec);
    const boardFile = path.join(dir, 'board.json');
    fs.writeFileSync(boardFile, JSON.stringify({ version: 1, issues, projects: {} }));
    let result = spawnSync(process.execPath, [script, 'export', '--epic', 'EX-1', '--board', boardFile], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(comparable(result.stdout), comparable(PLAN));
    result = spawnSync(process.execPath, [script, 'export', '--epic', 'EX-404', '--board', boardFile], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /EX-404/);
    const planFile = path.join(dir, 'plan.md');
    fs.writeFileSync(planFile, PLAN);
    result = spawnSync(process.execPath, [script, 'split', '--plan', planFile], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(JSON.parse(result.stdout).phases.length, 9);
    result = spawnSync(process.execPath, [script, 'check', '--epic', 'EX-1', '--plan', planFile, '--board', boardFile], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    result = spawnSync(process.execPath, [script, 'keys', '--epic', 'EX-1', '--board', boardFile], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    const keys = JSON.parse(result.stdout);
    assert.equal(keys.epic, 'EX-1');
    assert.equal(keys.milestones['1'], 'EX-2');
    assert.equal(keys.phases['1'], 'EX-5');
    assert.equal(Object.keys(keys.phases).length, 9);
    fs.writeFileSync(planFile, PLAN.replace('the shell renders', 'the shell renders fast'));
    result = spawnSync(process.execPath, [script, 'check', '--epic', 'EX-1', '--plan', planFile, '--board', boardFile], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /phases\.1\.doneWhen/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
