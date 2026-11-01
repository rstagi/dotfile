// Turn the observed runs into React Flow nodes for one moment. x is time, y is run and lane.
import { runs, T0, T_END } from './data.js';

export const PX = 11; // pixels per minute
export const X0 = 300; // x of T0, right of the lane headers
const PHASE_MIN_W = 230;
const GAP = 14;
const PHASE_H = 280;
const RUN_HEAD = 64;
const LANE_H = { coordinator: 150, phases: 440, phasesNoMedia: 320 };
const RUN_GAP = 90;

export const mins = (value) => Math.floor(Date.parse(value) / 60_000);
export const fmt = (m) => new Date(m * 60_000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
let origin = mins(T0);
export const xOf = (m) => X0 + (m - origin) * PX;
const COLLAPSED_H = 128;

/** First minute of a run: its earliest phase start or checkpoint. */
export function runStart(run) {
  const times = run.lanes.flatMap((l) => [...(l.phases ?? []).map((p) => p.start).filter(Boolean), ...(l.checkpoints ?? []).map((c) => c.t)]);
  return Math.min(mins(run.startedAt), ...times.map(mins));
}
export const isFinished = (run, now) => Boolean(run.finishedAt) && mins(run.finishedAt) <= now;

/** The runs a view shows at minute `now`: the active ones first, then the finished ones. */
export function visibleRuns(now, { view = 'all', chat } = {}) {
  const started = runs.filter((r) => runStart(r) <= now);
  const inView = view === 'session' ? started.filter((r) => r.lanes.some((l) => l.chat === chat || l.chats?.includes(chat))) : started;
  return [...inView.filter((r) => !isFinished(r, now)), ...inView.filter((r) => isFinished(r, now))];
}

/** All nodes and edges of the cockpit at minute `now`. A finished run is a card until it is expanded. */
export function layout(now, { view = 'all', chat, expanded = new Set() } = {}) {
  const nodes = [];
  const edges = [];
  const shown = visibleRuns(now, { view, chat });
  const open = (run) => !isFinished(run, now) || expanded.has(run.id);
  origin = Math.min(mins(T0), ...shown.filter(open).map(runStart));
  const width = xOf(mins(T_END)) + 60;
  let y = 0;
  nodes.push({ id: 'ruler', type: 'ruler', position: { x: X0, y: -56 }, data: { from: origin, to: mins(T_END) }, selectable: false, zIndex: 2 });

  let finishedLabel = false;
  for (const run of shown) {
    if (isFinished(run, now) && !finishedLabel) {
      finishedLabel = true;
      nodes.push({ id: 'label:finished', type: 'sectionLabel', position: { x: 0, y }, data: { text: 'Finished today' }, selectable: false });
      y += 44;
    }
    if (!open(run)) {
      nodes.push({ id: `run:${run.id}`, type: 'runCollapsed', position: { x: 0, y }, data: { run, ...collapsedSummary(run) }, style: { width: Math.min(width, 1200), height: COLLAPSED_H } });
      y += COLLAPSED_H + 24;
      continue;
    }
    const top = y;
    let laneY = top + RUN_HEAD;
    const summary = { done: 0, active: 0, todo: 0, drift: 0, hil: 0 };
    for (const lane of run.lanes) {
      const laneNodes = lane.role === 'coordinator' ? coordinatorLane(run, lane, laneY, now) : phaseLane(run, lane, laneY, now, summary, edges);
      nodes.push(...laneNodes.nodes);
      laneY += laneNodes.height;
    }
    nodes.push({
      id: `run:${run.id}`, type: 'run', position: { x: 0, y: top }, zIndex: -1, selectable: true,
      data: { run, summary, finished: isFinished(run, now) }, style: { width, height: laneY - top + 16 },
    });
    y = laneY + 16 + RUN_GAP;
  }
  nodes.push({ id: 'now', type: 'nowLine', position: { x: xOf(now), y: -64 }, data: { now, height: y }, selectable: false, zIndex: 3 });
  return { nodes, edges, width, height: y, runs: shown };
}

function collapsedSummary(run) {
  const phases = run.lanes.flatMap((l) => l.phases ?? []);
  return {
    phases: phases.length,
    drift: phases.filter((p) => (p.drift ?? []).length).length,
    minutes: mins(run.finishedAt) - runStart(run),
    media: phases.flatMap((p) => p.media ?? []),
  };
}

function phaseLane(run, lane, laneY, now, summary, edges) {
  const nodes = [];
  const shown = lane.phases.map((p) => phaseState(p, now)).filter(Boolean);
  // Keep time as x, but never let a phase overlap the one before it in its lane.
  let right = -Infinity;
  for (const s of shown) {
    s.x = Math.max(s.x, right + GAP);
    right = s.x + s.w;
  }
  const hasMedia = shown.some((s) => s.media.length);
  const height = hasMedia ? LANE_H.phases : LANE_H.phasesNoMedia;
  const current = shown.find((s) => s.status === 'active' || s.status === 'hil');
  nodes.push({
    id: `lane:${run.id}:${lane.id}`, type: 'lane', position: { x: 20, y: laneY + 10 }, zIndex: 1,
    data: { lane, run, current, idle: !current, waiting: shown.find((s) => s.status === 'todo' && s.phase.waiting) },
  });
  for (const s of shown) {
    summary[s.status === 'todo' ? 'todo' : s.status] = (summary[s.status === 'todo' ? 'todo' : s.status] ?? 0) + 1;
    const id = `phase:${run.id}:${s.phase.key}`;
    nodes.push({
      id, type: 'phase', position: { x: s.x, y: laneY + 14 }, zIndex: 1,
      data: { ...s, run, lane }, style: { width: s.w, height: PHASE_H },
    });
    s.media.forEach((m, i) => {
      const mid = `media:${run.id}:${s.phase.key}:${i}`;
      nodes.push({ id: mid, type: 'media', position: { x: s.x + i * 168, y: laneY + PHASE_H + 30 }, zIndex: 1, data: { media: m, phase: s.phase, run } });
      edges.push({ id: `e:${mid}`, source: id, target: mid, type: 'straight', className: 'edge-proof', sourceHandle: 'bottom' });
    });
    for (const dep of s.phase.deps ?? []) {
      edges.push({
        id: `dep:${run.id}:${dep}->${s.phase.key}`, source: `phase:${run.id}:${dep}`, target: id, type: 'smoothstep',
        className: s.status === 'todo' ? 'edge-dep edge-dep-todo' : 'edge-dep', sourceHandle: 'right', targetHandle: 'left',
      });
    }
  }
  return { nodes, height };
}

function phaseState(phase, now) {
  const visible = (list = []) => list.filter((item) => mins(item.t) <= now);
  if (!phase.start || mins(phase.start) > now) {
    if (phase.inferred) return null;
    const start = Math.max(mins(phase.planned ?? phase.start), now + 4);
    return { phase, status: 'todo', x: xOf(start), w: Math.max(PHASE_MIN_W, (phase.est?.[1] ?? 20) * PX), minutes: null, intents: [], live: [], groups: [], media: [], drift: [] };
  }
  const start = mins(phase.start);
  const end = phase.end && mins(phase.end) <= now ? mins(phase.end) : null;
  const drift = visible(phase.drift);
  const ask = phase.ask && mins(phase.ask.t) <= now ? phase.ask : null;
  let status = end ? (drift.length ? 'drift' : 'done') : ask ? 'hil' : 'active';
  if (!end && drift.length) status = 'active';
  const minutes = (end ?? now) - start;
  return {
    phase, status, ask, drift, minutes,
    x: xOf(start), w: Math.max(PHASE_MIN_W, minutes * PX),
    media: visible(phase.media),
    ...intentView(phase, now, status === 'active' || status === 'hil'),
  };
}

/**
 * Split the intents seen so far into live ones (shown in full) and older ones (merged into groups).
 * A running phase keeps its last 2 intents live. A finished phase shows only groups.
 */
export function intentView(phase, now, running) {
  const intents = (phase.intents ?? [])
    .map((it, index) => ({ ...it, index, actions: (it.actions ?? []).filter((x) => mins(x.t) <= now) }))
    .filter((it) => mins(it.t) <= now);
  const live = running ? intents.filter(it => it.status !== 'earlier').slice(-2) : [];
  const liveIdx = new Set(live.map((it) => it.index));
  const groups = (phase.groups ?? [])
    .map((g, gi) => {
      const members = intents.filter((it) => g.members.includes(it.index) && !liveIdx.has(it.index));
      return members.length ? { gi, text: g.text, members, from: fmt(mins(members[0].t)), to: fmt(mins(members.at(-1).t)) } : null;
    })
    .filter(Boolean);
  return { intents, live, groups };
}

function coordinatorLane(run, lane, laneY, now) {
  const nodes = [{ id: `lane:${run.id}:${lane.id}`, type: 'lane', position: { x: 20, y: laneY + 10 }, zIndex: 1, data: { lane, run, coordinator: true } }];
  for (const cp of lane.checkpoints.filter((c) => mins(c.t) <= now)) {
    nodes.push({ id: `cp:${run.id}:${cp.n}`, type: 'checkpoint', position: { x: xOf(mins(cp.t)) - 14, y: laneY + 18 }, zIndex: 2, data: { cp, run } });
  }
  for (const other of run.lanes.filter((l) => l.phases)) {
    for (const p of other.phases.filter((ph) => ph.end && mins(ph.end) <= now)) {
      nodes.push({ id: `merge:${run.id}:${p.key}`, type: 'merge', position: { x: xOf(mins(p.end)) - 6, y: laneY + 104 }, zIndex: 2, data: { phase: p, drift: (p.drift ?? []).length > 0 }, selectable: false });
    }
  }
  return { nodes, height: LANE_H.coordinator };
}
