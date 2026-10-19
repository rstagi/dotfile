// The graph of the page, drawn with the same positions, under the camera of the storyboard. The
// focus nodes of a stop and their edges stay bright; their neighbours dim a little; the rest dims.
import { useMemo } from "react";
import { useCurrentFrame } from "remotion";
import { CARD, cameraAt, HEIGHT, transitionAt, WIDTH } from "./storyboard.mjs";
import { COLORS as C, FONTS } from "./theme.mjs";

// The graph shows between the header band and the footer band, left of the card.
const CLIP_TOP = 104;
const CLIP_BOTTOM = 64;
const DIM = 0.14;
const NEAR = 0.5;
const EDGE_REST = 0.5;
const EDGE_COLOR = {
  depends: C.accent, then: C.accent, checks: C.accent, flow: C.info, touches: C.info, delivers: C.info,
  affects: C.ok, mitigates: C.bad, about: C.bad, concerns: C.warn, during: C.warn, contains: C.line,
};
const EDGE_DASH = { flow: "7 5", checks: "3 4" };

export function GraphCanvas({ graph, kinds, board }) {
  const frame = useCurrentFrame();
  const camera = cameraAt(board, frame);
  const { index, progress } = transitionAt(board, frame);
  const levels = useMemo(() => board.stops.map((stop) => levelsOf(graph, stop)), [graph, board]);
  const now = levels[index];
  const before = levels[Math.max(0, index - 1)];
  const mix = (a, b) => a + (b - a) * progress;
  const root = graph.nodes.find((n) => n.group === null);
  const cx = root.x + root.w / 2;
  const cy = root.y + root.h / 2;
  return (
    <svg width={WIDTH} height={HEIGHT} style={{ position: "absolute", left: 0, top: 0 }}>
      <defs>
        <clipPath id="graph-area"><rect x="0" y={CLIP_TOP} width={CARD.x - 24} height={HEIGHT - CLIP_TOP - CLIP_BOTTOM} /></clipPath>
        <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M0 0L10 5L0 10z" fill={C.muted} />
        </marker>
      </defs>
      <g clipPath="url(#graph-area)">
        <g transform={`translate(${WIDTH / 2} ${HEIGHT / 2}) scale(${camera.scale}) translate(${-camera.x} ${-camera.y})`}>
          {graph.groups.map((g) => (
            <path key={`s-${g.id}`} d={`M${cx} ${cy} L${Math.max(g.x, Math.min(cx, g.x + g.w))} ${Math.max(g.y, Math.min(cy, g.y + g.h))}`}
              fill="none" stroke={C.line} strokeWidth="2" strokeDasharray="6 8" vectorEffect="non-scaling-stroke" opacity={mix(before.rest, now.rest)} />
          ))}
          {graph.groups.map((g) => (
            <g key={g.id} opacity={mix(before.groups.get(g.id), now.groups.get(g.id))}>
              <rect x={g.x} y={g.y} width={g.w} height={g.h} rx="22" fill={C["surface-2"]} fillOpacity="0.4" stroke={C.line} strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
              <text x={g.x + 26} y={g.y + 38} fill={C["ink-2"]} style={{ font: `700 16px ${FONTS.sans}`, letterSpacing: "0.08em", textTransform: "uppercase" }}>{g.label}</text>
            </g>
          ))}
          {graph.edges.map((e, i) => {
            const opacity = mix(before.edges[i], now.edges[i]);
            if (opacity < 0.01) return null;
            const hot = now.edges[i] === 1 && !now.overview;
            return (
              <path key={`e-${i}`} d={e.d} fill="none" stroke={EDGE_COLOR[e.type] ?? C.muted} strokeWidth={hot ? 3.2 : 1.8} strokeDasharray={EDGE_DASH[e.type]}
                vectorEffect="non-scaling-stroke" opacity={opacity} markerEnd={e.type === "contains" ? undefined : "url(#arrow)"} />
            );
          })}
          {graph.nodes.map((n) => (
            <Node key={n.id} node={n} kinds={kinds} opacity={mix(before.nodes.get(n.id), now.nodes.get(n.id))} focus={!now.overview && now.nodes.get(n.id) === 1}
              root={n === root} />
          ))}
        </g>
      </g>
    </svg>
  );
}

function Node({ node, kinds, opacity, focus, root }) {
  const color = C[kinds[node.kind]?.color] ?? C.accent;
  const stripe = node.tone ? C[node.tone] : color;
  const pad = root ? 20 : 18;
  const firstY = root ? 70 : 52;
  const step = root ? 28 : 20;
  const subY = firstY + (node.lines.length - 1) * step + 26;
  return (
    <g transform={`translate(${node.x} ${node.y})`} opacity={opacity}>
      <rect width={node.w} height={node.h} rx={root ? 18 : 12} fill={C.surface} stroke={focus ? C.accent : root ? C.accent : C.line}
        strokeWidth={focus ? 3.5 : root ? 2.5 : 1.5} vectorEffect="non-scaling-stroke" />
      <rect width="6" height={node.h} rx="3" fill={stripe} />
      <text x={pad} y={root ? 34 : 26} fill={color} style={{ font: `700 ${root ? 13 : 11.5}px ${FONTS.sans}`, letterSpacing: "0.05em", textTransform: "uppercase" }}>{node.kicker}</text>
      {node.lines.map((line, i) => (
        <text key={i} x={pad} y={firstY + i * step} fill={C.ink} style={{ font: `${root ? 700 : 600} ${root ? 22 : 15}px ${FONTS.sans}` }}>{line}</text>
      ))}
      {(node.sub ?? []).map((line, i) => (
        <text key={`s${i}`} x={pad} y={subY + i * 19} fill={C["ink-2"]} style={{ font: `400 13.5px ${FONTS.sans}` }}>{line}</text>
      ))}
    </g>
  );
}

// The opacity of each node, edge and group frame at one stop. An overview shows everything; the
// edges inside a group block show only for a one-node focus.
function levelsOf(graph, stop) {
  const focus = new Set(stop.nodes);
  const near = new Set();
  const single = stop.nodes.length === 1;
  const byId = new Map(graph.nodes.map((n) => [n.id, n]));
  const inner = (e) => e.type === "contains" && byId.get(e.from).group === byId.get(e.to).group;
  if (!stop.overview) {
    for (const e of graph.edges) {
      if (inner(e) && !single) continue;
      if (focus.has(e.from) && !focus.has(e.to)) near.add(e.to);
      if (focus.has(e.to) && !focus.has(e.from)) near.add(e.from);
    }
  }
  const level = (id) => (stop.overview || focus.has(id) ? 1 : near.has(id) ? NEAR : DIM);
  const nodes = new Map(graph.nodes.map((n) => [n.id, level(n.id)]));
  const edges = graph.edges.map((e) => {
    if (inner(e)) return !stop.overview && single && (focus.has(e.from) || focus.has(e.to)) ? 1 : 0;
    if (stop.overview) return EDGE_REST;
    return focus.has(e.from) || focus.has(e.to) ? 1 : DIM / 2;
  });
  const groups = new Map(graph.groups.map((g) => [g.id, stop.overview || graph.nodes.some((n) => n.group === g.id && nodes.get(n.id) > DIM) ? 1 : 0.35]));
  return { overview: stop.overview, nodes, edges, groups, rest: stop.overview ? 1 : 0.3 };
}
