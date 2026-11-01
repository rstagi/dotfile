#!/usr/bin/env node
// Loop progress checkpoint: one artifact set from a loop directory, for any client to render.
//
// Usage:
//   loop-checkpoint.mjs --dir .loop [--since <ISO time>] [--all] [--sheet <file.svg> [--label <text>]]
//     Print JSON: {name, milestone, milestones, stats, phases, timeline, graph, media}.
//     stats: phases done in the plan and in the current milestone, running phases, attempts, retries,
//     onPlan (done phases with no drift) and drifted (phases with drift).
//     phases: one row per phase: status, attempts, minutes, the plan estimate, the diff, Verify and its
//     drift from the plan (over-time, retried, unplanned-files, verify-fail, merge-conflict, hil,
//     escalated).
//     timeline: one row per phase with an attempt: start, end (null while it runs), minutes, status.
//     graph: a beluga-graph object. It shows the current milestone, or every phase with --all.
//     media: proof files (runs/*/proof/*) changed after --since, oldest first.
//     --sheet writes one SVG: the whole plan by milestone, the drift list and the proof images.
//     Without state.json (before init), the statuses come from plan.md and milestone 1 is current.
//     Exit 1 when the loop directory has no plan.md.
// Requires Node >= 22.18 (the plan parser is TypeScript).
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { parsePlan } from './loop-plan-model/parse-plan.ts';

const DONE = new Set(['done', 'merged']);
const RUNNING = new Set(['running', 'in-progress', 'spawned']);
const GRAPH_STATUS = { done: 'done', merged: 'done', running: 'active', 'in-progress': 'active', spawned: 'active', blocked: 'blocked', failed: 'failed', paused: 'idle', 'review-ready': 'review' };
const MEDIA_KIND = { '.png': 'image', '.jpg': 'image', '.jpeg': 'image', '.gif': 'image', '.webp': 'image', '.webm': 'video', '.mp4': 'video', '.mov': 'video' };

/** Build the checkpoint of one loop directory. */
export function checkpoint({ dir, since = null, all = false, now = new Date().toISOString() }) {
  const markdown = fs.readFileSync(path.join(dir, 'plan.md'), 'utf8');
  const plan = parsePlan(markdown);
  const stateFile = path.join(dir, 'state.json');
  const state = fs.existsSync(stateFile) ? JSON.parse(fs.readFileSync(stateFile, 'utf8')) : { milestone: 1, phases: {} };
  const milestone = Number(state.milestone ?? 1);
  const phases = plan.phases.map((p) => ({ ...p, state: state.phases?.[p.phase] ?? {} }));
  const statusOf = (p) => p.state.status ?? p.status;
  const runs = readRuns(dir, phases);
  const blocks = phaseBlocks(markdown);
  const events = readEvents(dir);

  const rows = phases.map((p) => phaseRow(p, statusOf(p), runs, blocks.get(p.phase) ?? '', events, state, now));
  const current = phases.filter((p) => p.milestone === milestone);
  const attempts = runs.length;
  const started = new Set(runs.map((r) => r.phase));
  const stats = {
    plan: { done: phases.filter((p) => DONE.has(statusOf(p))).length, total: phases.length },
    milestone: { done: current.filter((p) => DONE.has(statusOf(p))).length, total: current.length },
    running: phases.filter((p) => RUNNING.has(statusOf(p))).map((p) => p.phase),
    blocked: phases.filter((p) => statusOf(p) === 'blocked').map((p) => p.phase),
    attempts,
    retries: attempts - started.size,
    onPlan: rows.filter((r) => r.status === 'done' && !r.drift.length).length,
    drifted: rows.filter((r) => r.drift.length).length,
  };

  const timeline = rows.filter((r) => r.start).map((r) => ({
    phase: r.phase, title: r.title, milestone: r.milestone, status: r.status,
    attempts: r.attempts, start: r.start, end: r.end, minutes: r.minutes,
  }));

  const sinceTime = since ? new Date(since) : null;
  const media = runs.flatMap((r) => r.media)
    .filter((m) => !sinceTime || m.mtime > sinceTime)
    .sort((a, b) => a.mtime - b.mtime)
    .map((m) => ({ phase: m.phase, path: m.path, kind: m.kind, mtime: m.mtime.toISOString() }));

  return {
    name: plan.name,
    generatedAt: new Date(now).toISOString(),
    milestone,
    milestones: plan.milestones.map((m) => ({ number: m.number, title: m.title })),
    stats,
    phases: rows,
    timeline,
    graph: buildGraph(plan, rows, milestone, all, stats, media, readHistory(dir)),
    media,
  };
}

/** One phase: what the plan said, what happened and where it drifted. */
function phaseRow(p, rawStatus, runs, block, events, state, now) {
  const status = GRAPH_STATUS[rawStatus] ?? 'todo';
  const mine = runs.filter((r) => r.phase === p.phase).sort((a, b) => a.start - b.start);
  const last = mine.at(-1);
  const running = RUNNING.has(rawStatus);
  const start = mine[0]?.start ?? null;
  const end = !last || running || !last.end ? null : last.end;
  const minutes = start ? Math.round(((end ?? new Date(now)) - start) / 60000) : null;
  const estimateMatch = block.match(/\*\*Estimate:\*\*\s*(\d+)\s*[–-]\s*(\d+)\s*min/);
  const estimate = estimateMatch ? { min: Number(estimateMatch[1]), max: Number(estimateMatch[2]) } : null;
  const changes = mine.length ? readDiff(mine, p, state) : null;
  const touches = touchedPaths(block);
  const unplanned = changes && touches.length ? changes.paths.filter((f) => !touches.some((t) => f === t || f.startsWith(t.endsWith('/') ? t : `${t}/`))) : [];
  const verify = last ? { passed: last.verified, summary: last.verifySummary } : null;

  const drift = [];
  if (estimate && minutes !== null && minutes > estimate.max) drift.push({ kind: 'over-time', detail: `${minutes} min (plan ${estimate.min}–${estimate.max})` });
  if (mine.length > 1) drift.push({ kind: 'retried', detail: `${mine.length} attempts` });
  if (unplanned.length) drift.push({ kind: 'unplanned-files', detail: unplanned.join(', ') });
  const count = (name) => events.filter((e) => e.event === name && String(e.phase) === p.phase).length;
  const verifyFails = count('phase.verify-fail') + count('merge.verify-fail');
  if (verifyFails) drift.push({ kind: 'verify-fail', detail: `${verifyFails} failed Verify` });
  if (count('merge.conflict')) drift.push({ kind: 'merge-conflict', detail: `${count('merge.conflict')} merge conflict` });
  if (count('hil.raise')) drift.push({ kind: 'hil', detail: `${count('hil.raise')} question to the human` });
  if (count('phase.escalate')) drift.push({ kind: 'escalated', detail: `${count('phase.escalate')} escalation` });

  return {
    phase: p.phase, title: p.title, milestone: p.milestone, lane: p.lane, kind: p.kind, dependsOn: p.dependsOn,
    status, rawStatus, doneWhen: p.doneWhen, attempts: mine.length,
    start: start ? start.toISOString() : null, end: end ? end.toISOString() : null, minutes, estimate,
    engine: last?.meta?.model ?? last?.meta?.engine ?? null,
    diff: changes ? { files: changes.paths.length, insertions: changes.insertions, deletions: changes.deletions } : null,
    files: changes ? changes.paths.map((f) => ({ path: f, planned: !unplanned.includes(f) })) : [],
    verify, drift,
  };
}

/** The raw `### Phase N` block of each phase, by number. */
function phaseBlocks(markdown) {
  const blocks = new Map();
  for (const part of markdown.split(/^(?=### Phase \d+ )/m).slice(1)) blocks.set(part.match(/^### Phase (\d+)/)[1], part);
  return blocks;
}

/** The backticked paths of the Touches bullet and its sub-bullets. */
function touchedPaths(block) {
  const lines = block.split('\n');
  const k = lines.findIndex((l) => /^- \*\*Touches:\*\*/.test(l));
  if (k < 0) return [];
  const own = [lines[k]];
  for (let i = k + 1; i < lines.length && /^\s+- /.test(lines[i]); i += 1) own.push(lines[i]);
  return [...own.join('\n').matchAll(/`([^`\s]+\/[^`\s]*)`/g)].map((m) => m[1].replace(/\*.*$/, ''));
}

/** The diff from the first attempt base to the last attempt head, or null without git facts. */
function readDiff(mine, p, state) {
  const before = mine.find((r) => r.meta?.headBefore)?.meta.headBefore;
  const after = [...mine].reverse().find((r) => r.meta?.headAfter && r.meta.headAfter !== 'unknown')?.meta.headAfter;
  if (!before || !after) return null;
  const repos = [p.state.worktree, state.integrationWorktree, ...Object.values(state.repositories ?? {}).map((r) => r?.integrationWorktree)].filter(Boolean);
  for (const repo of repos) {
    try {
      const out = execFileSync('git', ['-C', repo, 'diff', '--numstat', before, after], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
      const rows = out.trim().split('\n').filter(Boolean).map((l) => l.split('\t'));
      return {
        paths: rows.map((r) => r[2]),
        insertions: rows.reduce((n, r) => n + (Number(r[0]) || 0), 0),
        deletions: rows.reduce((n, r) => n + (Number(r[1]) || 0), 0),
      };
    } catch {
      // Try the next repository path.
    }
  }
  return null;
}

/** Earlier checkpoints from checkpoints.jsonl: {n, ts, done, total}. */
function readHistory(dir) {
  const file = path.join(dir, 'checkpoints.jsonl');
  if (!fs.existsSync(file)) return [];
  return fs.readFileSync(file, 'utf8').split('\n').filter(Boolean).map((l) => {
    try {
      return JSON.parse(l);
    } catch {
      return null;
    }
  }).filter((c) => c && Number.isFinite(Number(c.n)));
}

/** Events of events.jsonl and of the emitted journal, as {event, phase}. */
function readEvents(dir) {
  const lines = (file) => (fs.existsSync(file) ? fs.readFileSync(file, 'utf8').split('\n').filter(Boolean) : []);
  const parse = (line) => {
    try {
      return JSON.parse(line);
    } catch {
      return null;
    }
  };
  return [
    ...lines(path.join(dir, 'events.jsonl')).map(parse).filter(Boolean),
    ...lines(path.join(dir, 'emitted.jsonl')).map(parse).filter((e) => e?.body?.event).map((e) => e.body),
  ];
}

function buildGraph(plan, rows, milestone, all, stats, media, history) {
  const shown = all ? rows : rows.filter((p) => p.milestone === milestone);
  const ids = new Set(shown.map((p) => p.phase));
  const nodes = [];
  for (const m of plan.milestones) {
    const mine = rows.filter((p) => p.milestone === m.number);
    if (all || m.number === milestone) {
      nodes.push({ id: `m${m.number}`, type: 'group', label: `M${m.number} · ${m.title}`.slice(0, 80) });
    } else {
      const done = mine.filter((p) => p.status === 'done').length;
      nodes.push({
        id: `m${m.number}`, type: 'milestone', label: `M${m.number} · ${m.title}`.slice(0, 80),
        sublabel: `${done}/${mine.length} phases done`,
        status: done === mine.length ? 'done' : mine.some((p) => p.status === 'active') ? 'active' : 'todo',
      });
    }
  }
  for (const p of shown) {
    const time = p.minutes !== null ? `${p.minutes}m${p.estimate ? ` / ${p.estimate.max}m` : ''}` : null;
    const verdict = p.drift.length ? `⚠ ${p.drift.length} drift` : p.status === 'done' ? 'on plan' : p.rawStatus;
    const node = {
      id: `p${p.phase}`, type: 'task', label: `P${p.phase} · ${p.title}`.slice(0, 80),
      sublabel: [p.kind === 'pr-review' ? 'review' : `lane ${p.lane}`, time, p.attempts > 1 ? `a${p.attempts}` : null, verdict]
        .filter(Boolean).join(' · ').slice(0, 120),
      group: `m${p.milestone}`, status: p.status,
    };
    const details = [];
    if (p.doneWhen) details.push(`**Done when:** ${p.doneWhen}`);
    if (p.drift.length) details.push(`**Drift from the plan:**\n${p.drift.map((d) => `- ${d.kind}: ${d.detail}`).join('\n')}`);
    if (p.verify) details.push(`**Verify:** ${p.verify.passed ? 'passed' : 'not recorded'}${p.verify.summary ? ` (${p.verify.summary})` : ''}`);
    if (p.diff) details.push(`**Diff:** +${p.diff.insertions} −${p.diff.deletions} in ${p.diff.files} files`);
    if (details.length) node.details = details.join('\n\n').slice(0, 4000);
    if (p.status === 'active' || p.drift.length) node.emphasis = true;
    nodes.push(node);
  }
  const edges = shown.flatMap((p) => p.dependsOn.filter((d) => ids.has(d)).map((d) => ({ from: `p${d}`, to: `p${p.phase}`, kind: 'depends' })));

  const newest = new Map();
  for (const m of media.filter((x) => x.kind === 'image' && ids.has(x.phase))) newest.set(m.phase, m);
  const images = [...newest.values()].sort((a, b) => a.mtime - b.mtime).slice(-6);
  for (const m of images) {
    const id = `img-p${m.phase}`;
    nodes.push({ id, type: 'image', image: m.path, label: path.basename(m.path).slice(0, 80), sublabel: `P${m.phase} proof`, group: `m${rows.find((r) => r.phase === m.phase).milestone}` });
    edges.push({ from: `p${m.phase}`, to: id, kind: 'data', label: 'proof' });
  }

  if (history.length) {
    nodes.push({ id: 'checkpoints', type: 'group', label: 'Checkpoints' });
    history.forEach((c, k) => {
      const id = `cp${c.n}`;
      nodes.push({ id, type: 'milestone', label: `CP${String(c.n).padStart(2, '0')}`, sublabel: `${String(c.ts).slice(11, 16)} · ${c.done}/${c.total} done`, group: 'checkpoints', status: 'done' });
      if (k) edges.push({ from: `cp${history[k - 1].n}`, to: id, kind: 'flow' });
    });
  }

  const pick = (list) => list.map((p) => `p${p.phase}`);
  const onPlan = shown.filter((p) => p.status === 'done' && !p.drift.length);
  const drifted = shown.filter((p) => p.drift.length);
  const active = shown.filter((p) => p.status === 'active');
  const doneSet = new Set(rows.filter((p) => p.status === 'done').map((p) => p.phase));
  const ready = shown.filter((p) => p.status === 'todo' && p.dependsOn.every((d) => doneSet.has(d)));
  const gates = shown.filter((p) => p.kind === 'pr-review');
  const steps = [{ title: 'The plan', text: `${stats.plan.done}/${stats.plan.total} phases done. ${stats.onPlan} on plan, ${stats.drifted} drifted.`, focus: plan.milestones.filter((m) => all || m.number === milestone).map((m) => `m${m.number}`) }];
  if (onPlan.length) steps.push({ title: 'On plan', text: `${onPlan.length} finished phases match the plan.`, focus: pick(onPlan) });
  if (drifted.length) steps.push({ title: 'Drift from the plan', text: drifted.map((p) => `P${p.phase}: ${p.drift.map((d) => d.kind).join(', ')}`).join('. '), focus: pick(drifted) });
  if (active.length) steps.push({ title: 'Running now', text: active.map((p) => `P${p.phase} ${p.title}`).join(', '), focus: pick(active) });
  if (ready.length) steps.push({ title: 'Next', text: `Ready to start: ${ready.map((p) => `P${p.phase}`).join(', ')}.`, focus: pick(ready) });
  if (images.length) steps.push({ title: 'Proof', text: 'The newest proof of each phase.', focus: images.map((m) => `img-p${m.phase}`) });
  if (history.length) steps.push({ title: 'Checkpoints', text: `${history.length} checkpoints so far.`, focus: history.map((c) => `cp${c.n}`) });
  if (steps.length < 3 && gates.length) steps.push({ title: 'Review gates', text: 'Each milestone ends with a review.', focus: pick(gates) });

  const running = stats.running.map((n) => `P${n}`).join(', ');
  return {
    title: `${plan.name} — ${all ? 'all milestones' : `milestone ${milestone}`}`.slice(0, 80),
    summary: `${stats.plan.done}/${stats.plan.total} done · ${stats.onPlan} on plan · ${stats.drifted} drifted${running ? ` · ${running} running` : ''}`.slice(0, 120),
    direction: 'right',
    nodes,
    edges,
    steps: steps.slice(0, 7),
  };
}

/** One entry per attempt directory (runs/<slug>-a<k>): its phase, start, end and proof media. */
function readRuns(dir, phases) {
  const runsDir = path.join(dir, 'runs');
  if (!fs.existsSync(runsDir)) return [];
  const slugs = phases.filter((p) => p.state.slug).map((p) => [p.state.slug, p.phase]);
  const runs = [];
  for (const name of fs.readdirSync(runsDir)) {
    const match = slugs.find(([slug]) => new RegExp(`^${escapeRegExp(slug)}-a\\d+$`).test(name));
    if (!match) continue;
    const runDir = path.join(runsDir, name);
    const prompt = statOrNull(path.join(runDir, 'prompt.md'));
    if (!prompt) continue;
    const meta = statOrNull(path.join(runDir, 'meta.json'));
    const proofDir = path.join(runDir, 'proof');
    const media = fs.existsSync(proofDir)
      ? fs.readdirSync(proofDir)
        .filter((f) => MEDIA_KIND[path.extname(f).toLowerCase()])
        .map((f) => ({ phase: match[1], path: path.join(proofDir, f), kind: MEDIA_KIND[path.extname(f).toLowerCase()], mtime: fs.statSync(path.join(proofDir, f)).mtime }))
      : [];
    runs.push({
      phase: match[1], start: prompt.mtime, end: meta?.mtime ?? null, media,
      meta: meta ? readJson(path.join(runDir, 'meta.json')) : null,
      verified: fs.existsSync(path.join(runDir, 'verified.json')),
      verifySummary: verifySummary(path.join(runDir, 'verify.log')),
    });
  }
  return runs;
}

function readJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch {
    return null;
  }
}

/** The last test-count lines of a Verify log, joined, or null. */
function verifySummary(file) {
  if (!fs.existsSync(file)) return null;
  const lines = fs.readFileSync(file, 'utf8').split('\n').map((l) => l.trim())
    .filter((l) => /\b(pass|passed|fail|failed|tests?)\b/i.test(l));
  return lines.length ? lines.slice(-3).join(' · ').slice(0, 200) : null;
}

function statOrNull(file) {
  try {
    return fs.statSync(file);
  } catch {
    return null;
  }
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

const SHEET_COLOR = { done: '#2f9e5b', active: '#3987e5', review: '#8a63d2', blocked: '#d9822b', failed: '#d64545', idle: '#9aa3ad', todo: '#d7dce2' };
const SHEET_MEDIA_BYTES = 1.5 * 1024 * 1024;

/** One SVG sheet: the whole plan by milestone, the drift from the plan and the newest proof images. */
export function renderSheet(cp, { title = cp.name, label = '' } = {}) {
  const width = 1200;
  const tile = { w: 132, h: 58, gap: 8 };
  const parts = [];
  let y = 0;
  const text = (x, ty, value, attrs = '') => parts.push(`<text x="${x}" y="${ty}" ${attrs}>${esc(value)}</text>`);

  text(24, 34, title, 'font-size="22" font-weight="700"');
  text(24, 58, [label, cp.generatedAt.slice(0, 16).replace('T', ' ') + ' UTC'].filter(Boolean).join(' · '), 'font-size="13" fill="#5b6470"');
  const s = cp.stats;
  const running = s.running.length ? ` · running ${s.running.map((n) => `P${n}`).join(', ')}` : '';
  text(width - 24, 34, `${s.plan.done}/${s.plan.total} phases done`, 'font-size="18" font-weight="700" text-anchor="end"');
  text(width - 24, 58, `${s.onPlan} on plan · ${s.drifted} drifted · ${s.retries} retries${running}`, 'font-size="13" fill="#5b6470" text-anchor="end"');
  parts.push(`<rect x="24" y="70" width="${width - 48}" height="8" rx="4" fill="${SHEET_COLOR.todo}"/>`);
  parts.push(`<rect x="24" y="70" width="${Math.round((width - 48) * (s.plan.total ? s.plan.done / s.plan.total : 0))}" height="8" rx="4" fill="${SHEET_COLOR.done}"/>`);
  y = 104;

  const groups = cp.milestones.length ? cp.milestones : [{ number: 1, title: 'Phases' }];
  for (const m of groups) {
    const rows = cp.phases.filter((p) => p.milestone === m.number);
    if (!rows.length) continue;
    text(24, y, `M${m.number} · ${m.title}${m.number === cp.milestone ? ' (current)' : ''}`, 'font-size="15" font-weight="600"');
    y += 10;
    const perRow = Math.floor((width - 48 + tile.gap) / (tile.w + tile.gap));
    rows.forEach((p, k) => {
      const x = 24 + (k % perRow) * (tile.w + tile.gap);
      const ty = y + Math.floor(k / perRow) * (tile.h + tile.gap);
      const stroke = p.drift.length ? ' stroke="#d64545" stroke-width="3"' : '';
      parts.push(`<rect x="${x}" y="${ty}" width="${tile.w}" height="${tile.h}" rx="8" fill="${SHEET_COLOR[p.status] ?? SHEET_COLOR.todo}"${stroke}/>`);
      const ink = p.status === 'todo' || p.status === 'idle' ? '#1f2933' : '#ffffff';
      text(x + 10, ty + 20, `P${p.phase}`, `font-size="14" font-weight="700" fill="${ink}"`);
      text(x + 10, ty + 36, p.title.length > 20 ? `${p.title.slice(0, 19)}…` : p.title, `font-size="10" fill="${ink}"`);
      const time = p.minutes !== null ? `${p.minutes}m${p.estimate ? `/${p.estimate.max}m` : ''}` : p.kind === 'pr-review' ? 'review' : '';
      text(x + 10, ty + 50, [time, p.attempts > 1 ? `a${p.attempts}` : '', p.drift.length ? `⚠${p.drift.length}` : ''].filter(Boolean).join(' · '), `font-size="10" fill="${ink}"`);
    });
    y += Math.ceil(rows.length / perRow) * (tile.h + tile.gap) + 24;
  }

  const drifted = cp.phases.filter((p) => p.drift.length);
  text(24, y, drifted.length ? 'Drift from the plan' : 'No drift from the plan', 'font-size="15" font-weight="600"');
  y += 22;
  for (const p of drifted) {
    text(24, y, `P${p.phase} · ${p.drift.map((d) => `${d.kind}: ${d.detail}`).join(' · ')}`.slice(0, 170), 'font-size="12" fill="#1f2933"');
    y += 18;
  }
  y += 14;

  const images = cp.media.filter((m) => m.kind === 'image').slice(-6);
  if (images.length) {
    text(24, y, 'Newest proof', 'font-size="15" font-weight="600"');
    y += 12;
    const box = { w: 180, h: 120 };
    images.forEach((m, k) => {
      const x = 24 + k * (box.w + 14);
      parts.push(`<rect x="${x}" y="${y}" width="${box.w}" height="${box.h}" rx="6" fill="#f2f4f7"/>`);
      const href = embed(m.path);
      if (href) parts.push(`<image x="${x}" y="${y}" width="${box.w}" height="${box.h}" preserveAspectRatio="xMidYMid meet" href="${href}"/>`);
      text(x, y + box.h + 14, `P${m.phase} · ${path.basename(m.path)}`.slice(0, 30), 'font-size="10" fill="#5b6470"');
    });
    y += box.h + 30;
  }

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${y + 10}" viewBox="0 0 ${width} ${y + 10}" font-family="-apple-system, Helvetica, Arial, sans-serif">`
    + `<rect width="100%" height="100%" fill="#ffffff"/>${parts.join('')}</svg>\n`;
}

function embed(file) {
  const types = { '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.gif': 'image/gif', '.webp': 'image/webp' };
  const type = types[path.extname(file).toLowerCase()];
  try {
    if (!type || fs.statSync(file).size > SHEET_MEDIA_BYTES) return null;
    return `data:${type};base64,${fs.readFileSync(file).toString('base64')}`;
  } catch {
    return null;
  }
}

export function esc(value) {
  return String(value ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function main(argv) {
  const args = { all: false };
  for (let k = 0; k < argv.length; k += 1) {
    if (argv[k] === '--all') args.all = true;
    else if (['--dir', '--since', '--sheet', '--label'].includes(argv[k])) args[argv[k].slice(2)] = argv[++k];
    else return fail(`unknown argument ${argv[k]}`);
  }
  if (!args.dir) return fail('use --dir <loop dir> [--since <ISO time>] [--all] [--sheet <file.svg> [--label <text>]]');
  const dir = path.resolve(args.dir);
  if (!fs.existsSync(path.join(dir, 'plan.md'))) return fail(`${dir} has no plan.md`);
  const cp = checkpoint({ dir, since: args.since ?? null, all: args.all });
  if (args.sheet) {
    fs.mkdirSync(path.dirname(path.resolve(args.sheet)), { recursive: true });
    fs.writeFileSync(args.sheet, renderSheet(cp, { label: args.label ?? '' }));
  }
  process.stdout.write(`${JSON.stringify(cp, null, 2)}\n`);
  return 0;
}

function fail(message) {
  console.error(`loop-checkpoint: ${message}`);
  return 1;
}

if (process.argv[1] && fs.realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    process.exitCode = fail(error.message);
  }
}
