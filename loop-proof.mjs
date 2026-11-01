#!/usr/bin/env node
// Loop proof: publish the proof of each finished phase to one folder, with clear names.
//
// Usage:
//   loop-proof.mjs publish --dir .loop --out <folder>
//     For each done or merged phase, copy its runner proof (runs/*/proof/*) and write a proof card.
//     Names: M<m>-P<NN>-<phase title>--<proof file>, and M<m>-P<NN>-<phase title>--00-proof-card.svg.
//     The card shows the Done when, Verify, the diff, the time against the estimate and the drift.
//     A file that exists is not written again. Print the new files as a JSON array.
// Requires Node >= 22.18 (the plan parser is TypeScript).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkpoint, esc } from './loop-checkpoint.mjs';

/** Publish the proof of every finished phase. Return the paths that this call wrote. */
export function publishProof({ dir, out, now = new Date().toISOString() }) {
  const cp = checkpoint({ dir, all: true, now });
  fs.mkdirSync(out, { recursive: true });
  const written = [];
  for (const p of cp.phases.filter((row) => row.status === 'done' && row.attempts > 0)) {
    const prefix = `M${p.milestone}-P${p.phase.padStart(2, '0')}-${slug(p.title)}`;
    const card = path.join(out, `${prefix}--00-proof-card.svg`);
    if (!fs.existsSync(card)) {
      fs.writeFileSync(card, renderCard(p));
      written.push(card);
    }
    for (const m of cp.media.filter((media) => media.phase === p.phase)) {
      const target = path.join(out, `${prefix}--${path.basename(m.path)}`);
      if (fs.existsSync(target)) continue;
      fs.copyFileSync(m.path, target);
      written.push(target);
    }
  }
  return written;
}

/** One SVG card that proves the work of a phase without a screen: plan, result, diff and drift. */
function renderCard(p) {
  const lines = [];
  const add = (value, attrs = 'font-size="13" fill="#1f2933"') => lines.push({ value, attrs });
  add(`M${p.milestone} · P${p.phase} · ${p.title}`, 'font-size="18" font-weight="700" fill="#1f2933"');
  add([p.status, p.engine, p.attempts === 1 ? '1 attempt' : `${p.attempts} attempts`].filter(Boolean).join(' · '), 'font-size="12" fill="#5b6470"');
  add('');
  add('Done when', 'font-size="13" font-weight="700" fill="#1f2933"');
  for (const line of wrap(p.doneWhen ?? 'not stated', 100)) add(line);
  add('');
  const verify = p.verify?.passed ? 'Verify passed' : 'Verify not recorded';
  add([verify, p.verify?.summary].filter(Boolean).join(' — '), `font-size="13" font-weight="700" fill="${p.verify?.passed ? '#2f9e5b' : '#d9822b'}"`);
  if (p.minutes !== null) add(`${p.minutes} min${p.estimate ? ` (plan ${p.estimate.min}–${p.estimate.max})` : ''}`);
  if (p.diff) add(`+${p.diff.insertions} −${p.diff.deletions} in ${p.diff.files} files`);
  for (const f of p.files.slice(0, 12)) add(`${f.planned ? '  ' : '⚠ '}${f.path}`, `font-size="12" fill="${f.planned ? '#1f2933' : '#d64545'}"`);
  if (p.files.length > 12) add(`  … ${p.files.length - 12} more files`, 'font-size="12" fill="#5b6470"');
  add('');
  if (p.drift.length) {
    add('Drift from the plan', 'font-size="13" font-weight="700" fill="#d64545"');
    for (const d of p.drift) for (const line of wrap(`${d.kind}: ${d.detail}`, 100)) add(line, 'font-size="12" fill="#d64545"');
  } else {
    add('On plan', 'font-size="13" font-weight="700" fill="#2f9e5b"');
  }
  const height = 40 + lines.length * 20;
  const body = lines.map((l, k) => `<text x="24" y="${40 + k * 20}" ${l.attrs}>${esc(l.value)}</text>`).join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" width="900" height="${height}" viewBox="0 0 900 ${height}" font-family="-apple-system, Helvetica, Arial, sans-serif">`
    + `<rect width="100%" height="100%" rx="12" fill="#ffffff" stroke="${p.drift.length ? '#d64545' : '#2f9e5b'}" stroke-width="4"/>${body}</svg>\n`;
}

function slug(title) {
  const words = title.toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim().split(' ');
  let out = '';
  for (const word of words) {
    if ((out ? out.length + 1 : 0) + word.length > 48) break;
    out = out ? `${out}-${word}` : word;
  }
  return out || 'phase';
}

function wrap(textValue, width) {
  const out = [];
  let line = '';
  for (const word of String(textValue).split(/\s+/)) {
    if (line && line.length + word.length + 1 > width) {
      out.push(line);
      line = word;
    } else {
      line = line ? `${line} ${word}` : word;
    }
  }
  if (line) out.push(line);
  return out;
}

function main(argv) {
  const [command, ...rest] = argv;
  const args = {};
  for (let k = 0; k < rest.length; k += 2) args[rest[k].replace(/^--/, '')] = rest[k + 1];
  if (command !== 'publish' || !args.dir || !args.out) return fail('use publish --dir <loop dir> --out <folder>');
  const dir = path.resolve(args.dir);
  if (!fs.existsSync(path.join(dir, 'plan.md'))) return fail(`${dir} has no plan.md`);
  process.stdout.write(`${JSON.stringify(publishProof({ dir, out: path.resolve(args.out) }), null, 2)}\n`);
  return 0;
}

function fail(message) {
  console.error(`loop-proof: ${message}`);
  return 1;
}

if (process.argv[1] && fs.realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    process.exitCode = fail(error.message);
  }
}
