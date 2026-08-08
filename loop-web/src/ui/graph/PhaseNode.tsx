import { Handle, Position } from "@xyflow/react";
import type { NodeProps } from "@xyflow/react";
import type { GraphNode, NodeUiState, PrInfo, PlanOverview } from "../../model/types.ts";
import { reviewPill } from "../theme/glyphs.ts";

export interface PhaseNodeData {
  node: GraphNode;
  pr?: PrInfo | null;
  plan?: PlanOverview | null;
  [key: string]: unknown;
}

const ACCENT: Record<NodeUiState, string> = {
  todo: "var(--steel)",
  running: "var(--aqua)",
  blocked: "var(--red)",
  done: "var(--green)",
  awaiting: "var(--amber)",
  problem: "var(--red)",
};

export function PhaseNode({ data, selected }: NodeProps) {
  const { node, pr, plan } = data as PhaseNodeData;
  const rt = node.runtime;
  const accent = ACCENT[node.ui];

  return (
    <div
      className={`node node--${node.ui}${selected ? " node--selected" : ""}`}
      data-ui={node.ui}
      style={{ ["--accent" as string]: accent }}
    >
      <Handle type="target" position={Position.Left} />
      <Handle type="source" position={Position.Right} />

      {rt?.hilOpen && <span className="node__hil">HIL</span>}
      {node.notePending && (
        <span className="node__note" title={node.noteMarkdown ?? undefined}>NOTE</span>
      )}

      <div className="node__head">
        <span className="node__chip">{chip(node)}</span>
        <span className="node__status" style={{ color: accent }}>
          {statusLabel(node)}
        </span>
      </div>

      <div className="node__title">{node.title}</div>

      {showTrace(node) && <Trace mode={traceMode(node)} />}

      <div className="node__meta">{metaBadges(node, pr, plan)}</div>
    </div>
  );
}

// --- trace ---------------------------------------------------------------------------

const ECG = "M0,11 H18 l3,-7.5 l4,15 l3,-7.5 H46 l3,-4 l4,8 l3,-4 H100";
const FLAT = "M0,11 H100";

function Trace({ mode }: { mode: "live" | "flat" | "idle" }) {
  return (
    <div className={`trace trace--${mode}`}>
      <svg className="trace__svg" viewBox="0 0 100 22" preserveAspectRatio="none">
        <path className="trace__line" d={mode === "live" ? ECG : FLAT} />
      </svg>
      {mode === "live" && <div className="trace__beam" />}
    </div>
  );
}

function showTrace(n: GraphNode): boolean {
  if (n.kind === "plan" || n.kind === "pr-review") return false;
  return n.ui === "running" || n.pulse != null;
}
function traceMode(n: GraphNode): "live" | "flat" | "idle" {
  if (n.pulse === "flatline") return "flat";
  if (n.pulse === "live") return "live";
  return "idle";
}

// --- labels & badges -----------------------------------------------------------------

function chip(n: GraphNode): string {
  if (n.kind === "plan") return "PLAN";
  if (n.kind === "pr-review") return "REVIEW";
  return `P${n.phase}${n.lane ? ` · ${n.lane}` : ""}`;
}

function statusLabel(n: GraphNode): string {
  const rt = n.runtime;
  if (rt?.hilOpen) return "HIL PAUSE";
  if (rt?.awaiting) return "QUESTION";
  if (rt?.problem) return rt.problem.replace("-", " ").toUpperCase();
  return n.status.toUpperCase();
}

function metaBadges(n: GraphNode, pr?: PrInfo | null, plan?: PlanOverview | null) {
  if (n.kind === "pr-review") {
    const pill = reviewPill(pr?.outcome);
    if (pill)
      return (
        <span className="node__badge" style={{ color: pill.color, borderColor: pill.color }}>
          {pill.label}
        </span>
      );
    if (pr?.url) return <span className="node__badge">PR open · awaiting review</span>;
    return <span className="node__badge">no PR yet</span>;
  }
  if (n.kind === "plan") {
    if (!plan) return <span className="node__badge">effort</span>;
    return [
      <span key="p" className="node__badge">{plan.phaseSummary.length} phases</span>,
      <span key="l" className="node__badge">{plan.laneCount} lanes</span>,
    ];
  }

  const rt = n.runtime;
  if (!rt) return null;
  const out = [];
  if (rt.attempt) out.push(<span key="a" className="node__badge">a{rt.attempt}</span>);
  if (rt.model)
    out.push(
      <span key="m" className="node__badge node__badge--model">
        {rt.engine ? `${rt.engine}:` : ""}
        {rt.model}
      </span>,
    );
  else if (rt.branch) out.push(<span key="b" className="node__badge">{truncate(rt.branch, 20)}</span>);
  return out;
}

function truncate(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}
