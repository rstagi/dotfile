import type {
  Runtime,
  PhaseRuntime,
  Attempt,
  ReviewRun,
  RawEvent,
  StateJson,
  MetaJson,
  StatusJson,
} from "./types.ts";

/** One run dir under `.kestral/loop/runs/`, read by the server (contents, not paths). */
export interface RawRunDir {
  /** basename, e.g. "build-feature-a2" or "review-a1". */
  name: string;
  meta: string | null;
  status: string | null;
  /** spawn.log tail — carries the exit-12 "verify failed" marker. */
  spawnLog: string | null;
  transcriptMtime: number | null;
  metaMtime: number | null;
}

/** One HIL request under `.kestral/loop/hil/`. */
export interface RawHil {
  slug: string;
  markdown: string;
  answered: boolean;
}

/** Everything the server reads out of `.kestral/loop/`, handed in as raw content. */
export interface LoopInput {
  present: boolean;
  state: string | null;
  events: string | null;
  runs: RawRunDir[];
  hil: RawHil[];
}

const RUN_NAME = /^(.+)-a(\d+)$/;
const REVIEW_NAME = /^review-a(\d+)$/;

const EMPTY: Runtime = { present: false, state: null, phases: {}, events: [], reviewRuns: [] };

/**
 * Normalize the raw `.kestral/loop/` contents into a `Runtime`. Pure — the server does
 * all I/O and mtimes; this only parses and groups defensively (every field may be absent).
 * Phase↔attempt mapping is state.json-driven: state.phases[N].slug is the run-dir prefix.
 */
export function parseLoop(input: LoopInput): Runtime {
  if (!input.present) return EMPTY;

  const state = safeJson<StateJson>(input.state);
  const events = parseEvents(input.events);
  const { reviewRuns, attemptDirs } = splitRuns(input.runs);

  // slug → phase-number, from the authoritative state.json.
  const slugToPhase = new Map<string, string>();
  for (const [num, ph] of Object.entries(state?.phases ?? {})) {
    if (ph?.slug) slugToPhase.set(ph.slug, num);
  }

  const attemptsByPhase = groupAttempts(attemptDirs, slugToPhase);
  const hilByPhase = groupHil(input.hil, slugToPhase);

  const phases: Record<string, PhaseRuntime> = {};
  const phaseNums = new Set<string>([
    ...Object.keys(state?.phases ?? {}),
    ...attemptsByPhase.keys(),
    ...hilByPhase.keys(),
  ]);
  for (const num of phaseNums) {
    phases[num] = {
      phase: num,
      state: state?.phases?.[num] ?? null,
      attempts: (attemptsByPhase.get(num) ?? []).sort((a, b) => a.k - b.k),
      hil: hilByPhase.get(num) ?? null,
    };
  }

  return { present: true, state: state ?? null, phases, events, reviewRuns };
}

// --- run dirs ------------------------------------------------------------------------

function splitRuns(runs: RawRunDir[]): { reviewRuns: ReviewRun[]; attemptDirs: RawRunDir[] } {
  const reviewRuns: ReviewRun[] = [];
  const attemptDirs: RawRunDir[] = [];
  for (const r of runs) {
    const rev = r.name.match(REVIEW_NAME);
    if (rev) {
      reviewRuns.push({ k: Number(rev[1]), runDir: `runs/${r.name}`, status: safeJson<StatusJson>(r.status) });
    } else {
      attemptDirs.push(r);
    }
  }
  reviewRuns.sort((a, b) => a.k - b.k);
  return { reviewRuns, attemptDirs };
}

function groupAttempts(
  dirs: RawRunDir[],
  slugToPhase: Map<string, string>,
): Map<string, Attempt[]> {
  const out = new Map<string, Attempt[]>();
  for (const d of dirs) {
    const m = d.name.match(RUN_NAME);
    if (!m) continue;
    const slug = m[1];
    const phase = slugToPhase.get(slug);
    if (!phase) continue; // no state mapping ⇒ cannot place on the graph
    const attempt: Attempt = {
      k: Number(m[2]),
      runDir: `runs/${d.name}`,
      meta: safeJson<MetaJson>(d.meta),
      status: safeJson<StatusJson>(d.status),
      spawnLog: d.spawnLog,
      transcriptMtime: d.transcriptMtime,
      endedAt: d.metaMtime,
    };
    const list = out.get(phase) ?? [];
    list.push(attempt);
    out.set(phase, list);
  }
  return out;
}

function groupHil(
  hil: RawHil[],
  slugToPhase: Map<string, string>,
): Map<string, PhaseRuntime["hil"]> {
  const out = new Map<string, PhaseRuntime["hil"]>();
  for (const h of hil) {
    const phase = slugToPhase.get(h.slug);
    if (!phase) continue;
    out.set(phase, { open: !h.answered, markdown: h.markdown });
  }
  return out;
}

// --- small helpers -------------------------------------------------------------------

function parseEvents(raw: string | null): RawEvent[] {
  if (!raw) return [];
  const out: RawEvent[] = [];
  for (const line of raw.split(/\r?\n/)) {
    if (!line.trim()) continue;
    const e = safeJson<RawEvent>(line);
    // Accept only plain objects — an array or scalar would slip a wrong shape into normalizeEvent.
    if (e && typeof e === "object" && !Array.isArray(e)) out.push(e);
  }
  return out;
}

function safeJson<T>(raw: string | null): T | null {
  if (raw == null) return null;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}
