import type {
  Plan,
  PlanPhase,
  PlanPhaseStatus,
  Runtime,
  PhaseRuntime,
  Graph,
  GraphNode,
  GraphEdge,
  NodeRuntime,
  NodeUiState,
  PhaseStateStatus,
  ProblemClass,
  PrInfo,
} from "./types.ts";
import {
  classifyAttempt,
  deriveLiveness,
  lastWorkedBy,
  summarizeAttempt,
} from "./derive.ts";

export const PLAN_ID = "plan";
export const REVIEW_ID = "pr-review";

const STATE_STATUSES: readonly PhaseStateStatus[] = [
  "todo",
  "claimed",
  "running",
  "merged",
  "blocked",
  "done",
];

interface BuildOpts {
  now?: number;
  pr?: PrInfo | null;
  repositories?: Record<string, PrInfo | null>;
}

/**
 * Turn the normalized plan (+ optional runtime overlay) into the left-to-right DAG:
 * a synthetic `Plan` root, one node per phase laid out in lane bands, and a synthetic
 * `PR Review` terminal. Edges: Plan→lane-roots, phase Depends-on, terminals→PR Review.
 */
export function buildGraph(plan: Plan, runtime: Runtime | null, opts: BuildOpts = {}): Graph {
  const now = opts.now ?? Date.now();
  const lanes = orderLanes(plan.phases);

  const planNode: GraphNode = {
    id: PLAN_ID,
    kind: "plan",
    title: plan.name || "Plan",
    phase: null,
    lane: null,
    repository: null,
    status: mapHeaderStatus(plan.status),
    ui: resolveUi(mapHeaderStatus(plan.status), null, false),
    pulse: null,
    runtime: null,
    notePending: false,
    noteMarkdown: null,
  };

  const phaseNodes = plan.phases.map((p) => buildPhaseNode(p, runtime?.phases[p.phase], now));
  const reviewNodes = plan.repositories.map((repository) => {
    const legacy = repository.slug === "primary";
    const info = opts.repositories?.[repository.slug] ?? (legacy ? opts.pr ?? null : null);
    const note = legacy ? runtime?.reviewNote ?? null : runtime?.reviewNotes[repositoryKey(repository.slug)] ?? null;
    return buildReviewNode(repository.slug, info, note);
  });

  const nodes = [planNode, ...phaseNodes, ...reviewNodes];
  const edges = buildEdges(plan, nodes);

  return { nodes, edges, lanes };
}

// --- phase nodes ---------------------------------------------------------------------

function buildPhaseNode(
  phase: PlanPhase,
  pr: PhaseRuntime | undefined,
  now: number,
): GraphNode {
  const kind = isIntegrationLane(phase.lane) ? "integration" : "phase";
  const base = {
    id: phase.phase,
    kind: kind as GraphNode["kind"],
    title: phase.title,
    phase: phase.phase,
    lane: phase.lane,
    repository: phase.repository,
  };

  if (!pr) {
    const status = mapPlanStatus(phase.status);
    return {
      ...base,
      status,
      ui: resolveUi(status, null, false),
      pulse: null,
      runtime: null,
      notePending: false,
      noteMarkdown: null,
    };
  }
  return { ...base, ...resolveRuntime(phase, pr, now) };
}

/** Fold state.json + attempt files into the node's lifecycle status, ui token, pulse, runtime. */
function resolveRuntime(
  phase: PlanPhase,
  pr: PhaseRuntime,
  now: number,
): Pick<GraphNode, "status" | "ui" | "pulse" | "runtime" | "notePending" | "noteMarkdown"> {
  const attempts = [...pr.attempts].sort((a, b) => a.k - b.k);
  const ended = attempts.filter((a) => a.meta).sort((a, b) => b.k - a.k);
  const inFlight = attempts.filter((a) => !a.meta).sort((a, b) => b.k - a.k)[0] ?? null;

  const rawProblem = ended[0] ? classifyAttempt(ended[0]) : null;
  const hilOpen = pr.hil?.open ?? false;
  const awaiting = hilOpen || rawProblem === "question";
  // 'question' surfaces via `awaiting`, not as a red problem; blocked stays a problem.
  const problem: ProblemClass | null = rawProblem === "question" ? null : rawProblem;

  const stateStatus = validStateStatus(pr.state?.status);
  const lifecycle = stateStatus ?? mapPlanStatus(phase.status);

  // A runner is live only if an attempt is still in flight (no meta.json yet). An ended
  // attempt under a stale `state=running` is skew, not a heartbeat — never a live pulse.
  const pulse = inFlight ? deriveLiveness(inFlight.transcriptMtime, now) : null;
  const lastHeartbeatAgeSec =
    inFlight?.transcriptMtime != null
      ? Math.max(0, Math.round((now - inFlight.transcriptMtime) / 1000))
      : null;

  const runtime: NodeRuntime = {
    attempt: pr.state?.attempt ?? (attempts.length || null),
    branch: pr.state?.branch ?? null,
    ...lastWorkedBy(attempts),
    lastHeartbeatAgeSec,
    problem,
    awaiting,
    hilOpen,
    hilMarkdown: pr.hil?.markdown ?? null,
    attempts: attempts.map(summarizeAttempt),
    runDir: ended[0]?.runDir ?? inFlight?.runDir ?? pr.state?.runDir ?? null,
    slug: pr.state?.slug ?? slugFromRunDir(ended[0]?.runDir ?? inFlight?.runDir ?? null),
  };

  return {
    status: lifecycle,
    ui: resolveUi(lifecycle, problem, awaiting),
    pulse,
    runtime,
    notePending: pr.note != null && lifecycle !== "done" && lifecycle !== "merged",
    noteMarkdown: pr.note,
  };
}

function resolveUi(
  lifecycle: PhaseStateStatus,
  problem: ProblemClass | null,
  awaiting: boolean,
): NodeUiState {
  if (awaiting) return "awaiting";
  if (problem && problem !== "blocked") return "problem"; // crash/timeout/verify-fail/chain-exhausted
  if (problem === "blocked" || lifecycle === "blocked") return "blocked";
  if (lifecycle === "running" || lifecycle === "claimed") return "running";
  if (lifecycle === "done" || lifecycle === "merged") return "done";
  return "todo";
}

// --- PR Review terminal --------------------------------------------------------------

function buildReviewNode(repository: string, pr: PrInfo | null, noteMarkdown: string | null): GraphNode {
  // Drive the terminal off the STRUCTURED review outcome, never the free-form summary prose
  // (a passing review's summary almost always contains "changes"/"blocking"/"approve").
  let status: PhaseStateStatus = "todo";
  let ui: NodeUiState = "todo";
  const outcome = pr?.outcome;
  if (pr?.reviewPresent && (outcome === "blocked" || outcome === "question")) {
    status = "blocked";
    ui = "problem";
  } else if (pr?.reviewPresent && outcome === "done") {
    status = "done";
    ui = "done";
  } else if (pr?.url) {
    status = "running";
    ui = "running";
  }
  return {
    id: reviewId(repository),
    kind: "pr-review",
    title: repository === "primary" ? "PR Review" : `${repository} Review`,
    phase: null,
    lane: null,
    repository,
    status,
    ui,
    pulse: null,
    runtime: null,
    notePending: noteMarkdown != null && pr?.outcome == null,
    noteMarkdown,
  };
}

// --- edges ---------------------------------------------------------------------------

function buildEdges(plan: Plan, nodes: GraphNode[]): GraphEdge[] {
  const phases = plan.phases;
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const satisfied = (id: string) => {
    const s = byId.get(id)?.status;
    return s === "done" || s === "merged";
  };
  const edges: GraphEdge[] = [];

  if (phases.length === 0) {
    for (const repository of plan.repositories) {
      const target = reviewId(repository.slug);
      edges.push({ id: `${PLAN_ID}->${target}`, source: PLAN_ID, target, kind: "to-review", blocking: false });
    }
    return edges;
  }

  // Plan → lane roots (phases with no dependencies).
  for (const p of phases) {
    if (p.dependsOn.length === 0) {
      edges.push({ id: `${PLAN_ID}->${p.phase}`, source: PLAN_ID, target: p.phase, kind: "plan-to-lane", blocking: false });
    }
  }

  // Depends-on edges.
  for (const p of phases) {
    for (const dep of p.dependsOn) {
      if (!byId.has(dep)) continue; // dangling dep — skip rather than draw to nowhere
      edges.push({ id: `${dep}->${p.phase}`, source: dep, target: p.phase, kind: "depends", blocking: !satisfied(dep) });
    }
  }

  // Terminals (nothing depends on them) → PR Review. The integration phase is naturally one.
  for (const p of phases) {
    const hasSameRepositoryDependent = phases.some(
      (candidate) => candidate.repository === p.repository && candidate.dependsOn.includes(p.phase),
    );
    if (!hasSameRepositoryDependent) {
      const target = reviewId(p.repository);
      if (byId.has(target)) {
        edges.push({ id: `${p.phase}->${target}`, source: p.phase, target, kind: "to-review", blocking: false });
      }
    }
  }

  return edges;
}

export function reviewId(repository: string): string {
  return repository === "primary" ? REVIEW_ID : `${REVIEW_ID}:${repository}`;
}

function repositoryKey(repository: string): string {
  return repository.replace("/", "--");
}

// --- lane ordering & small helpers ---------------------------------------------------

function orderLanes(phases: PlanPhase[]): string[] {
  const seen: string[] = [];
  for (const p of phases) if (!seen.includes(p.lane)) seen.push(p.lane);
  const integ = seen.filter(isIntegrationLane);
  const rest = seen.filter((l) => !isIntegrationLane(l));
  return [...rest, ...integ];
}

function isIntegrationLane(lane: string): boolean {
  return /integration/i.test(lane);
}

function mapPlanStatus(s: PlanPhaseStatus): PhaseStateStatus {
  return s === "in-progress" ? "running" : s;
}

function mapHeaderStatus(s: string | null): PhaseStateStatus {
  if (!s) return "todo";
  const l = s.toLowerCase();
  if (l.includes("done")) return "done";
  if (l.includes("integrat") || l.includes("progress")) return "running";
  return "todo";
}

function validStateStatus(s: string | undefined): PhaseStateStatus | undefined {
  return s && (STATE_STATUSES as readonly string[]).includes(s)
    ? (s as PhaseStateStatus)
    : undefined;
}

function slugFromRunDir(runDir: string | null): string | null {
  if (!runDir) return null;
  const m = runDir.match(/runs\/(.+?)-a\d+\/?$/);
  return m ? m[1] : null;
}
