import { createHash } from "node:crypto";
import { isAbsolute, relative, resolve, sep } from "node:path";
import type { Ask, CockpitEvent, RunMeta } from "../../core/events.ts";
import type { JsonlLine } from "./tailer.ts";

export interface SessionContext {
  path: string;
  userHome: string;
  modifiedAt: number;
  now: number;
}

export interface SessionSnapshot {
  runId: string;
  meta: RunMeta;
  pendingAsks: Record<string, Ask>;
}

export interface SessionResult {
  events: CockpitEvent[];
  state: SessionSnapshot;
}

export type JsonObject = Record<string, unknown>;
export const IDLE_MS = 2 * 60 * 1000;

export class SessionBuilder {
  readonly events: CockpitEvent[] = [];
  readonly state: SessionSnapshot;
  private offset = -1;
  private timestamp: string;

  constructor(source: "claude" | "codex", private readonly context: SessionContext, previous?: SessionSnapshot) {
    this.state = previous ? structuredClone(previous) : {
      runId: `${source}-${digest(resolve(context.path))}`,
      meta: { source, title: source === "claude" ? "Claude Code session" : "Codex session", isSidechain: false },
      pendingAsks: {},
    };
    this.timestamp = new Date(context.modifiedAt).toISOString();
  }

  line(line: JsonlLine, record: JsonObject): void {
    this.offset = line.offset;
    this.timestamp = validTime(record.timestamp) ?? new Date(this.context.modifiedAt).toISOString();
  }

  metadata(patch: RunMeta): void {
    const clean = Object.fromEntries(Object.entries(patch).filter(([, value]) => value !== undefined));
    if (typeof clean.cwd === "string") {
      const chatId = belugaChatId(clean.cwd, this.context.userHome);
      if (chatId) clean.chatId = chatId;
      else if (this.state.meta.chatId) clean.chatId = "";
    }
    const changed = Object.entries(clean).some(([key, value]) => this.state.meta[key as keyof RunMeta] !== value);
    this.state.meta = { ...this.state.meta, ...clean };
    if (changed) this.events.push({ ...this.base(`meta:${digest(JSON.stringify(this.state.meta))}`), type: "run.meta", meta: { ...this.state.meta } });
  }

  tool(id: string, name: string, input: unknown): void {
    const action = { id, tool: name, summary: inputSummary(input), input, timestamp: this.timestamp, sessionId: this.state.meta.sessionId };
    this.events.push({ ...this.base(`action:${id}`), type: "action", action });
    if (/(?:^|\.)(?:AskUserQuestion|request_user_input)$/.test(name)) {
      const object = asObject(input);
      const questions = Array.isArray(object.questions) ? object.questions : [];
      const text = questions.map((question) => string(asObject(question).question)).filter(Boolean).join("\n") || action.summary;
      this.ask({ id, text, resolved: false, sessionId: this.state.meta.sessionId });
    }
  }

  answer(id: string): void {
    const ask = this.state.pendingAsks[id];
    if (ask) this.ask({ ...ask, resolved: true });
  }

  userTurn(text?: string): void {
    if (text && (this.state.meta.title === "Codex session" || this.state.meta.title === "Claude Code session")) {
      this.metadata({ title: text.trim().split("\n")[0].slice(0, 120) });
    }
    for (const id of Object.keys(this.state.pendingAsks)) {
      if (id.startsWith("question:")) this.answer(id);
    }
  }

  finalMessage(text: string): void {
    if (!text.trim().endsWith("?")) return;
    this.ask({ id: `question:${this.offset}`, text: text.trim().slice(-1000), resolved: false, sessionId: this.state.meta.sessionId });
  }

  finish(): SessionResult {
    this.timestamp = new Date(this.context.now).toISOString();
    const status = this.context.now - this.context.modifiedAt >= IDLE_MS ? "idle" : "active";
    this.metadata({ status });
    return { events: this.events, state: this.state };
  }

  private ask(ask: Ask): void {
    if (ask.resolved) delete this.state.pendingAsks[ask.id];
    else this.state.pendingAsks[ask.id] = ask;
    this.events.push({ ...this.base(`ask:${ask.id}:${ask.resolved}`), type: "ask", ask });
  }

  private base(suffix: string) {
    return {
      id: `${this.state.runId}:${this.offset}:${suffix}:${digest(this.timestamp)}`, runId: this.state.runId,
      timestamp: this.timestamp, sessionId: this.state.meta.sessionId,
    };
  }
}

export function belugaChatId(cwd: string, userHome: string): string | undefined {
  if (!isAbsolute(cwd)) return undefined;
  const local = relative(resolve(userHome, ".beluga/chats"), resolve(cwd));
  if (!local || local === ".." || local.startsWith(`..${sep}`) || isAbsolute(local)) return undefined;
  return local.split(sep)[0];
}

export function parseLine(line: JsonlLine): JsonObject | undefined {
  try {
    const value: unknown = JSON.parse(line.text);
    if (value && typeof value === "object" && !Array.isArray(value)) return value as JsonObject;
  } catch { /* Ignore a malformed source record. The tailer retains incomplete lines. */ }
  return undefined;
}

export function asObject(value: unknown): JsonObject {
  return value && typeof value === "object" && !Array.isArray(value) ? value as JsonObject : {};
}

export function string(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

export function parseInput(value: unknown): unknown {
  if (typeof value !== "string") return value;
  try { return JSON.parse(value); } catch { return value; }
}

export function inputSummary(input: unknown): string {
  const object = asObject(input);
  const selected = [object.description, object.command, object.cmd, object.file_path, object.path, object.query, object.prompt]
    .find((value) => typeof value === "string");
  const text = typeof input === "string" ? input : typeof selected === "string" ? selected : JSON.stringify(input) ?? "";
  return text.replace(/\s+/g, " ").trim().slice(0, 240);
}

export function digest(text: string): string {
  return createHash("sha256").update(text).digest("hex").slice(0, 24);
}

function validTime(value: unknown): string | undefined {
  return typeof value === "string" && Number.isFinite(Date.parse(value)) ? value : undefined;
}
