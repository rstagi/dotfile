import { describe, it, expect } from "vitest";
import {
  classifyAttempt,
  attemptEnded,
  deriveLiveness,
  normalizeEvent,
  lastWorkedBy,
  summarizeAttempt,
  problemSeverity,
  DEFAULT_STALE_MS,
} from "./derive.ts";
import type { Attempt, MetaJson, StatusJson } from "./types.ts";

function attempt(over: Partial<Attempt> & { k?: number } = {}): Attempt {
  return {
    k: over.k ?? 1,
    runDir: over.runDir ?? `.loop/runs/slug-a${over.k ?? 1}`,
    meta: over.meta ?? null,
    status: over.status ?? null,
    spawnLog: over.spawnLog ?? null,
    transcriptMtime: over.transcriptMtime ?? null,
    endedAt: over.endedAt ?? null,
  };
}
const meta = (o: MetaJson): MetaJson => o;
const status = (o: StatusJson): StatusJson => o;

// The classification is grounded in loop-runner.sh's exit-code paths, NOT the idealized docs:
// meta.engineExit ∈ {0, 40, 124}; the 10/12/20/50 distinction needs status.json + spawn.log.
describe("classifyAttempt — problem derivation", () => {
  it("returns null while the attempt is still in flight (no meta.json)", () => {
    expect(classifyAttempt(attempt({ meta: null }))).toBeNull();
  });

  it("timeout ← meta.timedOut === true", () => {
    expect(
      classifyAttempt(attempt({ meta: meta({ engineExit: 124, timedOut: true }) })),
    ).toBe("timeout");
  });

  it("timeout ← meta.engineExit === 124 even if timedOut is missing", () => {
    expect(classifyAttempt(attempt({ meta: meta({ engineExit: 124 }) }))).toBe(
      "timeout",
    );
  });

  it("chain-exhausted ← meta.engineExit === 40 (observable, not unobservable)", () => {
    expect(classifyAttempt(attempt({ meta: meta({ engineExit: 40 }) }))).toBe(
      "chain-exhausted",
    );
  });

  it("question ← status.json outcome question", () => {
    expect(
      classifyAttempt(
        attempt({ meta: meta({ engineExit: 0 }), status: status({ outcome: "question" }) }),
      ),
    ).toBe("question");
  });

  it("blocked ← status.json outcome blocked", () => {
    expect(
      classifyAttempt(
        attempt({ meta: meta({ engineExit: 0 }), status: status({ outcome: "blocked" }) }),
      ),
    ).toBe("blocked");
  });

  it("crash ← meta present but no valid status.json outcome (exit 50)", () => {
    expect(classifyAttempt(attempt({ meta: meta({ engineExit: 0 }), status: null }))).toBe(
      "crash",
    );
    expect(
      classifyAttempt(
        attempt({ meta: meta({ engineExit: 0 }), status: status({ outcome: "weird" }) }),
      ),
    ).toBe("crash");
  });

  it("a passing verify (outcome done, no marker, commits landed) is NOT a problem", () => {
    // Both a passing and a failing verify write verify.log — presence alone can't tell them
    // apart. So a done attempt with no marker and real commits must NOT be flagged.
    expect(
      classifyAttempt(
        attempt({
          meta: meta({ engineExit: 0, headBefore: "aaa", headAfter: "bbb" }),
          status: status({ outcome: "done" }),
          spawnLog: "loop-runner: done\n",
        }),
      ),
    ).toBeNull();
  });

  it("verify-fail ← the spawn.log marker (exit 12 disambiguator)", () => {
    expect(
      classifyAttempt(
        attempt({
          meta: meta({ engineExit: 0 }),
          status: status({ outcome: "done" }),
          spawnLog: "loop-runner: claimed done but verify failed (see verify.log)\n",
        }),
      ),
    ).toBe("verify-fail");
  });

  it("stall ← outcome done but headBefore === headAfter (committed nothing)", () => {
    expect(
      classifyAttempt(
        attempt({ meta: meta({ engineExit: 0, headBefore: "abc", headAfter: "abc" }), status: status({ outcome: "done" }) }),
      ),
    ).toBe("stall");
  });

  it("is NOT a stall when heads differ, are absent, or are 'unknown' (git-read failure)", () => {
    expect(
      classifyAttempt(attempt({ meta: meta({ engineExit: 0, headBefore: "abc", headAfter: "def" }), status: status({ outcome: "done" }) })),
    ).toBeNull();
    expect(
      classifyAttempt(attempt({ meta: meta({ engineExit: 0, headBefore: "unknown", headAfter: "unknown" }), status: status({ outcome: "done" }) })),
    ).toBeNull();
    expect(classifyAttempt(attempt({ meta: meta({ engineExit: 0 }), status: status({ outcome: "done" }) }))).toBeNull();
  });
});

describe("attemptEnded", () => {
  it("is true exactly when meta.json exists", () => {
    expect(attemptEnded(attempt({ meta: null }))).toBe(false);
    expect(attemptEnded(attempt({ meta: meta({ engineExit: 0 }) }))).toBe(true);
  });
});

describe("deriveLiveness — transcript.jsonl mtime is the heartbeat", () => {
  const now = 1_000_000_000_000;
  it("live when the transcript was touched within the stale window", () => {
    expect(deriveLiveness(now - 1000, now)).toBe("live");
  });
  it("flatline when the transcript is older than 25min (protocol threshold)", () => {
    expect(deriveLiveness(now - DEFAULT_STALE_MS - 1, now)).toBe("flatline");
  });
  it("flatline when there is no transcript mtime", () => {
    expect(deriveLiveness(null, now)).toBe("flatline");
  });
  it("honours a custom stale window", () => {
    expect(deriveLiveness(now - 5000, now, 1000)).toBe("flatline");
    expect(deriveLiveness(now - 500, now, 1000)).toBe("live");
  });
});

describe("normalizeEvent — open-string event names", () => {
  it("maps known-ish names to a glyph by keyword", () => {
    expect(normalizeEvent({ event: "merged", phase: "1" }).glyph).not.toBe("•");
    expect(normalizeEvent({ event: "runner-spawned" }).known).toBe(true);
    expect(normalizeEvent({ event: "hil-pause" }).label).toMatch(/HIL/i);
    expect(normalizeEvent({ event: "pr-created" }).label).toMatch(/PR/i);
  });
  it("falls back generically for an unknown event, humanizing the name", () => {
    const e = normalizeEvent({ event: "quantum_flux", phase: "2", detail: "x", ts: "T" });
    expect(e.known).toBe(false);
    expect(e.glyph).toBe("•");
    expect(e.label).toBe("Quantum flux");
    expect(e.event).toBe("quantum_flux");
    expect(e.ts).toBe("T");
  });
  it("tolerates a totally empty row", () => {
    const e = normalizeEvent({});
    expect(e.event).toBe("");
    expect(e.phase).toBe("");
    expect(e.ts).toBeNull();
  });
  it("coerces a non-string event/phase without throwing (would otherwise freeze the snapshot)", () => {
    const e = normalizeEvent({ event: 5, phase: 2, ts: 9 } as unknown as Parameters<typeof normalizeEvent>[0]);
    expect(e.event).toBe("");
    expect(e.phase).toBe("");
    expect(e.ts).toBeNull();
    expect(e.known).toBe(false);
  });
  it("carries a warning tone for hazard events", () => {
    expect(normalizeEvent({ event: "hil-pause" }).tone).toBe("hil");
    expect(normalizeEvent({ event: "verify-failed" }).tone).toBe("verify");
    expect(normalizeEvent({ event: "merged" }).tone).toBeNull();
  });
});

describe("lastWorkedBy — model is the last leg only", () => {
  it("takes engine/model from the highest-K ended attempt", () => {
    const a = [
      attempt({ k: 1, meta: meta({ engine: "codex", model: "gpt-5.6-sol" }) }),
      attempt({ k: 2, meta: meta({ engine: "claude", model: "opus" }) }),
      attempt({ k: 3, meta: null }), // running, no meta yet
    ];
    expect(lastWorkedBy(a)).toEqual({ engine: "claude", model: "opus" });
  });
  it("returns nulls when nothing has ended", () => {
    expect(lastWorkedBy([attempt({ k: 1, meta: null })])).toEqual({
      engine: null,
      model: null,
    });
  });
});

describe("summarizeAttempt", () => {
  it("captures per-attempt engine/model/outcome/problem", () => {
    const s = summarizeAttempt(
      attempt({ k: 2, meta: meta({ engine: "claude", model: "opus", engineExit: 124, timedOut: true }) }),
    );
    expect(s).toMatchObject({ k: 2, engine: "claude", model: "opus", problem: "timeout", ended: true });
  });
});

describe("problemSeverity — HIL is loudest by construction", () => {
  it("ranks hil above every other class", () => {
    const others = (["question", "blocked", "verify-fail", "crash", "timeout", "chain-exhausted"] as const).map(
      problemSeverity,
    );
    expect(others.every((s) => problemSeverity("hil") > s)).toBe(true);
  });
});
