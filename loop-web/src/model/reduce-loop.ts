// The daemon's pure fold: (LoopRecord, Ingest) → LoopRecord. No I/O or wall-clock reads —
// the server supplies timestamped messages. Owns the promotion lattice and the event
// vocabulary; `materialize.ts` turns the resulting record into a `Snapshot`.
//
// Contract, in one line: a phase's rank only ever advances (monotone max over
// `todo<claimed<running<done<merged`), so a `phase.attempt.finish{done,exit0}` event
// promotes a work phase to `done` even while state.json still says `running` — the staleness
// fix. Explicit review phases advance only from their aggregate pipeline state.

import type {
  Ingest,
  LoopRecord,
  LoopStatus,
  PhaseOverlay,
  PhaseRank,
  EventInfo,
  FinishInfo,
  ReviewInfo,
  RepositoryRecord,
} from "./store-types.ts";
import { STORE_SCHEMA_VERSION, EVENT_CAP } from "./store-types.ts";
import type { StateJson, RawEvent, PhaseStateStatus, JevDecision, JevMode } from "./types.ts";
import { EVENT_RULES } from "./derive.ts";
import { parsePlan } from "./parse-plan.ts";

const RANK_INDEX: Record<PhaseRank, number> = { todo: 0, claimed: 1, running: 2, done: 3, merged: 4 };
const STATE_STATUSES: readonly PhaseStateStatus[] = ["todo", "claimed", "running", "merged", "blocked", "done", "paused"];

/** Event names the reducer folds structurally. Anything else falls to the keyword fallback
 * (EVENT_RULES) so a legacy free-form events.jsonl still drives the overlay. */
const TYPED_EVENTS = new Set([
  "phase.attempt.start",
  "phase.attempt.finish",
  "phase.merged",
  "merge.conflict",
  "hil.raise",
  "hil.resolve",
  "sub.recycle",
  "sub.saturation",
  "review.finish",
  "loop.finish",
  "jev.decision",
  // User controls (POST /control): timeline-only — pause state is read from control/ files.
  "control.pause",
  "control.resume",
  "control.model",
]);

/** Effect of one event on the record (all fields optional; `phase`/`patch` drive the overlay). */
interface EventEffect {
  phase: string | null;
  patch: Partial<PhaseOverlay> | null;
  prUrl?: string | null;
  review?: ReviewInfo | null;
  finish?: FinishInfo | null;
  /** Loop-level context occupancy heartbeat (last-write-wins). */
  occupancy?: { tokens: number; percent: number | null } | null;
  /** Loop-level ABSOLUTE recycle count to max-fold (idempotent under re-fold), not a delta. */
  subRecycles?: number;
}

// ---------------------------------------------------------------------------------------
// The fold
// ---------------------------------------------------------------------------------------

export function reduceLoop(prev: LoopRecord, msg: Ingest): LoopRecord {
  switch (msg.kind) {
    case "register":
      return applyRegister(prev, msg);
    case "state":
      return applyState(prev, msg.state, msg.planText);
    case "event":
      return applyEvent(prev, msg.event);
    case "eventsFile":
      return applyEventsFile(prev, msg.text);
    case "finish":
      return applyFinish(prev, msg.info);
  }
}

export function emptyRecord(runId: string): LoopRecord {
  return {
    schemaVersion: STORE_SCHEMA_VERSION,
    runId,
    effort: null,
    projectId: null,
    loopDir: null,
    planFile: null,
    integrationBranch: null,
    startedAt: null,
    finishedAt: null,
    status: "active",
    lastState: null,
    planText: null,
    phases: {},
    repositories: {},
    events: [],
    decisions: [],
    review: null,
    prUrl: null,
    lastSnapshot: null,
    updatedAt: null,
    subRecycles: 0,
    occupancy: null,
  };
}

/**
 * The effective lifecycle status for one phase. A `done`/`merged` rank OVERRIDES the live
 * state (the staleness fix, never regresses); below that, the live state.json status wins
 * (so `blocked` shows and can later move back to `running`), falling back to the rank when
 * no live status exists yet (e.g. an `attempt.start` before the first state push).
 */
export function effectivePhaseStatus(rec: LoopRecord, num: string): PhaseStateStatus {
  const overlay = rec.phases[num];
  const live = validStatus(rec.lastState?.phases?.[num]?.status);
  const planned = planPhaseStatus(rec.planText, num);
  const rank = joinRank(overlay?.rank ?? "todo", rankOf(live ?? planned));
  if (rank === "merged") return "merged";
  if (rank === "done") return "done";
  if (live) return live;
  if (planned === "blocked" && rank === "todo") return "blocked";
  return rankToStatus(rank);
}

// ---------------------------------------------------------------------------------------
// register / state / finish
// ---------------------------------------------------------------------------------------

function applyRegister(prev: LoopRecord, msg: Extract<Ingest, { kind: "register" }>): LoopRecord {
  const info = msg.info;
  const reopened = prev.status === "finished" && hasNewUnfinishedPhase(prev.planText, info.planText ?? null);
  const repositories = info.planText ? repositoriesFromPlan(info.planText, prev.repositories) : prev.repositories;
  const primary = firstRepository(repositories);
  const next: LoopRecord = {
    ...prev,
    runId: info.runId || prev.runId,
    effort: info.effort ?? prev.effort,
    projectId: info.projectId ?? prev.projectId,
    loopDir: info.loopDir ?? prev.loopDir,
    planFile: info.planFile ?? prev.planFile,
    integrationBranch: info.integrationBranch ?? primary?.integrationBranch ?? prev.integrationBranch,
    startedAt: info.startedAt ?? prev.startedAt,
    finishedAt: reopened ? null : prev.finishedAt,
    planText: info.planText ?? prev.planText,
    repositories,
    updatedAt: latestTimestamp(prev.updatedAt, info.startedAt),
  };
  return { ...next, status: reopened ? "active" : deriveStatus(next) };
}

function hasNewUnfinishedPhase(previousText: string | null, nextText: string | null): boolean {
  if (!previousText || !nextText) return false;
  const previousIds = new Set(parsePlan(previousText).phases.map((phase) => phase.phase));
  return parsePlan(nextText).phases.some(
    (phase) => !previousIds.has(phase.phase) && phase.status !== "done",
  );
}

function repositoriesFromPlan(
  planText: string,
  existing: Record<string, RepositoryRecord>,
): Record<string, RepositoryRecord> {
  const out = { ...existing };
  for (const repository of parsePlan(planText).repositories) {
    const prior = out[repository.slug] ?? emptyRepository();
    out[repository.slug] = {
      ...prior,
      integrationBranch: prior.integrationBranch ?? repository.integrationBranch,
      prUrl: prior.prUrl ?? repository.pr,
    };
  }
  return out;
}

function applyState(prev: LoopRecord, state: StateJson, planText?: string | null): LoopRecord {
  const phases = { ...prev.phases };
  for (const [num, ph] of Object.entries(state?.phases ?? {})) {
    phases[num] = joinOverlay(phases[num], {
      rank: rankOf(validStatus(ph?.status)),
      repository: nonEmpty(ph?.repository) ?? phases[num]?.repository ?? legacyRepositorySlug(state),
    });
  }
  const repositories = normalizeRepositories(state, prev.repositories);
  const primary = firstRepository(repositories);
  const next: LoopRecord = {
    ...prev,
    lastState: state ?? prev.lastState,
    effort: state?.effort ?? prev.effort,
    projectId: state?.projectId ?? prev.projectId,
    integrationBranch: state?.integrationBranch ?? prev.integrationBranch,
    prUrl: state?.prUrl ?? prev.prUrl,
    review: normalizeReview(state?.review) ?? prev.review,
    repositories,
    planText: planText ?? prev.planText,
    phases,
  };
  return { ...next, integrationBranch: next.integrationBranch ?? primary?.integrationBranch ?? null,
    prUrl: next.prUrl ?? primary?.prUrl ?? null,
    review: next.review ?? primary?.review ?? null, status: deriveStatus(next) };
}

function applyFinish(prev: LoopRecord, info: FinishInfo): LoopRecord {
  if (!terminalReviewComplete(prev)) return prev;
  const repositories = { ...prev.repositories };
  for (const [slug, result] of Object.entries(info.repositories ?? {})) {
    repositories[slug] = {
      ...(repositories[slug] ?? emptyRepository()),
      prUrl: result.prUrl ?? repositories[slug]?.prUrl ?? null,
      review: result.review ?? repositories[slug]?.review ?? null,
    };
  }
  const primary = firstRepository(repositories);
  return {
    ...prev,
    finishedAt: info.finishedAt ?? prev.finishedAt,
    repositories,
    prUrl: info.prUrl ?? primary?.prUrl ?? prev.prUrl,
    review: info.review ?? primary?.review ?? prev.review,
    updatedAt: latestTimestamp(prev.updatedAt, info.finishedAt),
    status: "finished",
  };
}

function terminalReviewComplete(record: LoopRecord): boolean {
  if (!record.planText) return true;
  const phases = parsePlan(record.planText).phases;
  if (!phases.some((phase) => phase.kind === "pr-review")) return true;
  const terminal = phases.at(-1);
  if (!terminal || terminal.kind !== "pr-review") return false;
  const status = effectivePhaseStatus(record, terminal.phase);
  return status === "done" || status === "merged";
}

// ---------------------------------------------------------------------------------------
// events
// ---------------------------------------------------------------------------------------

function applyEvent(prev: LoopRecord, ev: EventInfo): LoopRecord {
  // sub.saturation is a high-frequency occupancy heartbeat — fold its effect but keep it OUT
  // of the timeline/store, so it can't flood record.events or evict real lifecycle events at
  // the EVENT_CAP. (sub.recycle IS a real milestone and stays in the timeline.)
  const heartbeat = (ev.event ?? "").trim().toLowerCase() === "sub.saturation";
  const events = heartbeat ? prev.events : mergeEvents(prev.events, [toRawEvent(ev)]);
  const eff = eventSemantics(ev, prev.planText);
  const decision = decisionFromEvent(prev.runId, ev);
  const decisions = decision ? mergeDecisions(prev.decisions, [decision]) : prev.decisions;
  let next: LoopRecord = { ...prev, events, decisions, updatedAt: latestTimestamp(prev.updatedAt, ev.ts) };
  if (eff.finish) return applyFinish(next, eff.finish);
  if (eff.phase && eff.patch) {
    const patch = { ...eff.patch, repository: nonEmpty(ev.repository) ?? eff.patch.repository };
    next = { ...next, phases: { ...next.phases, [eff.phase]: joinOverlay(next.phases[eff.phase], patch) } };
  }
  if (eff.prUrl) next = { ...next, prUrl: eff.prUrl };
  if (eff.prUrl && ev.repository) {
    const prior = next.repositories[ev.repository] ?? emptyRepository();
    next = { ...next, repositories: { ...next.repositories,
      [ev.repository]: { ...prior, prUrl: eff.prUrl } } };
  }
  if (eff.review) next = { ...next, review: eff.review };
  if (eff.occupancy !== undefined) next = { ...next, occupancy: eff.occupancy };
  if (eff.subRecycles !== undefined) next = { ...next, subRecycles: Math.max(next.subRecycles, eff.subRecycles) };
  return { ...next, status: deriveStatus(next) };
}

function applyEventsFile(prev: LoopRecord, text: string): LoopRecord {
  return parseJsonlEvents(text).reduce((rec, raw) => applyEvent(rec, rawToEventInfo(raw)), prev);
}

function eventSemantics(ev: EventInfo, planText: string | null): EventEffect {
  const phase = nonEmpty(ev.phase);
  const name = (ev.event ?? "").trim().toLowerCase();
  if (TYPED_EVENTS.has(name)) return typedSemantics(name, ev, phase, planText);
  return keywordSemantics(name, phase);
}

function typedSemantics(
  name: string,
  ev: EventInfo,
  phase: string | null,
  planText: string | null,
): EventEffect {
  switch (name) {
    case "phase.attempt.start":
      return { phase, patch: phase ? { rank: "running" } : null };
    case "phase.attempt.finish": {
      if (!phase) return { phase, patch: null };
      if (ev.exitCode === 12) return { phase, patch: { problem: "verify-fail" } };
      const outcome = (ev.outcome ?? "").toLowerCase();
      if (outcome === "done" && (ev.exitCode == null || ev.exitCode === 0)) {
        if (isExplicitReviewPhase(planText, phase)) return { phase, patch: { problem: null } };
        return { phase, patch: { rank: "done", problem: null } };
      }
      const problem = outcome && outcome !== "done" ? outcome : ev.exitCode ? `exit-${ev.exitCode}` : null;
      return { phase, patch: { problem } };
    }
    case "phase.merged":
      return { phase, patch: phase ? { rank: "merged", problem: null } : null, prUrl: ev.prUrl ?? undefined };
    case "merge.conflict":
      return { phase, patch: phase ? { problem: "merge-conflict" } : null };
    case "hil.raise":
      return { phase, patch: phase ? { hilOpen: true } : null };
    case "hil.resolve":
      return { phase, patch: phase ? { hilOpen: false } : null };
    case "sub.recycle":
      return {
        phase: null,
        patch: null,
        occupancy: ev.tokens != null ? { tokens: ev.tokens, percent: ev.percent ?? null } : undefined,
        subRecycles: ev.recycleIndex ?? undefined,
      };
    case "sub.saturation":
      return {
        phase: null,
        patch: null,
        occupancy: ev.tokens != null ? { tokens: ev.tokens, percent: ev.percent ?? null } : undefined,
      };
    case "loop.finish":
      return { phase: null, patch: null, finish: { finishedAt: ev.ts ?? null, prUrl: ev.prUrl ?? null } };
    case "jev.decision":
      return { phase, patch: null };
    default:
      // review.finish and any other typed name: timeline-only (review is owned by state/finish).
      return { phase, patch: null };
  }
}

function isExplicitReviewPhase(planText: string | null, phase: string): boolean {
  if (!planText) return false;
  return parsePlan(planText).phases.some(
    (plannedPhase) => plannedPhase.phase === phase && plannedPhase.kind === "pr-review",
  );
}

/** Legacy fallback: reuse derive.ts EVENT_RULES so a free-form events.jsonl still promotes. */
function keywordSemantics(name: string, phase: string | null): EventEffect {
  if (!phase || !name) return { phase, patch: null };
  switch (matchLabel(name)) {
    case "Merged":
      return { phase, patch: { rank: "merged", problem: null } };
    case "Done":
      return { phase, patch: { rank: "done", problem: null } };
    case "Verify failed":
      return { phase, patch: { problem: "verify-fail" } };
    case "HIL pause":
      return { phase, patch: { hilOpen: true } };
    default:
      return { phase, patch: null };
  }
}

function matchLabel(name: string): string | null {
  for (const rule of EVENT_RULES) if (rule.kw.test(name)) return rule.label;
  return null;
}

// ---------------------------------------------------------------------------------------
// overlay & rank helpers
// ---------------------------------------------------------------------------------------

function joinOverlay(existing: PhaseOverlay | undefined, patch: Partial<PhaseOverlay>): PhaseOverlay {
  const base: PhaseOverlay = existing ?? { rank: "todo", hilOpen: false, problem: null, repository: "primary" };
  return {
    rank: patch.rank ? joinRank(base.rank, patch.rank) : base.rank,
    hilOpen: patch.hilOpen ?? base.hilOpen,
    problem: patch.problem !== undefined ? patch.problem : base.problem,
    repository: patch.repository ?? base.repository,
  };
}

function joinRank(a: PhaseRank, b: PhaseRank): PhaseRank {
  return RANK_INDEX[b] > RANK_INDEX[a] ? b : a;
}

/** state.json status → lattice rank. `blocked`/unknown → todo (blocked is not a rank; it
 * shows through as a live sub-`done` status instead). */
function rankOf(status: PhaseStateStatus | null): PhaseRank {
  switch (status) {
    case "merged":
      return "merged";
    case "done":
      return "done";
    case "running":
      return "running";
    case "claimed":
      return "claimed";
    default:
      return "todo";
  }
}

function rankToStatus(rank: PhaseRank): PhaseStateStatus {
  return rank; // PhaseRank ⊂ PhaseStateStatus
}

function deriveStatus(rec: LoopRecord): LoopStatus {
  if (rec.status === "finished" || rec.finishedAt) return "finished";
  if (Object.values(rec.phases).some((o) => o.hilOpen)) return "paused";
  // A register-only record (no state push, no lifecycle event) is a plan registered but not
  // yet running; the first state push or event flips it to active. No new ingest field needed.
  if (rec.lastState === null && !rec.events.some((event) => event.event !== "jev.decision")) return "planned";
  return "active";
}

// ---------------------------------------------------------------------------------------
// timeline & small helpers
// ---------------------------------------------------------------------------------------

function mergeEvents(existing: RawEvent[], incoming: RawEvent[]): RawEvent[] {
  const seen = new Set(existing.map(eventKey));
  const out = existing.slice();
  for (const e of incoming) {
    const k = eventKey(e);
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(e);
  }
  return out.length > EVENT_CAP ? out.slice(out.length - EVENT_CAP) : out;
}

function eventKey(e: RawEvent): string {
  return `${e.ts ?? ""}|${e.event ?? ""}|${e.phase ?? ""}|${e.repository ?? ""}|${e.detail ?? ""}|${e.attempt ?? ""}|${e.stage ?? ""}|${e.head ?? ""}`;
}

function toRawEvent(ev: EventInfo): RawEvent {
  return {
    runId: ev.runId ?? undefined, ts: ev.ts ?? undefined, event: ev.event, phase: ev.phase ?? "",
    repository: ev.repository ?? undefined, detail: ev.detail ?? "", attempt: ev.attempt ?? undefined,
    stage: ev.stage ?? undefined, mode: ev.mode ?? undefined, candidate: ev.candidate,
    confidence: ev.confidence, probabilities: ev.probabilities ?? undefined,
    appliedAction: ev.appliedAction, fallbackReason: ev.fallbackReason,
    resolvedModel: ev.resolvedModel, evidenceChecked: ev.evidenceChecked ?? undefined,
    evidenceSources: ev.evidenceSources ?? undefined, questionRound: ev.questionRound ?? undefined,
    head: ev.head ?? undefined, requiredGates: ev.requiredGates ?? undefined,
    completedGates: ev.completedGates ?? undefined, remainingGates: ev.remainingGates ?? undefined,
    focus: ev.focus ?? undefined,
  };
}

function rawToEventInfo(raw: RawEvent): EventInfo {
  return {
    event: str(raw.event), runId: str(raw.runId), phase: str(raw.phase), repository: str(raw.repository),
    detail: str(raw.detail), ts: typeof raw.ts === "string" ? raw.ts : null,
    attempt: typeof raw.attempt === "number" ? raw.attempt : null, stage: raw.stage ?? null,
    mode: raw.mode ?? null, candidate: raw.candidate, confidence: raw.confidence,
    probabilities: raw.probabilities, appliedAction: raw.appliedAction,
    fallbackReason: raw.fallbackReason, resolvedModel: raw.resolvedModel,
    evidenceChecked: raw.evidenceChecked, evidenceSources: raw.evidenceSources,
    questionRound: raw.questionRound,
    head: raw.head, requiredGates: raw.requiredGates, completedGates: raw.completedGates,
    remainingGates: raw.remainingGates, focus: raw.focus,
  };
}

function decisionFromEvent(runId: string, ev: EventInfo): JevDecision | null {
  if (ev.event !== "jev.decision" || !nonEmpty(ev.phase) || !nonEmpty(ev.stage) || !isJevMode(ev.mode)) return null;
  if (!Number.isInteger(ev.attempt) || Number(ev.attempt) < 1) return null;
  return {
    runId: nonEmpty(ev.runId) ?? runId,
    phase: nonEmpty(ev.phase)!,
    attempt: Number(ev.attempt),
    stage: nonEmpty(ev.stage)!,
    mode: ev.mode,
    candidate: nonEmpty(ev.candidate),
    confidence: typeof ev.confidence === "number" ? ev.confidence : null,
    probabilities: ev.probabilities ?? {},
    appliedAction: nonEmpty(ev.appliedAction),
    fallbackReason: nonEmpty(ev.fallbackReason),
    resolvedModel: nonEmpty(ev.resolvedModel),
    evidenceChecked: ev.evidenceChecked === true,
    evidenceSources: Array.isArray(ev.evidenceSources)
      ? ev.evidenceSources.filter((value): value is string => typeof value === "string" && value.length > 0)
      : [],
    questionRound: Number.isInteger(ev.questionRound) ? Number(ev.questionRound) : null,
    head: nonEmpty(ev.head) ?? undefined,
    requiredGates: stringList(ev.requiredGates),
    completedGates: stringList(ev.completedGates),
    remainingGates: stringList(ev.remainingGates),
    focus: stringList(ev.focus),
    ts: ev.ts ?? null,
  };
}

function stringList(value: string[] | null | undefined): string[] | undefined {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string" && item.length > 0)
    : undefined;
}

function mergeDecisions(existing: JevDecision[], incoming: JevDecision[]): JevDecision[] {
  const seen = new Set(existing.map(decisionKey));
  return existing.concat(incoming.filter((decision) => {
    const key = decisionKey(decision);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  })).sort(compareDecisions);
}

function compareDecisions(left: JevDecision, right: JevDecision): number {
  const leftTime = timestampMillis(left.ts);
  const rightTime = timestampMillis(right.ts);
  if (leftTime !== rightTime) return leftTime < rightTime ? -1 : 1;
  const leftKey = decisionKey(left);
  const rightKey = decisionKey(right);
  return leftKey < rightKey ? -1 : leftKey > rightKey ? 1 : 0;
}

function decisionKey(decision: JevDecision): string {
  return `${decision.runId}|${decision.phase}|${decision.attempt}|${decision.stage}|${decision.ts ?? ""}|${decision.head ?? ""}`;
}

function isJevMode(value: unknown): value is JevMode {
  return value === "off" || value === "shadow" || value === "active";
}

function emptyRepository(): RepositoryRecord {
  return { sourceRoot: null, defaultBranch: null, baseSha: null, integrationBranch: null,
    integrationWorktree: null, prUrl: null, review: null };
}

function normalizeRepositories(
  state: StateJson,
  existing: Record<string, RepositoryRecord>,
): Record<string, RepositoryRecord> {
  const out = { ...existing };
  for (const [slug, repo] of Object.entries(state.repositories ?? {})) {
    const prior = out[slug] ?? emptyRepository();
    out[slug] = {
      sourceRoot: repo.sourceRoot ?? prior.sourceRoot,
      defaultBranch: repo.defaultBranch ?? prior.defaultBranch,
      baseSha: repo.baseSha ?? prior.baseSha,
      integrationBranch: repo.integrationBranch ?? prior.integrationBranch,
      integrationWorktree: repo.integrationWorktree ?? prior.integrationWorktree,
      prUrl: repo.prUrl ?? prior.prUrl,
      review: normalizeReview(repo.review ?? undefined) ?? prior.review,
    };
  }
  if (Object.keys(out).length === 0 && (state.integrationBranch || state.prUrl || state.review)) {
    out.primary = {
      ...emptyRepository(),
      integrationBranch: state.integrationBranch ?? null,
      prUrl: state.prUrl ?? null,
      review: normalizeReview(state.review) ?? null,
    };
  }
  return out;
}

function firstRepository(repositories: Record<string, RepositoryRecord>): RepositoryRecord | null {
  return Object.values(repositories)[0] ?? null;
}

function legacyRepositorySlug(state: StateJson): string {
  return Object.keys(state.repositories ?? {})[0] ?? "primary";
}

function parseJsonlEvents(text: string): RawEvent[] {
  const out: RawEvent[] = [];
  for (const line of text.split(/\r?\n/)) {
    if (!line.trim()) continue;
    try {
      const e = JSON.parse(line);
      if (e && typeof e === "object" && !Array.isArray(e)) out.push(e as RawEvent);
    } catch {
      /* skip a malformed line */
    }
  }
  return out;
}

function normalizeReview(r: {
  outcome?: string | null;
  summary?: string | null;
  reportPath?: string | null;
  commentUrl?: string | null;
} | null | undefined): ReviewInfo | null {
  if (!r) return null;
  return {
    outcome: r.outcome ?? null,
    summary: r.summary ?? null,
    reportPath: r.reportPath ?? null,
    commentUrl: r.commentUrl ?? null,
  };
}

function validStatus(s: string | undefined): PhaseStateStatus | null {
  return s && (STATE_STATUSES as readonly string[]).includes(s) ? (s as PhaseStateStatus) : null;
}

function planPhaseStatus(planText: string | null, num: string): PhaseStateStatus | null {
  if (!planText) return null;
  const status = parsePlan(planText).phases.find((phase) => phase.phase === num)?.status;
  return status === "in-progress" ? "running" : status ?? null;
}

function nonEmpty(s: string | null | undefined): string | null {
  return s && s.trim() ? s.trim() : null;
}

function latestTimestamp(previous: string | null, incoming: string | null | undefined): string | null {
  if (!incoming) return previous;
  return !previous || timestampMillis(incoming) > timestampMillis(previous) ? incoming : previous;
}

function timestampMillis(value: string | null | undefined): number {
  if (!value) return Number.NEGATIVE_INFINITY;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? Number.NEGATIVE_INFINITY : parsed;
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}
