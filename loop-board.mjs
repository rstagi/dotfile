#!/usr/bin/env node
// Loop plan <-> Beluga board.
//
// The board holds the plan structure: an epic task (the plan), one sub-task per milestone and one
// task per phase. `.loop/plan.md` is the execution copy that the Loop scripts read.
//
// Usage:
//   loop-board.mjs split --plan <file|->
//     Print the board task specs of a plan as JSON: {planId, epic, milestones, phases}.
//     The coordinator creates the tasks with its board tools.
//   loop-board.mjs export --epic <KEY> [--board <board.json>] [--base <plan.md>]
//     Render the plan of an epic from the board file (default $BELUGA_BOARD_FILE, else
//     ~/.beluga/board.json). With --base, the fields that Loop owns come from that file: the plan
//     Status line, the Progress log, PR lines, phase statuses and archived milestone stubs.
//     Exit 1 on a usage error, an unknown epic or an unreadable board.
//   loop-board.mjs keys --epic <KEY> [--board <board.json>]
//     Print the task keys of the plan as JSON: {epic, milestones: {n: key}, phases: {n: key}}.
//   loop-board.mjs check --epic <KEY> --plan <plan.md> [--board <board.json>]
//     Exit 0 when the board renders the same parsed plan as the file (the file is the base).
//     Exit 1 and print the differing fields otherwise.
//
// Each task brief keeps its raw plan text in one fenced block (````loop-plan, ````loop-milestone or
// ````loop-phase): `key: value` lines, then `@@<part>` lines that start each raw part. The board
// owns the titles, the acceptance criteria (Done when), blocked_by (Depends on) and the status.
// Requires Node >= 22.18 (the plan parser is TypeScript).
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parsePlan } from './loop-plan-model/parse-plan.ts';

const FENCE = '````';
const FENCE_RE = /````loop-(plan|milestone|phase)\n([\s\S]*?)\n````/;
const DONE_WHEN_RE = /^- \*\*Done when:\*\*.*$/;
const DEPENDS_ON_RE = /^- \*\*Depends on:\*\*.*$/;
const DONE_WHEN_SLOT = '- **Done when:** {{done-when}}';
const DEPENDS_ON_SLOT = '- **Depends on:** {{depends-on}}';
const PR_LINE_RE = /^- \*\*PR( \([^)]+\))?:\*\*/;
const STATUS_TAG_RE = /\s*`\[status: [^\]]+\]`/;
const TAGS_RE = /((?:\s*`\[[^\]]+\]`)+)\s*$/;
const BOARD_TO_PLAN = { backlog: 'todo', todo: 'todo', in_progress: 'in-progress', in_review: 'in-progress', done: 'done' };

/** Split a plan into the specs of its board tasks. */
export function splitPlan(markdown) {
  const doc = readDocument(markdown);
  const parsed = parsePlan(markdown);
  const byNumber = new Map(parsed.phases.filter((p) => !p.archived).map((p) => [p.phase, p]));
  const taskPhases = new Set(byNumber.keys());
  const planId = doc.header.planId ?? null;
  const epic = {
    title: doc.title,
    goal: doc.goal,
    decisions: doc.decisions,
    context: fence('plan', { planId }, { head: doc.head, tail: doc.tail }),
  };
  const milestones = doc.milestones.filter((m) => m.explicit).map((m) => ({
    number: m.number,
    archived: m.archived,
    title: `M${m.number} · ${m.title}`,
    goal: `Milestone ${m.number} of ${doc.title}: ${m.title}.`,
    acceptance: m.archived ? '- [x] Milestone shipped and archived' : `- [ ] Every phase of milestone ${m.number} is done\n- [ ] The milestone review passes`,
    context: fence('milestone', { milestone: m.number, archived: m.archived }, { body: m.body }),
  }));
  let order = 0;
  const phases = [];
  for (const m of doc.milestones) {
    for (const raw of m.phases) {
      order += 1;
      const p = byNumber.get(raw.number);
      const extra = p.dependsOn.filter((d) => !taskPhases.has(d));
      const done = slotDoneWhen(raw.body.map((l) => (DEPENDS_ON_RE.test(l) ? DEPENDS_ON_SLOT : l)));
      phases.push({
        number: raw.number,
        milestone: m.number,
        status: p.status,
        title: `P${raw.number} · ${raw.title}`,
        goal: `Phase ${raw.number} of ${doc.title}: ${raw.title}.`,
        acceptance: done.items.map((item) => `- [ ] ${item}`).join('\n'),
        dependsOn: p.dependsOn.filter((d) => taskPhases.has(d)),
        context: fence('phase', { phase: raw.number, order, tags: raw.tags, 'depends-extra': extra.join(', '), 'done-when': done.list ? 'list' : '' }, { body: done.body.join('\n') }),
      });
    }
  }
  return { planId, epic, milestones, phases };
}

/** Render the plan of one epic from a board.json object. */
export function exportPlan(board, epicKey, baseMarkdown = null) {
  const issues = board.issues ?? [];
  const epic = issues.find((i) => i.key === epicKey);
  if (!epic) throw new Error(`no task ${epicKey} on the board`);
  const open = (i) => i.status !== 'canceled';
  const children = (id) => issues.filter((i) => i.parent === id && open(i));
  const plan = readFence(epic.brief?.context, 'plan');
  if (!plan) throw new Error(`${epicKey} has no loop-plan block in its context`);
  const base = baseMarkdown ? readBase(baseMarkdown) : null;

  const milestoneTasks = children(epic.id).map((i) => ({ issue: i, f: readFence(i.brief?.context, 'milestone') })).filter((m) => m.f);
  const groups = milestoneTasks.length
    ? milestoneTasks.map(({ issue, f }) => ({ number: Number(f.meta.milestone), archived: f.meta.archived === 'true', issue, body: f.parts.body ?? '' }))
    : [{ number: 1, implicit: true, issue: epic, body: '' }];
  groups.sort((a, b) => a.number - b.number);

  const phaseTasks = [];
  for (const g of groups) {
    for (const issue of children(g.issue.id)) {
      const f = readFence(issue.brief?.context, 'phase');
      if (f) phaseTasks.push({ issue, f, group: g });
    }
  }
  const numberOf = new Map(phaseTasks.map((t) => [t.issue.id, t.f.meta.phase]));

  const out = ['<!-- loop-plan', `planId: ${plan.meta.planId}`, `board: ${epicKey}`, '-->', '', `# ${epic.title} — Multi-Phase Plan`, ''];
  let head = plan.parts.head ?? '';
  if (base?.status) head = head.replace(/^\*\*Status:\*\*.*$/m, base.status);
  if (base) head = overlayPrLines(head, base.headText);
  pushBlock(out, head);
  pushBlock(out, `## Goal\n${epic.brief?.goal ?? ''}`);
  pushBlock(out, `## Approach & key decisions\n${epic.brief?.decisions ?? ''}`);

  for (const g of groups) {
    const stub = base?.archived.get(g.number);
    if (stub) {
      pushBlock(out, stub);
      continue;
    }
    if (g.implicit) {
      out.push('## Phases', '');
    } else {
      const title = g.issue.title.replace(/^M\d+ · /, '');
      let body = g.body;
      if (base?.milestoneBodies.has(g.number)) body = overlayPrLines(body, base.milestoneBodies.get(g.number));
      pushBlock(out, `## Milestone ${g.number} — ${title}${g.archived ? ' `[archived]`' : ''}${body ? `\n${body}` : ''}`);
      if (g.archived) continue;
    }
    const mine = phaseTasks.filter((t) => t.group === g).sort((a, b) => Number(a.f.meta.order) - Number(b.f.meta.order));
    for (const t of mine) pushBlock(out, renderPhase(t, numberOf, base));
  }
  let tail = plan.parts.tail ?? '';
  if (base?.progressLog) tail = replaceSection(tail, 'Progress log', base.progressLog);
  pushBlock(out, tail);
  return `${out.join('\n').replace(/\n{3,}/g, '\n\n').trimEnd()}\n`;
}

function renderPhase({ issue, f }, numberOf, base) {
  const number = f.meta.phase;
  const status = base?.statuses.get(number) ?? BOARD_TO_PLAN[issue.status] ?? 'todo';
  const tags = (f.meta.tags ?? '').trim();
  const title = issue.title.replace(/^P\d+ · /, '');
  const deps = [...(issue.blockedBy ?? []).map((id) => numberOf.get(id)).filter(Boolean), ...splitList(f.meta['depends-extra'])];
  const unique = [...new Set(deps)].sort((a, b) => Number(a) - Number(b));
  const dependsOn = unique.length ? unique.map((n) => `Phase ${n}`).join(', ') : 'none';
  const items = criteria(issue.brief?.acceptance);
  const doneWhen = f.meta['done-when'] === 'list'
    ? ['- **Done when:**', ...items.map((item) => `  - ${item}`)].join('\n')
    : `- **Done when:** ${items.map((item, k) => (k < items.length - 1 && !/[.!?]$/.test(item) ? `${item}.` : item)).join(' ')}`;
  const body = (f.parts.body ?? '')
    .replace(DEPENDS_ON_SLOT, `- **Depends on:** ${dependsOn}`)
    .replace(DONE_WHEN_SLOT, doneWhen.trimEnd());
  const heading = `### Phase ${number} — ${title}${tags ? ` ${tags}` : ''} \`[status: ${status}]\``;
  return `${heading}\n${body}`;
}

// --- reading plans --------------------------------------------------------------------

/** Cut a plan into header, title, head, goal, decisions, milestones (with phases) and tail. */
function readDocument(markdown) {
  const lines = markdown.replace(/\r\n/g, '\n').split('\n');
  const header = {};
  let i = 0;
  if (lines[0] === '<!-- loop-plan') {
    for (i = 1; i < lines.length && lines[i] !== '-->'; i += 1) {
      const m = lines[i].match(/^([A-Za-z]+):\s*(.*)$/);
      if (m) header[m[1]] = m[2].trim();
    }
    i += 1;
  }
  let title = '';
  const preamble = [];
  for (; i < lines.length && !lines[i].startsWith('## '); i += 1) {
    const m = lines[i].match(/^# (.*)$/);
    if (m && !title) title = m[1].replace(/ — Multi-Phase Plan$/, '').trim();
    else preamble.push(lines[i]);
  }
  const sections = [];
  for (; i < lines.length; i += 1) {
    if (lines[i].startsWith('## ')) sections.push({ heading: lines[i].slice(3), lines: [] });
    else sections.at(-1).lines.push(lines[i]);
  }
  const head = [trimLines(preamble).join('\n')];
  const tail = [];
  const milestones = [];
  let goal = '';
  let decisions = '';
  for (const s of sections) {
    const milestone = s.heading.match(/^Milestone (\d+) — (.*?)(\s*`?\[archived\]`?)?\s*$/);
    if (milestone || s.heading.trim() === 'Phases') {
      milestones.push(readMilestone(s, milestone));
    } else if (s.heading.trim() === 'Goal') {
      goal = trimLines(s.lines).join('\n');
    } else if (s.heading.trim() === 'Approach & key decisions') {
      decisions = trimLines(s.lines).join('\n');
    } else {
      (milestones.length ? tail : head).push(`## ${s.heading}\n${trimLines(s.lines).join('\n')}`);
    }
  }
  return { header, title, head: head.filter(Boolean).join('\n\n'), tail: tail.join('\n\n'), goal, decisions, milestones };
}

function readMilestone(section, match) {
  const body = [];
  const phases = [];
  for (const line of section.lines) {
    const p = line.match(/^### Phase (\d+) — (.*)$/);
    if (p) {
      const tagMatch = p[2].match(TAGS_RE);
      const tags = tagMatch ? tagMatch[1].replace(STATUS_TAG_RE, '').trim() : '';
      const title = tagMatch ? p[2].slice(0, tagMatch.index).trim() : p[2].trim();
      phases.push({ number: p[1], title, tags, body: [] });
    } else if (phases.length) {
      phases.at(-1).body.push(line);
    } else {
      body.push(line);
    }
  }
  for (const p of phases) p.body = trimLines(p.body);
  return {
    number: match ? Number(match[1]) : 1,
    title: match ? match[2].trim() : 'Phases',
    explicit: Boolean(match),
    archived: Boolean(match?.[3]),
    body: trimLines(body).join('\n'),
    phases,
  };
}

/** The fields that Loop owns in an execution copy of the plan. */
function readBase(markdown) {
  const doc = readDocument(markdown);
  const statusLine = markdown.match(/^\*\*Status:\*\*.*$/m);
  const progress = doc.tail.split(/\n(?=## )/).find((s) => s.startsWith('## Progress log'));
  const archived = new Map();
  const milestoneBodies = new Map();
  for (const m of doc.milestones.filter((x) => x.explicit)) {
    if (m.archived) archived.set(m.number, `## Milestone ${m.number} — ${m.title} \`[archived]\`\n${m.body}`);
    else milestoneBodies.set(m.number, m.body);
  }
  const statuses = new Map(parsePlan(markdown).phases.filter((p) => !p.archived).map((p) => [p.phase, p.status]));
  return { status: statusLine?.[0] ?? null, headText: doc.head, progressLog: progress ?? null, archived, milestoneBodies, statuses };
}

// --- fences ---------------------------------------------------------------------------

function fence(kind, meta, parts) {
  const lines = [`${FENCE}loop-${kind}`];
  for (const [key, value] of Object.entries(meta)) {
    if (value !== null && value !== undefined && value !== '' && value !== false) lines.push(`${key}: ${value}`);
  }
  for (const [name, text] of Object.entries(parts)) lines.push(`@@${name}`, text ?? '');
  lines.push(FENCE);
  return lines.join('\n');
}

function readFence(context, kind) {
  const m = (context ?? '').match(FENCE_RE);
  if (!m || m[1] !== kind) return null;
  const meta = {};
  const parts = {};
  let current = null;
  for (const line of m[2].split('\n')) {
    const part = line.match(/^@@([a-z-]+)$/);
    if (part) {
      current = part[1];
      parts[current] = [];
    } else if (current) {
      parts[current].push(line);
    } else {
      const kv = line.match(/^([a-zA-Z-]+):\s?(.*)$/);
      if (kv) meta[kv[1]] = kv[2];
    }
  }
  for (const key of Object.keys(parts)) parts[key] = trimLines(parts[key]).join('\n');
  return { meta, parts };
}

// --- helpers --------------------------------------------------------------------------

/**
 * Replace the Done when bullet (with its indented list, if any) by the slot.
 * A bullet with a value gives one criterion. An empty bullet with a list gives one per item.
 */
function slotDoneWhen(lines) {
  const k = lines.findIndex((l) => DONE_WHEN_RE.test(l));
  if (k < 0) return { body: lines, items: [], list: false };
  const value = lines[k].replace(/^- \*\*Done when:\*\*\s*/, '').trim();
  let end = k + 1;
  if (!value) while (end < lines.length && /^\s+- /.test(lines[end])) end += 1;
  const items = value ? [value] : lines.slice(k + 1, end).map((l) => l.replace(/^\s+- /, '').trim());
  return { body: [...lines.slice(0, k), DONE_WHEN_SLOT, ...lines.slice(end)], items, list: !value && items.length > 0 };
}

/** The criteria of an acceptance checklist, without the checkboxes and the deferral notes. */
function criteria(acceptance) {
  return (acceptance ?? '').split('\n')
    .map((l) => l.replace(/^\s*- \[[ xX]\]\s*/, '').replace(/\s+—\s+(deferred|not done):.*$/, '').trim())
    .filter(Boolean);
}

/** Replace each PR line of `text` with the line of the same label in the same block of `baseText`. */
function overlayPrLines(text, baseText) {
  const basePrs = new Map();
  let block = '';
  for (const line of (baseText ?? '').split('\n')) {
    if (line.startsWith('### ')) block = line;
    const m = line.match(PR_LINE_RE);
    if (m) basePrs.set(`${block}|${m[0]}`, line);
  }
  block = '';
  return text.split('\n').map((line) => {
    if (line.startsWith('### ')) block = line;
    const m = line.match(PR_LINE_RE);
    return m ? (basePrs.get(`${block}|${m[0]}`) ?? line) : line;
  }).join('\n');
}

function replaceSection(text, name, section) {
  const parts = text ? text.split(/\n(?=## )/) : [];
  const k = parts.findIndex((s) => s.startsWith(`## ${name}`));
  if (k >= 0) parts[k] = section;
  else parts.push(section);
  return parts.join('\n');
}

function pushBlock(out, text) {
  if (!text || !text.trim()) return;
  out.push(text.trimEnd(), '');
}

function trimLines(lines) {
  let start = 0;
  let end = lines.length;
  while (start < end && !lines[start].trim()) start += 1;
  while (end > start && !lines[end - 1].trim()) end -= 1;
  return lines.slice(start, end);
}

function splitList(value) {
  return (value ?? '').split(',').map((s) => s.trim()).filter(Boolean);
}

// --- CLI ------------------------------------------------------------------------------

function main(argv) {
  const [command, ...rest] = argv;
  const args = {};
  for (let k = 0; k < rest.length; k += 2) {
    if (!rest[k].startsWith('--') || rest[k + 1] === undefined) return fail(`bad argument ${rest[k]}`);
    args[rest[k].slice(2)] = rest[k + 1];
  }
  if (command === 'split') {
    if (!args.plan) return fail('split requires --plan <file|->');
    const markdown = fs.readFileSync(args.plan === '-' ? 0 : args.plan, 'utf8');
    process.stdout.write(`${JSON.stringify(splitPlan(markdown), null, 2)}\n`);
    return 0;
  }
  if (command === 'export') {
    if (!args.epic) return fail('export requires --epic <KEY>');
    const board = readBoard(args.board);
    const base = args.base && fs.existsSync(args.base) ? fs.readFileSync(args.base, 'utf8') : null;
    process.stdout.write(exportPlan(board, args.epic, base));
    return 0;
  }
  if (command === 'keys') {
    if (!args.epic) return fail('keys requires --epic <KEY>');
    process.stdout.write(`${JSON.stringify(boardKeys(readBoard(args.board), args.epic), null, 2)}\n`);
    return 0;
  }
  if (command === 'check') {
    if (!args.epic || !args.plan) return fail('check requires --epic <KEY> --plan <file>');
    const markdown = fs.readFileSync(args.plan, 'utf8');
    const differences = diffPaths(comparable(exportPlan(readBoard(args.board), args.epic, markdown)), comparable(markdown));
    if (!differences.length) return 0;
    return fail(`the board and ${args.plan} differ:\n${differences.slice(0, 20).join('\n')}`);
  }
  return fail('use split --plan <file>, export --epic <KEY> [--board <file>] [--base <plan.md>] or check --epic <KEY> --plan <file> [--board <file>]');
}

/** The task keys of a board plan: {epic, milestones: {n: key}, phases: {n: key}}. Canceled tasks are left out. */
function boardKeys(board, epicKey) {
  const issues = board.issues ?? [];
  const epic = issues.find((i) => i.key === epicKey);
  if (!epic) throw new Error(`no task ${epicKey} on the board`);
  const keys = { epic: epicKey, milestones: {}, phases: {} };
  const walk = (id) => {
    for (const issue of issues.filter((i) => i.parent === id && i.status !== 'canceled')) {
      const milestone = readFence(issue.brief?.context, 'milestone');
      const phase = readFence(issue.brief?.context, 'phase');
      if (milestone) keys.milestones[milestone.meta.milestone] = issue.key;
      if (phase) keys.phases[phase.meta.phase] = issue.key;
      if (milestone) walk(issue.id);
    }
  };
  walk(epic.id);
  return keys;
}

function readBoard(file) {
  return JSON.parse(fs.readFileSync(file ?? process.env.BELUGA_BOARD_FILE ?? path.join(os.homedir(), '.beluga', 'board.json'), 'utf8'));
}

/** The parsed plan without the fields that only describe the file. */
function comparable(markdown) {
  const plan = parsePlan(markdown);
  delete plan.updatedAt;
  delete plan.name;
  return plan;
}

/** The paths where two JSON values differ, as `a.b.0.c  <left> => <right>` lines. */
function diffPaths(left, right, at = '') {
  if (JSON.stringify(left) === JSON.stringify(right)) return [];
  if (typeof left !== 'object' || typeof right !== 'object' || left === null || right === null) {
    return [`${at.slice(1)}  ${JSON.stringify(left)} => ${JSON.stringify(right)}`];
  }
  return [...new Set([...Object.keys(left), ...Object.keys(right)])].flatMap((k) => diffPaths(left[k], right[k], `${at}.${k}`));
}

function fail(message) {
  console.error(`loop-board: ${message}`);
  return 1;
}

if (process.argv[1] && fs.realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    process.exitCode = fail(error.message);
  }
}
