import type { Graph } from "../../model/types.ts";

// A deterministic left→right layered layout with horizontal swimlane bands.
// Columns come from longest-path depth (Plan=0 … PR Review=last); rows come from the lane.
// The synthetic Plan / PR Review nodes span every band and sit vertically centred.

export const NODE_W = 216;
export const NODE_H = 108;
const COL_W = 288;
const MIN_BAND_H = 168;
const PAD_X = 48;
const PAD_Y = 40;
const BAND_PAD = 28;
const STACK_STEP = NODE_H + 20;

export interface LaneBand {
  lane: string;
  label: string;
  x: number;
  y: number;
  width: number;
  height: number;
  index: number;
}

export interface Layout {
  positions: Map<string, { x: number; y: number }>;
  bands: LaneBand[];
  width: number;
  height: number;
}

export function computeLayout(graph: Graph): Layout {
  const layers = computeLayers(graph);
  const maxLayer = Math.max(0, ...layers.values());

  const lanes = graph.lanes.length ? graph.lanes : ["A"];
  const laneIndex = new Map(lanes.map((l, i) => [l, i]));

  // Group phase nodes by (lane, layer) so co-located nodes can be stacked, not overlapped.
  const cell = new Map<string, string[]>();
  for (const n of graph.nodes) {
    if (n.kind === "plan" || n.kind === "pr-review") continue;
    const li = laneIndex.get(n.lane ?? lanes[0]) ?? 0;
    const key = `${li}:${layers.get(n.id) ?? 1}`;
    (cell.get(key) ?? cell.set(key, []).get(key)!).push(n.id);
  }

  // Each band is tall enough for its deepest stack, so a fan-out never bleeds into a neighbour.
  const laneHeight = lanes.map((_, li) => {
    let maxStack = 1;
    for (let layer = 0; layer <= maxLayer; layer++) {
      const peers = cell.get(`${li}:${layer}`);
      if (peers && peers.length > maxStack) maxStack = peers.length;
    }
    return Math.max(MIN_BAND_H, (maxStack - 1) * STACK_STEP + NODE_H + 2 * BAND_PAD);
  });
  const laneTop: number[] = [];
  let acc = PAD_Y;
  for (const h of laneHeight) {
    laneTop.push(acc);
    acc += h;
  }
  const totalBandH = acc - PAD_Y;
  const reviewNodes = graph.nodes.filter((node) => node.kind === "pr-review");
  const reviewStackH = Math.max(NODE_H, (reviewNodes.length - 1) * STACK_STEP + NODE_H);
  const contentH = Math.max(totalBandH, reviewStackH + 2 * BAND_PAD);
  const spanY = PAD_Y + contentH / 2 - NODE_H / 2;

  const positions = new Map<string, { x: number; y: number }>();
  for (const n of graph.nodes) {
    const layer = layers.get(n.id) ?? 0;
    const x = PAD_X + layer * COL_W;
    if (n.kind === "plan") {
      positions.set(n.id, { x, y: spanY });
      continue;
    }
    if (n.kind === "pr-review") {
      const rank = reviewNodes.findIndex((review) => review.id === n.id);
      const offset = (rank - (reviewNodes.length - 1) / 2) * STACK_STEP;
      positions.set(n.id, { x, y: spanY + offset });
      continue;
    }
    const li = laneIndex.get(n.lane ?? lanes[0]) ?? 0;
    const bandCenter = laneTop[li] + laneHeight[li] / 2;
    const peers = cell.get(`${li}:${layer}`) ?? [n.id];
    const rank = peers.indexOf(n.id);
    const offset = (rank - (peers.length - 1) / 2) * STACK_STEP;
    positions.set(n.id, { x, y: bandCenter + offset - NODE_H / 2 });
  }

  const width = PAD_X * 2 + maxLayer * COL_W + NODE_W;
  const height = PAD_Y * 2 + contentH;
  const bands: LaneBand[] = lanes.map((lane, index) => ({
    lane,
    label: lane,
    x: 0,
    y: laneTop[index],
    width,
    height: laneHeight[index],
    index,
  }));

  return { positions, bands, width, height };
}

/** Longest-path layer per node: plan at 0, phases relaxed over depends-edges, PR Review last. */
function computeLayers(graph: Graph): Map<string, number> {
  const layers = new Map<string, number>();
  for (const n of graph.nodes) layers.set(n.id, n.kind === "plan" ? 0 : 1);

  const depEdges = graph.edges.filter((e) => e.kind === "depends" || e.kind === "plan-to-lane");
  // Relax until stable (DAG ⇒ converges in ≤ node-count passes).
  for (let pass = 0; pass < graph.nodes.length + 1; pass++) {
    let changed = false;
    for (const e of depEdges) {
      const want = (layers.get(e.source) ?? 0) + 1;
      if (want > (layers.get(e.target) ?? 0)) {
        layers.set(e.target, want);
        changed = true;
      }
    }
    if (!changed) break;
  }

  const phaseMax = Math.max(
    1,
    ...graph.nodes.filter((n) => n.kind === "phase" || n.kind === "integration").map((n) => layers.get(n.id) ?? 1),
  );
  for (const node of graph.nodes) {
    if (node.kind === "pr-review") layers.set(node.id, phaseMax + 1);
  }
  return layers;
}
