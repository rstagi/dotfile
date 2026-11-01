#!/usr/bin/env node
// Loop progress checkpoint: one artifact set from a loop directory, for any client to render.
//
// Usage:
//   loop-checkpoint.mjs --dir .loop [--since <ISO time>] [--all]
//     Print JSON: {milestone, stats, timeline, graph, media}.
//     stats: phases done in the plan and in the current milestone, running phases, attempts, retries.
//     timeline: one row per phase with an attempt: start, end (null while it runs), minutes, status.
//     graph: a beluga-graph object. It shows the current milestone, or every phase with --all.
//     media: proof files (runs/*/proof/*) changed after --since, oldest first.
//     Without state.json (before init), the statuses come from plan.md and milestone 1 is current.
//     Exit 1 when the loop directory has no plan.md.
// Requires Node >= 22.18 (the plan parser is TypeScript).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parsePlan } from './loop-plan-model/parse-plan.ts';

const DONE = new Set(['done', 'merged']);
const RUNNING = new Set(['running', 'in-progress', 'spawned']);
const GRAPH_STATUS = { done: 'done', merged: 'done', running: 'active', 'in-progress': 'active', spawned: 'active', blocked: 'blocked', failed: 'failed', paused: 'idle', 'review-ready': 'review' };
const MEDIA_KIND = { '.png': 'image', '.jpg': 'image', '.jpeg': 'image', '.gif': 'image', '.webp': 'image', '.webm': 'video', '.mp4': 'video', '.mov': 'video' };

/** Build the checkpoint of one loop directory. */
export function checkpoint({ dir, since = null, all = false, now = new Date().toISOString() }) {
  const plan = parsePlan(fs.readFileSync(path.join(dir, 'plan.md'), 'utf8'));
  const stateFile = path.join(dir, 'state.json');
  const state = fs.existsSync(stateFile) ? JSON.parse(fs.readFileSync(stateFile, 'utf8')) : { milestone: 1, phases: {} };
  const milestone = Number(state.milestone ?? 1);
  const phases = plan.phases.map((p) => ({ ...p, state: state.phases?.[p.phase] ?? {} }));
  const statusOf = (p) => p.state.status ?? p.status;
  const runs = readRuns(dir, phases);

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
  };

  const timeline = [];
  for (const p of phases) {
    const mine = runs.filter((r) => r.phase === p.phase).sort((a, b) => a.start - b.start);
    if (!mine.length) continue;
    const running = RUNNING.has(statusOf(p));
    const last = mine.at(-1);
    const start = mine[0].start;
    const end = running || !last.end ? null : last.end;
    timeline.push({
      phase: p.phase,
      title: p.title,
      milestone: p.milestone,
      status: GRAPH_STATUS[statusOf(p)] ?? 'todo',
      attempts: mine.length,
      start: start.toISOString(),
      end: end ? end.toISOString() : null,
      minutes: Math.round(((end ?? new Date(now)) - start) / 60000),
    });
  }

  const sinceTime = since ? new Date(since) : null;
  const media = runs.flatMap((r) => r.media)
    .filter((m) => !sinceTime || m.mtime > sinceTime)
    .sort((a, b) => a.mtime - b.mtime)
    .map((m) => ({ phase: m.phase, path: m.path, kind: m.kind, mtime: m.mtime.toISOString() }));

  return { milestone, stats, timeline, graph: buildGraph(plan, phases, statusOf, milestone, all, stats), media };
}

function buildGraph(plan, phases, statusOf, milestone, all, stats) {
  const shown = all ? phases : phases.filter((p) => p.milestone === milestone);
  const ids = new Set(shown.map((p) => p.phase));
  const nodes = [];
  for (const m of plan.milestones) {
    const mine = phases.filter((p) => p.milestone === m.number);
    if (all || m.number === milestone) {
      nodes.push({ id: `m${m.number}`, type: 'group', label: `M${m.number} · ${m.title}`.slice(0, 80) });
    } else {
      const done = mine.filter((p) => DONE.has(statusOf(p))).length;
      const active = mine.some((p) => RUNNING.has(statusOf(p)));
      nodes.push({
        id: `m${m.number}`, type: 'milestone', label: `M${m.number} · ${m.title}`.slice(0, 80),
        sublabel: `${done}/${mine.length} phases done`,
        status: done === mine.length ? 'done' : active ? 'active' : 'todo',
      });
    }
  }
  for (const p of shown) {
    const status = GRAPH_STATUS[statusOf(p)] ?? 'todo';
    const node = {
      id: `p${p.phase}`, type: 'task', label: `P${p.phase} · ${p.title}`.slice(0, 80),
      sublabel: [p.kind === 'pr-review' ? 'review' : `lane ${p.lane}`, statusOf(p), p.state.attempt ? `a${p.state.attempt}` : null].filter(Boolean).join(' · ').slice(0, 120),
      group: `m${p.milestone}`, status,
    };
    if (p.doneWhen) node.details = `**Done when:** ${p.doneWhen}`.slice(0, 4000);
    if (status === 'active') node.emphasis = true;
    nodes.push(node);
  }
  const edges = shown.flatMap((p) => p.dependsOn.filter((d) => ids.has(d)).map((d) => ({ from: `p${d}`, to: `p${p.phase}`, kind: 'depends' })));
  const running = stats.running.map((n) => `P${n}`).join(', ');
  return {
    title: `${plan.name} — ${all ? 'all milestones' : `milestone ${milestone}`}`.slice(0, 80),
    summary: `${stats.plan.done}/${stats.plan.total} phases done${running ? ` · ${running} running` : ''}`,
    direction: 'right',
    nodes,
    edges,
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
    runs.push({ phase: match[1], start: prompt.mtime, end: meta?.mtime ?? null, media });
  }
  return runs;
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

function main(argv) {
  const args = { all: false };
  for (let k = 0; k < argv.length; k += 1) {
    if (argv[k] === '--all') args.all = true;
    else if (argv[k] === '--dir' || argv[k] === '--since') args[argv[k].slice(2)] = argv[++k];
    else return fail(`unknown argument ${argv[k]}`);
  }
  if (!args.dir) return fail('use --dir <loop dir> [--since <ISO time>] [--all]');
  const dir = path.resolve(args.dir);
  if (!fs.existsSync(path.join(dir, 'plan.md'))) return fail(`${dir} has no plan.md`);
  process.stdout.write(`${JSON.stringify(checkpoint({ dir, since: args.since ?? null, all: args.all }), null, 2)}\n`);
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
