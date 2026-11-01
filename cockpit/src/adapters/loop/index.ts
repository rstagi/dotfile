import { createHash } from "node:crypto";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import type { CockpitEvent, EventBase, Lane } from "../../core/events.ts";
import { readCheckpoint } from "./checkpoint.ts";
import type { LoopPhase } from "./checkpoint.ts";

export interface LoopPhaseState {
  slug?: string;
  lane?: string;
  worktree?: string | null;
  sessionId?: string | null;
  startedAt?: string;
  mergedAt?: string;
  status?: string;
}

export interface LoopState {
  runId?: string;
  effort?: string;
  phases?: Record<string, LoopPhaseState>;
}

export interface LoopOptions {
  now?: string;
  isPidAlive?: (pid: number) => boolean;
}

type RecordValue = Record<string, unknown>;
type Payload = CockpitEvent extends infer E ? E extends CockpitEvent ? Omit<E, keyof EventBase> : never : never;

const START = new Set(["spawned", "phase.start", "phase.attempt.start"]);
const END = new Set(["merged", "phase.merged", "phase.end"]);
const STARTED = new Set(["running", "in-progress", "spawned", "blocked", "paused", "done", "merged", "failed", "review-ready"]);
const MIME: Record<string, string> = {
  ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".gif": "image/gif",
  ".webp": "image/webp", ".webm": "video/webm", ".mp4": "video/mp4", ".mov": "video/quicktime",
};

/** Read one Loop source. The returned events have stable, content-derived IDs. */
export async function readLoop(directory: string, options: LoopOptions = {}): Promise<CockpitEvent[]> {
  const dir = resolve(directory);
  const markdown = readText(join(dir, "plan.md"));
  if (markdown === null) return [];
  const state = readJson(join(dir, "state.json")) as LoopState | null;
  const now = options.now ?? new Date().toISOString();
  const cp = await readCheckpoint(dir, now);
  const runId = state?.runId ?? markdown.match(/^planId:\s*(\S+)/m)?.[1]
    ?? `loop-${hash(dir)}`;
  const fallback = fileTime(join(dir, "state.json")) ?? fileTime(join(dir, "plan.md")) ?? now;
  const events: CockpitEvent[] = [];
  const emit = (payload: Payload, timestamp = fallback, phase?: LoopPhase) => {
    const event = {
      ...payload, runId, timestamp: iso(timestamp, fallback),
      ...(phase ? { phaseId: phase.phase, laneId: phase.lane } : {}),
    };
    events.push({ ...event, id: `loop-${hash(JSON.stringify(event))}` } as CockpitEvent);
  };

  const lanes: Lane[] = [];
  for (const phase of cp.phases) {
    let lane = lanes.find((lane) => lane.id === phase.lane);
    if (!lane) {
      lane = { id: phase.lane, title: `Lane ${phase.lane}`, phaseIds: [] };
      lanes.push(lane);
    }
    lane.phaseIds!.push(phase.phase);
  }
  emit({ type: "run.meta", meta: {
    title: state?.effort ?? cp.name, source: "loop", cwd: resolve(dir, ".."),
    status: runIsAlive(dir, options.isPidAlive ?? pidIsAlive) ? "active" : "done",
    lanes, plan: { id: runId, title: cp.name, content: markdown },
    milestones: cp.milestones.map((milestone) => ({
      id: String(milestone.number), title: milestone.title,
      phaseIds: cp.phases.filter((phase) => phase.milestone === milestone.number).map((phase) => phase.phase),
    })),
  } });

  const journal = [
    ...readLines(join(dir, "events.jsonl")),
    ...readLines(join(dir, "emitted.jsonl")).map((entry) => ({ ...object(entry.body), ts: object(entry.body)?.ts ?? entry.ts })),
  ].sort((a, b) => String(a.ts).localeCompare(String(b.ts)));
  for (const phase of cp.phases) {
    const mine = journal.filter((entry) => String(entry.phase) === phase.phase);
    const starts = mine.filter((entry) => START.has(String(entry.event)));
    const ends = mine.filter((entry) => END.has(String(entry.event)));
    const runtime = state?.phases?.[phase.phase];
    const identity = { id: phase.phase, title: phase.title, laneId: phase.lane };
    for (const entry of starts) emit({ type: "phase.start", phase: identity }, String(entry.ts ?? fallback), phase);
    if (!starts.length && STARTED.has(phase.rawStatus)) {
      emit({ type: "phase.start", phase: identity }, runtime?.startedAt ?? phase.start ?? fallback, phase);
    }
    for (const entry of ends) emit({ type: "phase.end", phase: { ...identity, status: "done" } }, String(entry.ts ?? fallback), phase);
    if (!ends.length && ["done", "merged", "failed"].includes(phase.rawStatus)) {
      emit({ type: "phase.end", phase: { ...identity, status: phase.rawStatus === "failed" ? "failed" : "done" } },
        runtime?.mergedAt ?? phase.end ?? fallback, phase);
    }
    if (phase.drift.length || phase.estimate || phase.diff || phase.verify) {
      emit({ type: "drift", drift: {
        id: `drift-${phase.phase}`, phaseId: phase.phase,
        summary: phase.drift.map((item) => `${item.kind}: ${item.detail}`).join("\n") || "No drift from the plan.",
        ...(phase.estimate ? { estimateMinutes: phase.estimate.max } : {}),
        ...(phase.minutes !== null ? { elapsedMinutes: phase.minutes } : {}),
        ...(phase.diff ? { diff: `+${phase.diff.insertions} -${phase.diff.deletions} in ${phase.diff.files} files` } : {}),
        ...(phase.verify ? { verification: `${phase.verify.passed ? "passed" : "not recorded"}${phase.verify.summary ? `: ${phase.verify.summary}` : ""}` } : {}),
      } }, now, phase);
    }
  }
  for (const marker of readLines(join(dir, "checkpoints.jsonl"))) {
    if (!Number.isInteger(marker.n) || Number(marker.n) < 1) continue;
    const phase = cp.phases.find((phase) => phase.phase === String(marker.phase));
    emit({ type: "checkpoint", checkpoint: {
      id: `checkpoint-${marker.n}`, summary: typeof marker.summary === "string" ? marker.summary : `${marker.done}/${marker.total} phases done`,
      ...(phase ? { phaseId: phase.phase } : {}),
    } }, String(marker.ts ?? fallback), phase);
  }
  for (const attempt of directoryNames(join(dir, "runs"))) {
    const slug = attempt.match(/^(.*)-a\d+$/)?.[1];
    const number = Object.entries(state?.phases ?? {}).find(([, phase]) => phase.slug === slug)?.[0];
    const phase = cp.phases.find((phase) => phase.phase === number);
    const proof = join(dir, "runs", attempt, "proof");
    for (const name of fileNames(proof)) {
      const mimeType = MIME[extname(name).toLowerCase()];
      if (!mimeType) continue;
      const path = join(proof, name);
      emit({ type: "media", media: {
        id: `proof-${hash(path)}`, path, mimeType, title: name,
        ...(phase ? { phaseId: phase.phase } : {}),
      } }, fileTime(path) ?? fallback, phase);
    }
  }
  for (const name of fileNames(join(dir, "hil"))) {
    if (name.endsWith(".answer.md")) continue;
    const text = readText(join(dir, "hil", name));
    if (text === null) continue;
    const slug = name.replace(/\.[^.]+$/, "");
    const number = Object.entries(state?.phases ?? {}).find(([, phase]) => phase.slug === slug)?.[0];
    const phase = cp.phases.find((phase) => phase.phase === number);
    emit({ type: "ask", ask: {
      id: `hil-${name}`, text, phaseId: number,
      resolved: readText(join(dir, "hil", `${slug}.answer.md`)) !== null
        || readText(join(dir, "answers", name)) !== null,
    } }, fileTime(join(dir, "hil", name)) ?? fallback, phase);
  }
  return [...new Map(events.map((event) => [event.id, event])).values()]
    .sort((a, b) => a.timestamp.localeCompare(b.timestamp));
}

function readText(file: string): string | null {
  try { return readFileSync(file, "utf8"); } catch { return null; }
}

function readJson(file: string): RecordValue | null {
  try { return object(JSON.parse(readText(file) ?? "null")); } catch { return null; }
}

function object(value: unknown): RecordValue | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as RecordValue : null;
}

function fileTime(file: string): string | null {
  try { return statSync(file).mtime.toISOString(); } catch { return null; }
}

function hash(value: string): string {
  return createHash("sha256").update(value).digest("hex").slice(0, 24);
}

function iso(value: string, fallback: string): string {
  return Number.isFinite(Date.parse(value)) ? new Date(value).toISOString() : fallback;
}

function runIsAlive(dir: string, alive: (pid: number) => boolean): boolean {
  const lock = join(dir, "lock");
  const pidText = readText(join(lock, "pid")) ?? readText(lock) ?? readText(join(lock, "owner"));
  if (pidText === null) return false;
  const pid = Number(pidText.trim());
  // New Loop locks record the run ID because sessions have no stable PID.
  if (!Number.isInteger(pid)) return true;
  return pid > 0 && alive(pid);
}

function pidIsAlive(pid: number): boolean {
  try { process.kill(pid, 0); return true; } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

function readLines(file: string): RecordValue[] {
  const records: RecordValue[] = [];
  for (const line of (readText(file) ?? "").split(/\r?\n/)) {
    try {
      const record = object(JSON.parse(line));
      if (record) records.push(record);
    } catch { /* Ignore incomplete journal records. */ }
  }
  return records;
}

function fileNames(dir: string): string[] {
  try { return readdirSync(dir, { withFileTypes: true }).filter((entry) => entry.isFile()).map((entry) => entry.name).sort(); }
  catch { return []; }
}

function directoryNames(dir: string): string[] {
  try { return readdirSync(dir, { withFileTypes: true }).filter((entry) => entry.isDirectory()).map((entry) => entry.name).sort(); }
  catch { return []; }
}
