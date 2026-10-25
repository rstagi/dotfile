import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const script = new URL('../loop-plan-tier.mjs', import.meta.url);

test('tier edits a pending review and refuses a started review without changing the file', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'loop-tier-'));
  try {
    const plan = path.join(dir, 'plan.md');
    fs.writeFileSync(plan, '# Plan\n## Phases\n### Phase 9 — Review `[kind: pr-review]` `[status: todo]` `[review: standard]` `[rounds: 2]` `[inner: 3]`\n');
    const run = (...extra) => spawnSync(process.execPath, [script.pathname, '--plan', plan, '--phase', '9', '--tier', 'max', ...extra], {encoding:'utf8'});
    let result = run('--rounds', '4');
    assert.equal(result.status, 0, result.stderr);
    let text = fs.readFileSync(plan, 'utf8');
    assert.match(text, /\[review: max\]/);
    assert.match(text, /\[rounds: 4\]/);
    assert.doesNotMatch(text, /\[inner:/);
    assert.equal(JSON.parse(result.stdout).text, text);
    result = run('--rounds', '0');
    assert.equal(result.status, 1);
    assert.equal(fs.readFileSync(plan, 'utf8'), text);
    text = text.replace('[status: todo]', '[status: in-progress]');
    fs.writeFileSync(plan, text);
    result = run();
    assert.equal(result.status, 1);
    assert.match(result.stderr, /already started/);
    assert.equal(fs.readFileSync(plan, 'utf8'), text);
  } finally {
    fs.rmSync(dir, {recursive:true, force:true});
  }
});
