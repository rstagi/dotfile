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
  RepositoryInfo,
  JevDecision,
  JevStatus,
} from "./types.ts";
import { buildGraph } from "./build-graph.ts";
import { normalizeEvent, problemSeverity } from "./derive.ts";

interface SnapshotOpts {
  now?: number;
  nowIso?: string | null;
  decisions?: JevDecision[];
}

/**
 * Compose the full browser-facing `Snapshot` from a parsed plan and (optional) runtime.
 * This is the single entry point the server calls per debounced change / reconcile.
 */
export function buildSnapshot(plan: Plan, runtime: Runtime | null, opts: SnapshotOpts = {}): Snapshot {
  const now = opts.now ?? Date.now();
  const decisions = opts.decisions ?? decisionsFromRuntime(runtime);
  const repositories = buildRepositoryInfo(plan, runtime);
  const repositoryPrs = Object.fromEntries(repositories.map((repository) => [repository.slug, repository.pr]));
  const pr = repositories.length === 1 ? repositories[0].pr : null;
  const overview = buildPlanOverview(plan, runtime, repositories);
  const graph = buildGraph(plan, runtime, { now, pr, repositories: repositoryPrs, decisions });
  graph.nodes = graph.nodes.map((node) => {
    if (node.kind !== "pr-review" || !node.phase) return node;
    const phase = plan.phases.find((entry) => entry.phase === node.phase);
    return { ...node, prs: overview.milestones.find((entry) => entry.number === phase?.milestone)?.prs ?? {} };
  });

  return {
    effort: buildEffort(plan, runtime, pr, repositories),
    plan: overview,
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
    decisions,
    jev: summarizeJev(decisions),
    paused: runtime?.control?.paused ?? false,
  };
}

function decisionsFromRuntime(runtime: Runtime | null): JevDecision[] {
  const out: JevDecision[] = [];
  const seen = new Set<string>();
  for (const event of runtime?.events ?? []) {
    if (event.event !== "jev.decision" || !event.phase || !event.stage || !event.mode || !Number.isInteger(event.attempt)) continue;
    const decision: JevDecision = {
      runId: event.runId ?? runtime?.state?.runId ?? "",
      phase: event.phase,
      attempt: event.attempt!,
      stage: event.stage,
      mode: event.mode,
      candidate: event.candidate ?? null,
      confidence: typeof event.confidence === "number" ? event.confidence : null,
      probabilities: event.probabilities ?? {},
      appliedAction: event.appliedAction ?? null,
      fallbackReason: event.fallbackReason ?? null,
      resolvedModel: event.resolvedModel ?? null,
      evidenceChecked: event.evidenceChecked === true,
      evidenceSources: event.evidenceSources ?? [],
      questionRound: Number.isInteger(event.questionRound) ? event.questionRound! : null,
      head: event.head ?? undefined,
      requiredGates: event.requiredGates,
      completedGates: event.completedGates,
      remainingGates: event.remainingGates,
      focus: event.focus,
      ts: event.ts ?? null,
    };
    const key = `${decision.runId}|${decision.phase}|${decision.attempt}|${decision.stage}|${decision.ts ?? ""}|${decision.head ?? ""}`;
    if (!seen.has(key)) { seen.add(key); out.push(decision); }
  }
  return out;
}

function summarizeJev(decisions: JevDecision[]): JevStatus | null {
  const latest = decisions.at(-1);
  if (!latest) return null;
  return {
    mode: latest.mode,
    count: decisions.length,
    fallbackCount: decisions.filter((decision) => decision.fallbackReason != null).length,
  };
}

// --- effort & PR ---------------------------------------------------------------------

function buildEffort(
  plan: Plan,
  runtime: Runtime | null,
  pr: PrInfo | null,
  repositories: RepositoryInfo[],
): EffortInfo {
  return {
    name: plan.name,
    status: plan.status,
    milestone: runtime?.state?.milestone ?? 1,
    milestoneTotal: Math.max(1, plan.milestones.length),
    awaitingMerge: runtime?.state?.awaitingMerge ?? null,
    integrationBranch: effortIntegrationBranch(plan, runtime, repositories),
    updatedAt: plan.updatedAt,
    pr,
    repositories,
    runId: runtime?.state?.runId ?? null,
  };
}

function effortIntegrationBranch(plan: Plan, runtime: Runtime | null, repositories: RepositoryInfo[]): string | null {
  if (repositories.length === 1) return repositories[0].integrationBranch;
  if (runtime?.state?.integrationBranch !== undefined) return runtime.state.integrationBranch;
  const branches = new Set(repositories.map((repository) => repository.integrationBranch));
  if (branches.size === 1) return repositories[0].integrationBranch;
  return (runtime?.state?.milestone ?? 1) === 1 ? plan.loopConfig?.integrationBranch ?? null : null;
}

function buildRepositoryInfo(plan: Plan, runtime: Runtime | null): RepositoryInfo[] {
  const milestone = plan.milestones.find((entry) => entry.number === (runtime?.state?.milestone ?? 1));
  return plan.repositories.filter((repository) => !milestone || milestone.participants.includes(repository.slug)).map((repository) => {
    const stateRepository = runtime?.state?.repositories?.[repository.slug];
    return {
      slug: repository.slug,
      integrationBranch: stateRepository?.integrationBranch !== undefined
        ? stateRepository.integrationBranch
        : (repository.slug === "primary" ? runtime?.state?.integrationBranch : null)
          ?? (milestone ? milestone.integrationBranches[repository.slug] ?? null : repository.integrationBranch),
      verify: repository.verify,
      pr: buildPrInfo(plan, runtime, repository.slug),
    };
  });
}

/** Prefer the orchestrator-promoted `state.review` verdict; fall back to the latest review run's
 * status.json. reportPath/commentUrl only exist on state.review; slug/attempt on the run. */
function buildPrInfo(plan: Plan, runtime: Runtime | null, repository: string): PrInfo | null {
  const legacy = repository === "primary";
  const stateRepository = runtime?.state?.repositories?.[repository];
  const planRepository = plan.repositories.find((entry) => entry.slug === repository);
  const milestoneNumber = runtime?.state?.milestone ?? 1;
  const milestone = plan.milestones.find((entry) => entry.number === milestoneNumber);
  const awaiting = runtime?.state?.awaitingMerge;
  const url = (awaiting?.milestone === milestoneNumber ? awaiting.prs[repository] : null)
    ?? (stateRepository?.prUrl !== undefined ? stateRepository.prUrl
      : (legacy ? runtime?.state?.prUrl : null)
        ?? (milestone ? milestone.prs[repository] ?? null : planRepository?.pr ?? null));
  const key = repositoryKey(repository);
  const allReviews = legacy
    ? [...runtime?.reviewRuns ?? [], ...Object.values(runtime?.reviewRunsByRepository ?? {}).flat()]
    : runtime?.reviewRunsByRepository[key] ?? [];
  const reviewPhases = plan.phases.filter((phase) => phase.milestone === milestoneNumber && phase.kind === "pr-review");
  const reviews = allReviews.filter((run) => run.phase == null
    ? milestoneNumber === 1
    : reviewPhases.some((phase) => phase.phase === run.phase))
    .sort((a, b) => reviewPhases.findIndex((phase) => phase.phase === a.phase)
      - reviewPhases.findIndex((phase) => phase.phase === b.phase) || a.k - b.k);
  const latest = reviews.length ? reviews[reviews.length - 1] : null;
  const stateReview = stateRepository?.review ?? (legacy ? runtime?.state?.review : null) ?? null;
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

function repositoryKey(repository: string): string {
  return repository.replace("/", "--");
}

function slugFromRunDir(runDir: string | null): string | null {
  if (!runDir) return null;
  const m = runDir.match(/runs\/(.+?)-a\d+\/?$/);
  return m ? m[1] : null;
}

// --- plan overview -------------------------------------------------------------------

function buildPlanOverview(plan: Plan, runtime: Runtime | null, repositories: RepositoryInfo[]): PlanOverview {
  const lanes = new Set(plan.phases.map((p) => p.lane));
  return {
    name: plan.name,
    status: plan.status,
    updatedAt: plan.updatedAt,
    loopConfig: plan.loopConfig,
    milestones: plan.milestones.map((milestone) => ({
      ...milestone,
      prs: {
        ...milestone.prs,
        ...runtime?.state?.milestones?.[String(milestone.number)]?.prs,
        ...(milestone.number === (runtime?.state?.milestone ?? 1)
          ? Object.fromEntries(repositories.map((repository) => [repository.slug, repository.pr?.url ?? null])) : {}),
      },
    })),
    repositories: plan.repositories.map((repository) => {
      const runtime = repositories.find((entry) => entry.slug === repository.slug);
      return {
        ...repository,
        integrationBranch: runtime?.integrationBranch ?? null,
        pr: runtime?.pr?.url ?? null,
      };
    }),
    phaseSummary: plan.phases.map((p) => ({
      phase: p.phase,
      milestone: p.milestone,
      title: p.title,
      kind: p.kind,
      lane: p.lane,
      status: p.status,
      repository: p.repository,
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
