import { useEffect, useMemo, useRef } from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  BackgroundVariant,
  useNodesState,
  useEdgesState,
  useReactFlow,
} from "@xyflow/react";
import type { Node, Edge, NodeProps } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { Snapshot, GraphNode } from "../../model/types.ts";
import { computeLayout } from "./layout.ts";
import type { LaneBand } from "./layout.ts";
import { PhaseNode } from "./PhaseNode.tsx";

function BandNode({ data }: NodeProps) {
  const band = (data as { band: LaneBand }).band;
  const label = band.lane === "integration" ? "INTEGRATION" : `LANE ${band.label}`;
  return (
    <div className={`band band--${band.lane}`} style={{ width: band.width, height: band.height }}>
      <span className="band__label">{label}</span>
    </div>
  );
}

const nodeTypes = { phase: PhaseNode, band: BandNode };

interface Props {
  snapshot: Snapshot;
  selectedId: string | null;
  onSelect: (node: GraphNode | null) => void;
}

function Canvas({ snapshot, selectedId, onSelect }: Props) {
  const layout = useMemo(() => computeLayout(snapshot.graph), [snapshot.graph]);
  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const rf = useReactFlow();
  const structRef = useRef("");

  useEffect(() => {
    setNodes(buildNodes(snapshot, layout, selectedId));
    setEdges(buildEdges(snapshot));
  }, [snapshot, layout, selectedId, setNodes, setEdges]);

  // Refit only when the graph's structure (not its live data) changes.
  useEffect(() => {
    const sig = snapshot.graph.nodes.map((n) => n.id).join(",") + "|" + snapshot.graph.lanes.join(",");
    if (sig !== structRef.current) {
      structRef.current = sig;
      // fitView is a JS-driven camera move — the CSS reduced-motion override doesn't touch it.
      const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
      requestAnimationFrame(() => rf.fitView({ padding: 0.16, duration: reduce ? 0 : 450 }));
    }
  }, [snapshot.graph, rf]);

  return (
    <ReactFlow
      className="canvas"
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      nodesDraggable={false}
      nodesConnectable={false}
      elementsSelectable={false}
      panOnScroll
      selectionOnDrag={false}
      minZoom={0.35}
      maxZoom={1.6}
      fitView
      fitViewOptions={{ padding: 0.16 }}
      proOptions={{ hideAttribution: true }}
      onNodeClick={(_, n) => {
        const gn = (n.data as { node?: GraphNode }).node;
        if (gn) onSelect(gn);
      }}
      onPaneClick={() => onSelect(null)}
    >
      <Background variant={BackgroundVariant.Dots} gap={34} size={1} color="#12202600" />
    </ReactFlow>
  );
}

export function GraphCanvas(props: Props) {
  return (
    <ReactFlowProvider>
      <Canvas {...props} />
    </ReactFlowProvider>
  );
}

// --- flow builders -------------------------------------------------------------------

function buildNodes(snapshot: Snapshot, layout: ReturnType<typeof computeLayout>, selectedId: string | null): Node[] {
  const bandNodes: Node[] = layout.bands.map((band) => ({
    id: `band-${band.lane}`,
    type: "band",
    position: { x: band.x, y: band.y },
    data: { band },
    draggable: false,
    selectable: false,
    zIndex: -1,
    style: { zIndex: -1 },
  }));

  const plan = snapshot.plan;
  const phaseNodes: Node[] = snapshot.graph.nodes.map((node) => {
    const pos = layout.positions.get(node.id) ?? { x: 0, y: 0 };
    return {
      id: node.id,
      type: "phase",
      position: pos,
      data: {
        node,
        pr: node.repository
          ? snapshot.effort.repositories.find((repository) => repository.slug === node.repository)?.pr ?? snapshot.effort.pr
          : snapshot.effort.pr,
        plan,
        selected: node.id === selectedId,
      },
      selected: node.id === selectedId,
      draggable: false,
      zIndex: 1,
    };
  });

  return [...bandNodes, ...phaseNodes];
}

function buildEdges(snapshot: Snapshot): Edge[] {
  return snapshot.graph.edges.map((e) => ({
    id: e.id,
    source: e.source,
    target: e.target,
    type: "smoothstep",
    animated: e.blocking,
    className: e.blocking ? "blocking" : e.kind === "to-review" ? "to-review" : "satisfied",
  }));
}
