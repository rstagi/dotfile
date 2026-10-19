// Storyboard: the graph, the tour stops and the voiceover -> the stops of the video, each with its
// camera, its card and its timing. Zero dependencies, so node --test checks it without Remotion.
//
// The input is what render-video.mjs passes: { mode, meta, graph, stops }. `graph` comes from
// scripts/graph.mjs (the positions of the page). `stops` come from scripts/narration.mjs tourStops:
// { id, title, text, nodes, overview }.
//
// The frame has two parts: the graph area on the left and the card on the right. The camera of a
// stop frames its focus nodes in the graph area. The card shows the facts of the focus.

export const FPS = 30;
export const WIDTH = 1920;
export const HEIGHT = 1080;
export const GRAPH_AREA = { x: 56, y: 124, w: 1176, h: 860 };
export const CARD = { x: 1288, y: 124, w: 576, h: 860 };
// The largest zoom: one node box (240 px) then fills 480 px, with 30 px title text.
export const MAX_SCALE = 2;
// The camera moves from the previous stop during the first 0.9 s of a stop.
export const TRANSITION_SECONDS = 0.9;

// Minimum visual time of a stop: reading time for the words of its card, within fixed bounds.
const TITLE_SECONDS = 5;
const END_SECONDS = 4;
const MIN_STOP_SECONDS = 5;
const MAX_STOP_SECONDS = 9;
const READ_BASE_SECONDS = 2;
const WORDS_PER_SECOND = 2.5;
// A narrated stop adds 0.3 s of silence before its audio and 0.3 s after it.
const AUDIO_LEAD_SECONDS = 0.3;
const AUDIO_PAD_SECONDS = 0.6;

const CARD_ITEMS = 6;
const CARD_FACTS = 3;
const CARD_BADGES = 3;
// Characters on the card: it is 576 px wide, so a fact of 130 characters at 26 px takes 4 lines.
const CHARS = { title: 80, listTitle: 70, item: 56, fact: 130, subtitle: 160 };
const ELLIPSIS = "…";
const END_TEXT = "Open the page to explore the graph: every node, every fact and every link.";

// storyboard(input, voiceover, { fps }) -> { fps, width, height, durationInFrames, stops }.
// `voiceover` maps a stop id to its audio clip: { src, seconds }. Each board stop: { id, title,
// nodes, overview, card, camera, from, durationInFrames, transitionInFrames, audio }. `audio` is
// { src, from, durationInFrames } or null. A title stop comes first and an end stop last: the
// first and the last narration stop take these roles when they show the overview ("all").
export function storyboard(input, voiceover = {}, { fps = FPS } = {}) {
  const stops = framed(input);
  const board = [];
  let from = 0;
  stops.forEach((stop, i) => {
    const card = cardOf(input, stop);
    const minimum = Math.round(minimumSeconds(stop, card) * fps);
    const audio = voiceover?.[stop.id];
    const audioFrames = audio ? Math.ceil(audio.seconds * fps) : 0;
    const durationInFrames = Math.max(minimum, audio ? audioFrames + Math.round(AUDIO_PAD_SECONDS * fps) : 0);
    board.push({
      id: stop.id,
      title: stop.title,
      nodes: stop.nodes,
      overview: stop.overview,
      card,
      camera: cameraFor(input.graph, stop),
      from,
      durationInFrames,
      transitionInFrames: i ? Math.min(Math.round(TRANSITION_SECONDS * fps), durationInFrames - 1) : 0,
      audio: audio ? { src: audio.src, from: from + Math.round(AUDIO_LEAD_SECONDS * fps), durationInFrames: audioFrames } : null,
    });
    from += durationInFrames;
  });
  return { fps, width: WIDTH, height: HEIGHT, durationInFrames: from, stops: board };
}

// stopIndexAt(board, frame) -> the index of the stop that shows at `frame`.
export function stopIndexAt(board, frame) {
  let index = 0;
  board.stops.forEach((s, i) => { if (s.from <= frame) index = i; });
  return index;
}

// cameraAt(board, frame) -> { x, y, scale }: the world point at the frame center and the zoom.
// During the transition of a stop the camera eases from the previous stop camera; the zoom
// changes at a constant rate on a log scale.
export function cameraAt(board, frame) {
  const { index: i, progress: e } = transitionAt(board, frame);
  const stop = board.stops[i];
  if (i === 0 || e >= 1) return stop.camera;
  const prev = board.stops[i - 1].camera;
  return {
    x: prev.x + (stop.camera.x - prev.x) * e,
    y: prev.y + (stop.camera.y - prev.y) * e,
    scale: prev.scale * (stop.camera.scale / prev.scale) ** e,
  };
}

// transitionAt(board, frame) -> { index, progress }: the stop at `frame` and the eased progress of
// its transition, from 0 (the previous stop) to 1 (this stop). The highlight fades with it.
export function transitionAt(board, frame) {
  const index = stopIndexAt(board, frame);
  const stop = board.stops[index];
  const t = stop.transitionInFrames ? (frame - stop.from) / stop.transitionInFrames : 1;
  return { index, progress: t >= 1 ? 1 : ease(Math.max(0, t)) };
}

// clip(text, max) -> text on one line, at most `max` characters. A cut ends at a word boundary
// with an ellipsis. A cut inside a `code` span closes the span.
export function clip(text, max) {
  const flat = String(text).replace(/\s+/g, " ").trim();
  if (flat.length <= max) return flat;
  let cut = cutAt(flat, max - ELLIPSIS.length);
  if (!isOdd(backticks(cut))) return `${cut}${ELLIPSIS}`;
  cut = cutAt(flat, max - ELLIPSIS.length - 1);
  return isOdd(backticks(cut)) ? `${cut}${ELLIPSIS}\`` : `${cut}${ELLIPSIS}`;
}

// The narration stops with their roles. A title stop and an end stop are added when the narration
// does not start or end on the overview.
function framed({ graph, meta, stops }) {
  const all = graph.nodes.map((n) => n.id);
  const list = stops.map((s) => ({ ...s, role: "stop" }));
  if (list[0]?.overview) list[0].role = "title";
  else list.unshift({ id: "title", title: meta.title, nodes: all, overview: true, role: "title", added: true });
  if (list.length > 1 && list.at(-1).overview && list.at(-1).role === "stop") list.at(-1).role = "end";
  else list.push({ id: "end", title: meta.title, nodes: all, overview: true, role: "end", added: true });
  return list;
}

// The camera that frames the focus nodes, or the whole graph, in the graph area, at most MAX_SCALE.
function cameraFor(graph, stop) {
  const box = stop.overview || !stop.nodes.length ? { x: 0, y: 0, w: graph.width, h: graph.height } : bounds(graph, stop.nodes);
  const scale = Math.min(GRAPH_AREA.w / box.w, GRAPH_AREA.h / box.h, MAX_SCALE);
  const ax = GRAPH_AREA.x + GRAPH_AREA.w / 2;
  const ay = GRAPH_AREA.y + GRAPH_AREA.h / 2;
  return { x: box.x + box.w / 2 + (WIDTH / 2 - ax) / scale, y: box.y + box.h / 2 + (HEIGHT / 2 - ay) / scale, scale };
}

function bounds(graph, ids) {
  const set = new Set(ids);
  const nodes = graph.nodes.filter((n) => set.has(n.id));
  const x = Math.min(...nodes.map((n) => n.x));
  const y = Math.min(...nodes.map((n) => n.y));
  return { x, y, w: Math.max(...nodes.map((n) => n.x + n.w)) - x, h: Math.max(...nodes.map((n) => n.y + n.h)) - y };
}

// ---- cards ----

function cardOf({ mode, meta, graph }, stop) {
  if (stop.role === "title") {
    return {
      type: "title", eyebrow: mode === "plan" ? "Plan" : "Ship", title: meta.title, subtitle: clip(meta.subtitle, CHARS.subtitle),
      stats: kindCounts(graph), date: meta.date,
    };
  }
  if (stop.role === "end") return { type: "end", title: meta.title, text: END_TEXT };
  if (stop.overview) {
    return { type: "overview", title: clip(stop.title, CHARS.listTitle), items: graph.groups.map((g) => ({ id: g.id, name: g.name, count: graph.nodes.filter((n) => n.group === g.id).length })) };
  }
  const nodes = graph.nodes.filter((n) => stop.nodes.includes(n.id));
  if (nodes.length === 1) return nodeCard(graph, nodes[0]);
  return {
    type: "list",
    title: clip(plain(stop.title), CHARS.listTitle),
    items: nodes.slice(0, CARD_ITEMS).map((n) => ({ id: n.id, kind: n.kind, code: n.code, kicker: n.kicker, title: clip(plain(n.title), CHARS.item), tone: n.tone })),
    more: Math.max(0, nodes.length - CARD_ITEMS),
  };
}

function nodeCard(graph, node) {
  return {
    type: "node", id: node.id, kind: node.kind, kicker: node.kicker, tone: node.tone,
    title: clip(plain(node.title), CHARS.title),
    badges: node.badges.slice(0, CARD_BADGES),
    facts: node.facts.filter((f) => f.key).slice(0, CARD_FACTS).map((f) => ({ label: f.label, text: clip(factText(f), CHARS.fact) })),
    connections: graph.edges.filter((e) => e.from === node.id || e.to === node.id).length,
  };
}

function factText(f) {
  if (f.list) return f.list.join("; ");
  if (f.command !== undefined) return f.command.split("\n").filter((l) => l.trim()).join(" · ");
  if (f.code !== undefined) return f.code;
  if (f.links) return f.links.map((l) => l.label).join(", ");
  return f.text;
}

function kindCounts(graph) {
  const counts = new Map();
  for (const n of graph.nodes) counts.set(n.kind, (counts.get(n.kind) ?? 0) + 1);
  return [...counts].map(([kind, count]) => ({ kind, count }));
}

function plain(text) {
  return String(text).replaceAll("`", "");
}

// ---- timing ----

function minimumSeconds(stop, card) {
  if (stop.added) return stop.role === "title" ? TITLE_SECONDS : END_SECONDS;
  return clamp(READ_BASE_SECONDS + countWords(card) / WORDS_PER_SECOND, MIN_STOP_SECONDS, MAX_STOP_SECONDS);
}

// Words of every text value on the card. Ids, kinds and tones are not on screen as text to read.
function countWords(value, key = "") {
  if (["id", "kind", "tone", "type"].includes(key)) return 0;
  if (typeof value === "string") return value.split(/\s+/).filter(Boolean).length;
  if (Array.isArray(value)) return value.reduce((n, v) => n + countWords(v), 0);
  if (value && typeof value === "object") return Object.entries(value).reduce((n, [k, v]) => n + countWords(v, k), 0);
  return 0;
}

// ---- helpers ----

function ease(t) {
  return t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2;
}

// The longest prefix of at most `budget` characters that ends before a space. One long word
// gets a hard cut, so a cut never drops more than half of the budget.
function cutAt(text, budget) {
  const hard = text.slice(0, budget);
  const space = hard.lastIndexOf(" ");
  return (space >= budget / 2 ? hard.slice(0, space) : hard).trimEnd();
}

function backticks(text) {
  return (text.match(/`/g) ?? []).length;
}

function isOdd(n) {
  return n % 2 === 1;
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}
