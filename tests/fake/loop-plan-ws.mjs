#!/usr/bin/env node
// A private plan store for the loop-plan CLI contract tests.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';

const args = process.argv.slice(2);
const root = process.env.FAKE_PLAN_WS;
const file = path.join(root, 'store.json');
const flag = name => args[args.indexOf(name) + 1];
const fail = message => { console.error(message); process.exit(1); };
const store = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file)) : [];
fs.appendFileSync(path.join(root, 'calls.jsonl'), JSON.stringify(args) + '\n');
if (process.env.FAKE_PLAN_WS_FAIL === args.slice(0, 2).join(' ')) fail('ws: fixture failure');

if (args[0] === 'loop' && args[1] === 'create') {
  const dir = fs.realpathSync(flag('--dir'));
  let row = store.find(row => row.loop_dir === dir);
  if (!row) {
    const text = fs.readFileSync(path.join(dir, 'plan.md'), 'utf8');
    const plan_id = args.includes('--plan-id') ? flag('--plan-id') : text.match(/^planId:\s*(.+)$/m)?.[1];
    row = { id: `session-${store.length + 1}`, plan_id, loop_dir: dir, engine: null, status: 'planned', plan: null };
    store.push(row);
  }
  console.log(args.includes('--json') ? JSON.stringify(row) : row.id);
} else if (args[0] === 'loop' && args[1] === 'list') {
  console.log(JSON.stringify(store));
} else if (args[0] === 'loop' && args[1] === 'plan') {
  const row = store.find(row => row.id === args[2] || row.plan_id === args[2]);
  if (!row?.plan) fail('ws: no plan');
  console.log(JSON.stringify({ ...row.plan, plan_id: row.plan_id, engine: row.engine }));
} else if (args[0] === 'event') {
  const row = store.find(row => row.id === flag('--target') || row.plan_id === flag('--target'));
  if (!row) fail('ws: unknown loop');
  const body = JSON.parse(flag('--body'));
  if (args[1] === 'loop.plan') {
    if (flag('--source') !== 'tool') fail('ws: invalid source');
    if (createHash('sha256').update(body.text).digest('hex') !== body.sha) fail('ws: invalid SHA');
    if (row.plan?.sha !== body.sha) {
      row.plan = body;
      row.engine = body.text.match(/^engine:\s*(.+)$/m)?.[1] ?? null;
      row.appends = (row.appends ?? 0) + 1;
    }
  } else if (args[1] === 'loop.note') {
    (row.notes ??= []).push(body);
  } else fail('ws: invalid event');
} else fail('ws: unsupported command');
fs.writeFileSync(file, JSON.stringify(store));
