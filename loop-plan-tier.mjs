#!/usr/bin/env node
// Change the tier of a review phase before it starts.
import fs from 'node:fs';
import crypto from 'node:crypto';

const options = new Map();
const args = process.argv.slice(2);
try {
  for (let i = 0; i < args.length; i += 2) {
    if (!['--plan', '--phase', '--tier', '--rounds'].includes(args[i]) || !args[i + 1] || options.has(args[i])) {
      throw new Error('Use --plan <file> --phase <N> --tier <standard|max> [--rounds <1..5>].');
    }
    options.set(args[i], args[i + 1]);
  }
  const file = options.get('--plan');
  const phase = options.get('--phase');
  const tier = options.get('--tier');
  const rounds = options.get('--rounds');
  if (!file || !/^[1-9]\d*$/.test(phase ?? '') || !['standard', 'max'].includes(tier) || (rounds !== undefined && !/^[1-5]$/.test(rounds))) {
    throw new Error('Use --plan <file> --phase <N> --tier <standard|max> [--rounds <1..5>].');
  }
  const lines = fs.readFileSync(file, 'utf8').split('\n');
  const index = lines.findIndex(line => new RegExp(`^#{2,4} Phase ${phase}\\b`).test(line));
  if (index < 0 || !/\[kind:\s*pr-review\s*\]/i.test(lines[index])) {
    throw new Error('The phase is not a review phase.');
  }
  const status = lines[index].match(/\[status:\s*([^\]]+?)\s*\]/i)?.[1] ?? 'todo';
  if (status !== 'todo') throw new Error('The review phase already started.');
  lines[index] = `${lines[index].replace(/\s*`?\[(review|rounds|inner):[^\]]*\]`?/gi, '')} \`[review: ${tier}]\`${rounds ? ` \`[rounds: ${rounds}]\`` : ''}`;
  const text = lines.join('\n');
  const temporary = `${file}.${crypto.randomUUID()}.tmp`;
  try {
    fs.writeFileSync(temporary, text, {flag:'wx'});
    fs.renameSync(temporary, file);
  } finally {
    fs.rmSync(temporary, {force:true});
  }
  console.log(JSON.stringify({text, sha:crypto.createHash('sha256').update(text).digest('hex')}));
} catch (error) {
  console.error(`loop-plan-tier: ${error.message}`);
  process.exit(1);
}
