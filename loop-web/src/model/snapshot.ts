import type {
  Plan,
  Runtime,
  Snapshot,
  Problem,
  PrInfo,
  GraphNode,
  TimelineEvent,
  EffortInfo,
  PlanOverview,
} from "./types.ts";
import { buildGraph } from "./build-graph.ts";
import { normalizeEvent, problemSeverity } from "./derive.ts";

interface SnapshotOpts {
  now?: number;
  nowIso?: string | null;
}

/**
 * Compose the full browser-facing `Snapshot` from a parsed plan and (optional) runtime.
 * This is the single entry point the server calls per debounced change / reconcile.
 */
export function buildSnapshot(plan: Plan, runtime: Runtime | null, opts: SnapshotOpts = {}): Snapshot {
  const now = opts.now ?? Date.now();
  const pr = buildPrInfo(plan, runtime);
  const graph = buildGraph(plan, runtime, { now, pr });

  return {
    effort: buildEffort(plan, runtime, pr),
    plan: buildPlanOverview(plan),
    graph,
    problems: buildProblems(graph.nodes),
    events: buildTimeline(runtime),
    loopActive: isLoopActive(graph.nodes, runtime),
    generatedAt: opts.nowIso ?? null,
    warnings: plan.warnings,
    // Defaults: buildSnapshot has no LoopRecord. render() (materialize.ts) overrides both
    // with the real overlay-derived values; other callers/tests keep compiling.
    subOrch: null,
    pendingHil: 0,
  };
}

// --- effort & PR ---------------------------------------------------------------------

function buildEffort(plan: Plan, runtime: Runtime | null, pr: PrInfo | null): EffortInfo {
  return {
    name: plan.name,
    status: plan.status,
    integrationBranch:
      runtime?.state?.integrationBranch ?? plan.loopConfig?.integrationBranch ?? null,
    updatedAt: plan.updatedAt,
    pr,
    runId: runtime?.state?.runId ?? null,
  };
}

/** Prefer the orchestrator-promoted `state.review` verdict; fall back to the latest review run's
 * status.json. reportPath/commentUrl only exist on state.review; slug/attempt on the run. */
function buildPrInfo(plan: Plan, runtime: Runtime | null): PrInfo | null {
  const url = runtime?.state?.prUrl ?? plan.loopConfig?.pr ?? null;
  const reviews = runtime?.reviewRuns ?? [];
  const latest = reviews.length ? reviews[reviews.length - 1] : null;
  const stateReview = runtime?.state?.review ?? null;
  if (!url && reviews.length === 0 && !stateReview) return null;
  return {
    url,
    outcome: stateReview?.outcome ?? latest?.status?.outcome ?? null,
    verdict:
      stateReview?.summary ??
      latest?.status?.summary ??
      stateReview?.outcome ??
      latest?.status?.outcome ??
      null,
    reviewPresent: reviews.length > 0 || stateReview != null,
    reportPath: stateReview?.reportPath ?? null,
    commentUrl: stateReview?.commentUrl ?? null,
    reviewSlug: latest ? slugFromRunDir(latest.runDir) : null,
    reviewAttempt: latest?.k ?? null,
  };
}

function slugFromRunDir(runDir: string | null): string | null {
  if (!runDir) return null;
  const m = runDir.match(/runs\/(.+?)-a\d+\/?$/);
  return m ? m[1] : null;
}

// --- plan overview -------------------------------------------------------------------

function buildPlanOverview(plan: Plan): PlanOverview {
  const lanes = new Set(plan.phases.map((p) => p.lane));
  return {
    name: plan.name,
    status: plan.status,
    updatedAt: plan.updatedAt,
    loopConfig: plan.loopConfig,
    phaseSummary: plan.phases.map((p) => ({
      phase: p.phase,
      title: p.title,
      lane: p.lane,
      status: p.status,
      dependsOn: p.dependsOn,
    })),
    laneCount: lanes.size,
    prose: plan.prose,
  };
}

// --- problems rail -------------------------------------------------------------------

function buildProblems(nodes: GraphNode[]): Problem[] {
  const problems: Problem[] = [];
  for (const n of nodes) {
    const rt = n.runtime;
    if (rt?.hilOpen) {
      problems.push(problem(n, "hil", "Awaiting human decision", rt.hilMarkdown));
    } else if (rt?.awaiting) {
      problems.push(problem(n, "question", "Runner asked a question", null));
    } else if (rt?.problem) {
      problems.push(problem(n, rt.problem, problemDetail(rt.problem), null));
    } else if (n.kind === "pr-review" && n.ui === "problem") {
      problems.push(problem(n, "blocked", "Review flagged changes", null));
    }
  }
  return problems.sort((a, b) => b.severity - a.severity || phaseNum(a) - phaseNum(b));
}

function problem(
  n: GraphNode,
  cls: Problem["class"],
  detail: string,
  hilMarkdown: string | null,
): Problem {
  return {
    id: `${n.id}:${cls}`,
    nodeId: n.id,
    phase: n.phase,
    title: n.title,
    class: cls,
    severity: problemSeverity(cls),
    detail,
    hilMarkdown,
  };
}

function problemDetail(cls: string): string {
  switch (cls) {
    case "timeout": return "Runner watchdog timed out";
    case "verify-fail": return "Claimed done but Verify failed";
    case "stall": return "Claimed done but committed nothing";
    case "crash": return "Exited without a valid status.json";
    case "chain-exhausted": return "Model chain exhausted on API errors";
    case "blocked": return "Runner reported blocked";
    default: return "Problem";
  }
}

// --- timeline ------------------------------------------------------------------------

function buildTimeline(runtime: Runtime | null): TimelineEvent[] {
  const events = (runtime?.events ?? []).map(normalizeEvent);
  // Newest first; rows with no ts sink to the bottom, order otherwise preserved.
  return events
    .map((e, i) => ({ e, i }))
    .sort((a, b) => cmpTsDesc(a.e.ts, b.e.ts) || a.i - b.i)
    .map((x) => x.e);
}

function cmpTsDesc(a: string | null, b: string | null): number {
  if (a === b) return 0;
  if (a == null) return 1;
  if (b == null) return -1;
  return a < b ? 1 : -1;
}

// --- loop liveness -------------------------------------------------------------------

function isLoopActive(nodes: GraphNode[], runtime: Runtime | null): boolean {
  if (!runtime?.present) return false;
  return nodes.some(
    (n) => n.runtime && (n.pulse === "live" || n.status === "running" || n.status === "claimed"),
  );
}

function phaseNum(p: Problem): number {
  return p.phase ? Number(p.phase) : Number.MAX_SAFE_INTEGER;
}
