import type {
  Attempt,
  AttemptSummary,
  EventTone,
  Liveness,
  ProblemClass,
  RawEvent,
  TimelineEvent,
} from "./types.ts";

/** Runner heartbeat window: transcript.jsonl older than this ⇒ hung (loop-protocol.md). */
export const DEFAULT_STALE_MS = 25 * 60 * 1000;

/** The exact loop-runner.sh stderr line ("claimed done but verify failed"), captured into the
 * run dir's spawn.log by the orchestrator's `nohup ... > spawn.log 2>&1`. It is the ONLY
 * file-observable exit-12 signal — verify.log is written on a passing verify too. */
const VERIFY_FAIL_MARKER = /claimed done but verify failed/i;

const SEVERITY: Record<ProblemClass | "hil", number> = {
  hil: 100,
  question: 80,
  blocked: 70,
  "verify-fail": 62,
  crash: 58,
  stall: 56,
  timeout: 54,
  "chain-exhausted": 50,
};

// Ordered keyword rules for the open-string `event` field (loop-state.sh log <event>).
// The orchestrator, not any script, picks these names, so match by keyword, most-specific first.
// Exported so `reduce-loop.ts` can reuse the SAME vocabulary for its legacy keyword fallback
// (a free-form `events.jsonl` "merged"/"verify-failed" line still drives the overlay).
export const EVENT_RULES: { kw: RegExp; glyph: string; label: string; tone: EventTone }[] = [
  { kw: /hil/, glyph: "⚠", label: "HIL pause", tone: "hil" },
  { kw: /verif/, glyph: "✗", label: "Verify failed", tone: "verify" },
  { kw: /crash/, glyph: "✕", label: "Crash", tone: "crash" },
  { kw: /time.?out/, glyph: "⏱", label: "Timeout", tone: "crash" },
  { kw: /escalat/, glyph: "▲", label: "Escalated", tone: null },
  { kw: /block/, glyph: "⊘", label: "Blocked", tone: "crash" },
  { kw: /question/, glyph: "?", label: "Question", tone: null },
  { kw: /answer/, glyph: "↳", label: "Answered", tone: null },
  { kw: /pr[-_ ]?(creat|open)|open[-_ ]?pr/, glyph: "⎇", label: "PR created", tone: null },
  { kw: /review/, glyph: "✓", label: "Review", tone: null },
  { kw: /merg/, glyph: "⇋", label: "Merged", tone: null },
  { kw: /spawn|launch/, glyph: "▸", label: "Runner spawned", tone: null },
  { kw: /schedul/, glyph: "◷", label: "Scheduled", tone: null },
  { kw: /claim/, glyph: "⊙", label: "Claimed", tone: null },
  { kw: /recycl/, glyph: "↻", label: "Recycled", tone: null },
  { kw: /resum/, glyph: "⏵", label: "Resumed", tone: null },
  { kw: /paus/, glyph: "⏸", label: "Paused", tone: null },
  { kw: /done|complet|finish/, glyph: "●", label: "Done", tone: null },
];

/**
 * Derive the problem class of one attempt from the files it left behind.
 *
 * Reality (from ~/dotfile/loop-runner.sh): `meta.json` existence means "attempt ended";
 * `meta.engineExit` is 124 on timeout, 40 on chain-exhausted, 0 otherwise (the semantic
 * 10/12/20/50 codes are NOT written to any file). So:
 *   timeout        ← timedOut===true OR engineExit===124
 *   chain-exhausted← engineExit===40
 *   question/blocked← status.json.outcome
 *   verify-fail    ← outcome done AND spawn.log has the exit-12 marker (verify.log alone
 *                    can't tell pass from fail — both write it)
 *   stall          ← outcome done but no new commits (headBefore === headAfter): the
 *                    orchestrator treats this as "never accept" (loop-protocol.md)
 *   crash          ← meta present but no valid outcome (exit 50)
 * Returns null while the attempt is still running (no meta.json) or ended cleanly (done+ok).
 */
export function classifyAttempt(a: Attempt): ProblemClass | null {
  const m = a.meta;
  if (!m) return null;
  if (m.timedOut === true || m.engineExit === 124) return "timeout";
  if (m.engineExit === 40) return "chain-exhausted";
  const outcome = a.status?.outcome;
  if (outcome === "question") return "question";
  if (outcome === "blocked") return "blocked";
  if (outcome === "done") {
    if (a.spawnLog && VERIFY_FAIL_MARKER.test(a.spawnLog)) return "verify-fail";
    if (isStall(m)) return "stall";
    return null;
  }
  return "crash";
}

/** exit 0 + done but no commits landed. `unknown` = a git rev-parse failure, not a stall. */
function isStall(m: NonNullable<Attempt["meta"]>): boolean {
  const { headBefore, headAfter } = m;
  return (
    !!headBefore &&
    !!headAfter &&
    headBefore !== "unknown" &&
    headBefore === headAfter
  );
}

/** An attempt has ended exactly when its meta.json exists. */
export function attemptEnded(a: Attempt): boolean {
  return a.meta != null;
}

/** live if the transcript was touched within the stale window, else flatline (hung). */
export function deriveLiveness(
  mtimeMs: number | null,
  nowMs: number,
  staleMs: number = DEFAULT_STALE_MS,
): Liveness {
  if (mtimeMs == null) return "flatline";
  return nowMs - mtimeMs <= staleMs ? "live" : "flatline";
}

/** Normalize an events.jsonl row into a display event (glyph + tone + humanized label).
 * Every field is coerced to a string first — a non-string `event` (hand-edited log, or a
 * future orchestrator) must not throw and freeze the whole snapshot. */
export function normalizeEvent(raw: RawEvent): TimelineEvent {
  const event = str(raw.event).trim();
  const key = event.toLowerCase();
  const base = {
    ts: typeof raw.ts === "string" ? raw.ts : null,
    event,
    phase: str(raw.phase),
    repository: str(raw.repository),
    detail: str(raw.detail),
  };
  for (const rule of EVENT_RULES) {
    if (key && rule.kw.test(key)) {
      return { ...base, glyph: rule.glyph, label: rule.label, known: true, tone: rule.tone };
    }
  }
  return { ...base, glyph: "•", label: humanize(event) || "Event", known: false, tone: null };
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

/** engine/model of the last leg only (the highest-K ended attempt); fallback is invisible. */
export function lastWorkedBy(attempts: Attempt[]): {
  engine: string | null;
  model: string | null;
} {
  const ended = attempts.filter((a) => a.meta).sort((a, b) => b.k - a.k);
  const m = ended[0]?.meta;
  return { engine: m?.engine || null, model: m?.model || null };
}

export function summarizeAttempt(a: Attempt): AttemptSummary {
  return {
    k: a.k,
    engine: a.meta?.engine || null,
    model: a.meta?.model || null,
    outcome: a.status?.outcome ?? null,
    problem: classifyAttempt(a),
    endedAt: a.endedAt,
    ended: attemptEnded(a),
  };
}

export function problemSeverity(cls: ProblemClass | "hil"): number {
  return SEVERITY[cls] ?? 0;
}

function humanize(name: string): string {
  const t = name.replace(/[-_]+/g, " ").replace(/\s+/g, " ").trim();
  return t ? t[0].toUpperCase() + t.slice(1) : "";
}
