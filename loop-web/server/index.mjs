// Loop Observatory server.
//
// Two modes, one machinery:
//   • daemon  (LOOP_DAEMON_MODE=1) — the perpetual central observer on 127.0.0.1:7717.
//     Loops POST lifecycle to it (`/api/loops/:runId/{register,state,event,finish}`); each is
//     kept forever in `~/.loop/loops/<runId>.json`. A selector lists them; per-loop SSE
//     streams the graph.
//   • legacy  (loop-web --dir X) — discovers one flattened `.loop/`, synthesizes a single
//     registration into the same registry. No store; ephemeral.
//
// Authoritative status is materialized through the tested pure model (../src/model): script
// events promote a phase's lifecycle even when the orchestrator's state.json bookkeeping
// lags — the fix for stuck running/todo nodes. The server does ALL I/O; the model stays pure.

import http from "node:http";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { reduceLoop, emptyRecord, materialize, summarize } from "../src/model/index.ts";
import { createStore } from "./store.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DIST_DIR = path.join(__dirname, "..", "dist");

const WATCH_DEBOUNCE_MS = 2000;
const RECONCILE_MS = 15000;
const TAIL_BYTES = 64 * 1024;
const NOTE_BYTES = 16 * 1024;
const CONTROL_ACTIONS = new Set(["pause", "resume", "model"]);
/** `engine:model[+fallback[,fallback2]]` — the loop-models.conf leg grammar. */
const MODEL_LEG = /^(codex|claude):@?[A-Za-z0-9._-]+(\+[A-Za-z0-9._-]+(,[A-Za-z0-9._-]+)*)?$/;
const DECISION_FILE_BYTES = 128 * 1024;
const BODY_LIMIT = 4 * 1024 * 1024; // cap an ingest body (plan md + state) — reject beyond
const HOST = "127.0.0.1";

// --- configuration -------------------------------------------------------------------

function resolveConfig() {
  const args = parseArgs(process.argv);
  const env = process.env;
  const port = Number(args.port ?? env.LOOP_WEB_PORT ?? 7717);
  const daemon = args.daemon || env.LOOP_DAEMON_MODE === "1";
  const storeDir = env.LOOP_STORE_DIR
    ? path.resolve(env.LOOP_STORE_DIR)
    : path.join(os.homedir(), ".loop", "loops");

  let loopDir = args.dir ?? env.LOOP_WEB_DIR ?? null;
  let planFile = args.plan ?? env.LOOP_WEB_PLAN ?? null;
  if (!daemon && !loopDir && !planFile) {
    const found = discoverLoop(process.cwd());
    if (found) {
      loopDir = found; // flattened: the `.loop/` dir IS the loop dir; plan.md sits inside it
      planFile = path.join(found, "plan.md");
    }
  }
  if (loopDir && !planFile) planFile = path.join(loopDir, "plan.md");
  return {
    port,
    daemon,
    storeDir,
    loopDir: loopDir ? path.resolve(loopDir) : null,
    planFile: planFile ? path.resolve(planFile) : null,
  };
}

function parseArgs(argv) {
  const out = {};
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dir") out.dir = argv[++i];
    else if (a === "--plan") out.plan = argv[++i];
    else if (a === "--port") out.port = argv[++i];
    else if (a === "--daemon") out.daemon = true;
  }
  return out;
}

/** Walk up from `start` for a `.loop/` directory; return its path or null (legacy mode). */
function discoverLoop(start) {
  let dir = path.resolve(start);
  for (;;) {
    const candidate = path.join(dir, ".loop");
    if (isDir(candidate)) return candidate;
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

// --- registry ------------------------------------------------------------------------

/** runId → { record, subscribers:Set<res>, lastSnapshotJson, stopWatch, live, seq } */
const loops = new Map();
/** clients that subscribed to a runId before it existed (drained on register). */
const pendingSubscribers = new Map();
/** bare /events (no runId) — a back-compat stream that always mirrors the default loop. */
const legacySubscribers = new Set();
let seqCounter = 0;
let STORE = null;

function getEntry(runId) {
  let entry = loops.get(runId);
  if (!entry) {
    entry = { record: emptyRecord(runId), subscribers: new Set(), lastSnapshotJson: "null", stopWatch: null, live: false, seq: 0 };
    loops.set(runId, entry);
    const waiting = pendingSubscribers.get(runId);
    if (waiting) {
      for (const res of waiting) entry.subscribers.add(res);
      pendingSubscribers.delete(runId);
    }
  }
  return entry;
}

/** Fold one ingest into a loop, re-materialize, broadcast, and persist. */
function ingest(runId, msg, { flush = false } = {}) {
  const entry = getEntry(runId);
  entry.record = reduceLoop(entry.record, msg);
  entry.seq = ++seqCounter;
  maybeStartWatcher(entry);
  rematerialize(entry);
  broadcast(entry);
  if (STORE) flush ? STORE.flush(entry.record) : STORE.save(entry.record);
}

function rematerialize(entry) {
  const live = readLive(entry.record);
  const now = Date.now();
  const snap = materialize(entry.record, live, { now, nowIso: new Date(now).toISOString() });
  entry.record.lastSnapshot = snap;
  entry.lastSnapshotJson = JSON.stringify(snap);
  entry.live = live != null;
}

/** LoopInput read fresh from the loop dir, or null when the worktree is gone (archived). */
function readLive(record) {
  return record.loopDir && isDir(record.loopDir) ? readLoopInput(record.loopDir) : null;
}

// --- watching (per live loop) --------------------------------------------------------

function maybeStartWatcher(entry) {
  if (entry.stopWatch || !entry.record.loopDir || !isDir(entry.record.loopDir)) return;
  const dir = entry.record.loopDir;
  const planFile = entry.record.planFile;
  let debounce = null;
  const schedule = () => {
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(() => reconcile(entry), WATCH_DEBOUNCE_MS);
  };
  const watchers = [];
  const watch = (p, opts) => {
    try {
      watchers.push(fs.watch(p, opts, schedule));
    } catch {
      /* path may not exist yet — the reconcile timer still covers it */
    }
  };
  watch(dir, { recursive: true });
  if (planFile) watch(planFile, {});
  // A hung runner emits no fs event, but its stale heartbeat must still flip live→flatline.
  const timer = setInterval(() => reconcile(entry), RECONCILE_MS);
  timer.unref?.();
  entry.stopWatch = () => {
    for (const w of watchers) {
      try {
        w.close();
      } catch {
        /* ignore */
      }
    }
    clearInterval(timer);
    if (debounce) clearTimeout(debounce);
    entry.stopWatch = null;
  };
}

/** Self-heal a live loop from disk (missed POSTs) + refresh the heartbeat, then broadcast. */
function reconcile(entry) {
  const dir = entry.record.loopDir;
  if (!dir || !isDir(dir)) {
    // worktree removed → archived: stop watching, freeze on the last snapshot.
    if (entry.stopWatch) entry.stopWatch();
    rematerialize(entry);
    broadcast(entry);
    return;
  }
  const stateText = readText(path.join(dir, "state.json"));
  const eventsText = readText(path.join(dir, "events.jsonl"));
  const planText = entry.record.planFile ? readText(entry.record.planFile) : null;
  const state = safeParse(stateText);
  if (state) entry.record = reduceLoop(entry.record, { kind: "state", state, planText });
  if (eventsText) entry.record = reduceLoop(entry.record, { kind: "eventsFile", text: eventsText });
  for (const event of readDecisionEvents(dir, entry.record.runId, entry.record.decisions)) {
    entry.record = reduceLoop(entry.record, { kind: "event", event });
  }
  rematerialize(entry);
  broadcast(entry);
  if (STORE) STORE.save(entry.record);
}

function readDecisionEvents(loopDir, runId, knownDecisions) {
  const runsDir = path.join(loopDir, "runs");
  const events = [];
  const known = new Set(knownDecisions.map(decisionIdentity));
  for (const runName of safeReaddir(runsDir)) {
    const attempt = Number(/-a([1-9][0-9]*)$/.exec(runName)?.[1]);
    if (!Number.isSafeInteger(attempt)) continue;
    const runDir = path.join(runsDir, runName);
    if (!isDir(runDir)) continue;
    for (const fileName of safeReaddir(runDir)) {
      const stage = decisionStage(fileName);
      if (!stage) continue;
      const filePath = path.join(runDir, fileName);
      try {
        const stat = fs.lstatSync(filePath);
        if (!stat.isFile() || stat.size > DECISION_FILE_BYTES) continue;
      } catch {
        continue;
      }
      const decision = safeParse(readText(filePath));
      if (!decision || Array.isArray(decision) || decision.version !== 1 || decision.stage !== stage
        || decision.attempt !== attempt || typeof decision.phase !== "string" || !decision.phase.trim()) continue;
      if (stage === "merge-risk" && decision.head !== fileName.slice("risk-decision-".length, -".json".length)) continue;
      const event = { ...decision, event: "jev.decision", runId };
      const identity = decisionIdentity(event);
      if (known.has(identity)) continue;
      known.add(identity);
      events.push(event);
    }
  }
  return events.sort((left, right) => decisionTime(left.ts) - decisionTime(right.ts)
    || decisionIdentity(left).localeCompare(decisionIdentity(right)));
}

function decisionIdentity(decision) {
  return JSON.stringify([decision.runId, decision.phase, decision.attempt, decision.stage,
    decision.ts ?? "", decision.head ?? ""]);
}

function decisionTime(ts) {
  const parsed = typeof ts === "string" ? Date.parse(ts) : NaN;
  return Number.isFinite(parsed) ? parsed : Number.NEGATIVE_INFINITY;
}

function decisionStage(fileName) {
  if (fileName === "route-decision.json") return "route";
  if (fileName === "question-decision.json") return "question";
  if (/^risk-decision-[0-9a-f]{7,64}\.json$/.test(fileName)) return "merge-risk";
  return null;
}

// --- SSE broadcast -------------------------------------------------------------------

function snapshotFrame(json) {
  return `event: snapshot\ndata: ${json}\n\n`;
}

function broadcast(entry) {
  const frame = snapshotFrame(entry.lastSnapshotJson);
  for (const res of entry.subscribers) res.write(frame);
  pushLegacy();
}

/** Re-point the bare-/events stream at whatever loop is currently the default. */
function pushLegacy() {
  if (legacySubscribers.size === 0) return;
  const d = defaultEntry();
  const frame = snapshotFrame(d ? d.lastSnapshotJson : "null");
  for (const res of legacySubscribers) res.write(frame);
}

/** The loop the back-compat endpoints resolve to: the most-recent active loop, else newest. */
function defaultEntry() {
  let best = null;
  for (const e of loops.values()) {
    if (!best) {
      best = e;
      continue;
    }
    const ea = isActiveish(e);
    const ba = isActiveish(best);
    if (ea !== ba) {
      if (ea) best = e;
      continue;
    }
    if (e.seq > best.seq) best = e;
  }
  return best;
}

function isActiveish(e) {
  return isDir(e.record.loopDir) && e.record.status !== "finished";
}

// --- HTTP ----------------------------------------------------------------------------

function handle(req, res) {
  const u = new URL(req.url, "http://localhost");
  const p = u.pathname;

  if (req.method === "POST") {
    const note = p.match(/^\/api\/loops\/([^/]+)\/note$/);
    if (note) return handleNote(decodeURIComponent(note[1]), req, res);
    const control = p.match(/^\/api\/loops\/([^/]+)\/control$/);
    if (control) return handleControl(decodeURIComponent(control[1]), req, res);
    const archive = p.match(/^\/api\/loops\/([^/]+)\/(archive|unarchive)$/);
    if (archive) return handleArchive(decodeURIComponent(archive[1]), archive[2] === "archive", res);
    const m = p.match(/^\/api\/loops\/([^/]+)\/(register|state|event|finish)$/);
    if (m) return handleIngest(decodeURIComponent(m[1]), m[2], req, res);
    return json(res, 404, { error: "unknown endpoint" });
  }

  if (p === "/events") return handleSse(req, res, u.searchParams.get("runId"));
  if (p === "/api/health") return json(res, 200, { ok: true, mode: MODE, loops: loops.size });
  if (p === "/api/loops") return json(res, 200, listLoops());

  let m;
  if ((m = p.match(/^\/api\/loops\/([^/]+)\/plan$/))) return handleLoopPlan(decodeURIComponent(m[1]), res);
  if ((m = p.match(/^\/api\/loops\/([^/]+)\/snapshot$/))) return handleLoopSnapshot(decodeURIComponent(m[1]), res);
  if ((m = p.match(/^\/api\/loops\/([^/]+)\/review$/))) {
    return handleReview(decodeURIComponent(m[1]), u.searchParams.get("repository"), res);
  }
  if ((m = p.match(/^\/api\/loops\/([^/]+)\/attempt\/(.+)\/(\d+)\/?$/)))
    return handleAttempt(decodeURIComponent(m[1]), decodeURIComponent(m[2]), m[3], res);

  // back-compat (pre-selector UI): resolve to the default loop.
  if (p === "/api/model" || p === "/api/snapshot") {
    const d = defaultEntry();
    return sendJsonText(res, d ? d.lastSnapshotJson : "null");
  }
  if ((m = p.match(/^\/api\/attempt\/(.+)\/(\d+)\/?$/))) {
    const d = defaultEntry();
    if (!d) return json(res, 404, { error: "no loop" });
    return handleAttempt(d.record.runId, decodeURIComponent(m[1]), m[2], res);
  }

  if (p.startsWith("/api/")) return json(res, 404, { error: "unknown endpoint" });
  return serveStatic(req, res);
}

/** Manual archive flag (`loop-top archive`): hides the loop from selectors without touching
 * its worktree. Persisted on the store record; `unarchive` clears it. */
function handleArchive(runId, archived, res) {
  const entry = loops.get(runId);
  if (!entry) return json(res, 404, { error: "no such loop", runId });
  entry.record = { ...entry.record, archived };
  broadcast(entry);
  if (STORE) STORE.flush(entry.record);
  json(res, 200, { ok: true, runId, archived });
}

function handleNote(runId, req, res) {
  const entry = loops.get(runId);
  if (!entry) return json(res, 404, { error: "no such loop", runId });
  const loopDir = entry.record.loopDir;
  if (!loopDir || !isDir(loopDir)) {
    return json(res, 410, { error: "worktree gone — archived loops are not steerable", runId });
  }
  readBody(req, BODY_LIMIT, (err, body) => {
    if (err) return json(res, 413, { error: "body too large" });
    const parsed = safeParse(body);
    const key = parsed?.key;
    if (typeof key !== "string" || !isSafeNoteKey(key)) {
      return json(res, 400, { error: "invalid note key" });
    }
    const notesDir = path.join(loopDir, "notes");
    const noteFile = path.resolve(notesDir, `${key}.md`);
    if (!isContained(noteFile, path.resolve(notesDir))) return json(res, 400, { error: "bad path" });

    try {
      if (parsed.clear === true) {
        fs.rmSync(noteFile, { force: true });
      } else {
        if (typeof parsed.markdown !== "string") return json(res, 400, { error: "markdown must be a string" });
        if (Buffer.byteLength(parsed.markdown, "utf8") > NOTE_BYTES) {
          return json(res, 413, { error: `note exceeds ${NOTE_BYTES} bytes` });
        }
        fs.mkdirSync(notesDir, { recursive: true });
        fs.writeFileSync(noteFile, parsed.markdown, "utf8");
      }
      reconcile(entry);
      return json(res, 200, { ok: true, runId, key });
    } catch (e) {
      return json(res, 500, { error: String(e?.message ?? e) });
    }
  });
}

/**
 * User controls → `.loop/control/` files the runner, orchestrator and SUB poll:
 * `pause` (whole loop), `pause-<N>` (phase), `model-<N>` (leg for the next attempt of a
 * not-yet-started phase). Writes the file, folds a timeline event, and re-materializes.
 */
function handleControl(runId, req, res) {
  const entry = loops.get(runId);
  if (!entry) return json(res, 404, { error: "no such loop", runId });
  const loopDir = entry.record.loopDir;
  if (!loopDir || !isDir(loopDir)) {
    return json(res, 410, { error: "worktree gone — archived loops are not steerable", runId });
  }
  readBody(req, BODY_LIMIT, (err, body) => {
    if (err) return json(res, 413, { error: "body too large" });
    const parsed = safeParse(body) ?? {};
    const { action, phase = null } = parsed;
    if (!CONTROL_ACTIONS.has(action)) return json(res, 400, { error: "action must be pause|resume|model" });
    if (phase !== null && (typeof phase !== "string" || !isSafeNoteKey(phase))) {
      return json(res, 400, { error: "invalid phase" });
    }
    if (action === "model" && phase === null) return json(res, 400, { error: "model requires a phase" });
    const leg = parsed.leg ?? null;
    if (action === "model" && leg !== null && (typeof leg !== "string" || !MODEL_LEG.test(leg))) {
      return json(res, 400, { error: "leg must be codex|claude:<model>[+fallback[,fallback]]" });
    }
    if (action === "model" && phaseStarted(entry.record, loopDir, phase)) {
      return json(res, 409, { error: "phase already started — model can only change before its first attempt", phase });
    }

    const controlDir = path.join(loopDir, "control");
    const file = path.join(controlDir, action === "model" ? `model-${phase}` : phase ? `pause-${phase}` : "pause");
    try {
      if (action === "pause") writeControl(controlDir, file, new Date().toISOString());
      else if (action === "model" && leg !== null) writeControl(controlDir, file, leg);
      else fs.rmSync(file, { force: true });
    } catch (e) {
      return json(res, 500, { error: String(e?.message ?? e) });
    }
    const detail = action === "model" ? (leg ?? "default") : phase ? `phase ${phase}` : "loop";
    ingest(runId, { kind: "event", event: { event: `control.${action}`, phase: phase ?? "", detail, ts: new Date().toISOString() } });
    reconcile(entry);
    json(res, 200, { ok: true, runId, action, ...(phase ? { phase } : {}) });
  });
}

/** Started = state.json moved past todo, or any attempt was counted. */
function phaseStarted(record, loopDir, phase) {
  const live = safeParse(readText(path.join(loopDir, "state.json")));
  const ph = live?.phases?.[phase] ?? record.lastState?.phases?.[phase] ?? null;
  if (!ph) return false;
  return (ph.status != null && ph.status !== "todo") || (ph.attempt ?? 0) > 0;
}

function writeControl(dir, file, content) {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(file, content + "\n", "utf8");
}

function handleIngest(runId, kind, req, res) {
  readBody(req, BODY_LIMIT, (err, body) => {
    if (err) return json(res, 413, { error: "body too large" });
    const parsed = safeParse(body) ?? {};
    let msg;
    if (kind === "register") {
      const info = { ...parsed, runId };
      if (info.loopDir && !info.planFile) info.planFile = path.join(info.loopDir, "plan.md");
      if (!info.planText && info.planFile) info.planText = readText(info.planFile);
      msg = { kind: "register", info };
    } else if (kind === "state") {
      msg = { kind: "state", state: parsed };
    } else if (kind === "event") {
      msg = { kind: "event", event: parsed };
    } else {
      msg = { kind: "finish", info: parsed };
    }
    try {
      ingest(runId, msg, { flush: kind === "finish" });
    } catch (e) {
      return json(res, 500, { error: String(e?.message ?? e) });
    }
    json(res, 200, { ok: true, runId });
  });
}

function listLoops() {
  const out = [];
  for (const entry of loops.values()) {
    const sum = summarize(entry.record);
    // A worktree that's gone can't serve live logs — surface it as archived in the selector.
    // A `planned` loop keeps its status: its origin worktree may be gone, but the plan still
    // lives in the daemon (register-only records have no live logs to lose anyway).
    if (!isDir(entry.record.loopDir) && sum.status !== "planned") sum.status = "archived";
    if (entry.record.archived) sum.status = "archived"; // manual archive (loop-top archive)
    out.push({ ...sum, seq: entry.seq });
  }
  return out.sort((a, b) => b.seq - a.seq).map(({ seq, ...rest }) => rest);
}

/** The plan-fetch endpoint: a fresh worktree pulls `planText` straight from the in-memory
 * record (no worktree/loop-dir needed — this is how a clean checkout gets the plan). */
function handleLoopPlan(runId, res) {
  const entry = loops.get(runId);
  if (!entry) return json(res, 404, { error: "no such loop", runId });
  const rec = entry.record;
  json(res, 200, {
    runId: rec.runId,
    effort: rec.effort,
    status: rec.status,
    integrationBranch: rec.integrationBranch,
    planText: rec.planText,
  });
}

function handleLoopSnapshot(runId, res) {
  const entry = loops.get(runId);
  if (!entry) return json(res, 404, { error: "no such loop", runId });
  sendJsonText(res, entry.lastSnapshotJson);
}

function handleReview(runId, repository, res) {
  const entry = loops.get(runId);
  if (!entry) return json(res, 404, { error: "no such loop", runId });
  const rec = entry.record;
  const repositoryRecord = repository ? rec.repositories?.[repository] : null;
  if (repository && !repositoryRecord) return json(res, 404, { error: "no such repository", runId, repository });
  const review = repositoryRecord?.review ?? rec.review;
  let reportMarkdown = null;
  if (review?.reportPath && rec.loopDir) {
    const abs = path.isAbsolute(review.reportPath) ? path.resolve(review.reportPath) : path.resolve(rec.loopDir, review.reportPath);
    if (isContained(abs, path.resolve(rec.loopDir))) reportMarkdown = readText(abs);
  }
  json(res, 200, {
    outcome: review?.outcome ?? null,
    summary: review?.summary ?? null,
    reportMarkdown,
    commentUrl: review?.commentUrl ?? null,
    prUrl: repositoryRecord?.prUrl ?? rec.prUrl ?? null,
    repository: repository ?? null,
  });
}

function handleAttempt(runId, slug, k, res) {
  const entry = loops.get(runId);
  if (!entry) return json(res, 404, { error: "no such loop", runId });
  const loopDir = entry.record.loopDir;
  if (!loopDir || !isDir(loopDir)) return json(res, 410, { error: "worktree gone — logs unavailable", runId });
  const runsRoot = path.resolve(path.join(loopDir, "runs"));
  const runDir = path.resolve(path.join(loopDir, "runs", `${slug}-a${k}`));
  if (!isContained(runDir, runsRoot)) return json(res, 400, { error: "bad path" });
  if (!isDir(runDir)) return json(res, 404, { error: "no such attempt", slug, k });
  return json(res, 200, {
    slug,
    attempt: Number(k),
    runDir: path.join("runs", `${slug}-a${k}`),
    meta: safeParse(readText(path.join(runDir, "meta.json"))),
    status: safeParse(readText(path.join(runDir, "status.json"))),
    verifyLog: tail(path.join(runDir, "verify.log")),
    lastMessage: tail(path.join(runDir, "last.md")),
    stderr: tail(path.join(runDir, "stderr.log"), 16384),
    spawnLog: tail(path.join(runDir, "spawn.log"), 16384),
    transcriptPath: path.join(runDir, "transcript.jsonl"),
    transcriptMtime: mtimeMs(path.join(runDir, "transcript.jsonl")),
  });
}

function handleSse(req, res, runId) {
  res.writeHead(200, {
    "Content-Type": "text/event-stream",
    "Cache-Control": "no-cache, no-transform",
    Connection: "keep-alive",
    "X-Accel-Buffering": "no",
  });
  res.write("retry: 3000\n\n");
  if (runId) {
    const entry = loops.get(runId);
    if (entry) {
      res.write(snapshotFrame(entry.lastSnapshotJson));
      entry.subscribers.add(res);
      req.on("close", () => entry.subscribers.delete(res));
    } else {
      // subscribe optimistically — attach when the loop first registers.
      res.write(snapshotFrame("null"));
      const set = pendingSubscribers.get(runId) ?? new Set();
      set.add(res);
      pendingSubscribers.set(runId, set);
      req.on("close", () => pendingSubscribers.get(runId)?.delete(res));
    }
    return;
  }
  // bare /events — back-compat stream mirroring the default loop.
  const d = defaultEntry();
  res.write(snapshotFrame(d ? d.lastSnapshotJson : "null"));
  legacySubscribers.add(res);
  req.on("close", () => legacySubscribers.delete(res));
}

// --- filesystem readers (I/O lives here; the model layer stays pure) -----------------

function readLoopInput(loopDir) {
  if (!loopDir || !isDir(loopDir)) {
    return { present: false, state: null, events: null, runs: [], hil: [], notes: [] };
  }

  const runsDir = path.join(loopDir, "runs");
  const runs = [];
  if (isDir(runsDir)) {
    for (const name of safeReaddir(runsDir)) {
      const d = path.join(runsDir, name);
      if (!isDir(d)) continue;
      runs.push({
        name,
        meta: readText(path.join(d, "meta.json")),
        leg: readText(path.join(d, "leg.json")),
        status: readText(path.join(d, "status.json")),
        spawnLog: tail(path.join(d, "spawn.log"), 8192),
        transcriptMtime: mtimeMs(path.join(d, "transcript.jsonl")),
        metaMtime: mtimeMs(path.join(d, "meta.json")),
      });
    }
  }

  const hilDir = path.join(loopDir, "hil");
  const hil = [];
  if (isDir(hilDir)) {
    for (const f of safeReaddir(hilDir)) {
      if (!f.endsWith(".md") || f.endsWith(".answer.md")) continue;
      const slug = f.slice(0, -3);
      hil.push({
        slug,
        markdown: readText(path.join(hilDir, f)) ?? "",
        answered: fs.existsSync(path.join(hilDir, `${slug}.answer.md`)),
      });
    }
  }

  const notesDir = path.join(loopDir, "notes");
  const notes = [];
  if (isDir(notesDir)) {
    for (const f of safeReaddir(notesDir)) {
      if (!f.endsWith(".md")) continue;
      notes.push({ key: f.slice(0, -3), markdown: tail(path.join(notesDir, f), NOTE_BYTES) ?? "" });
    }
  }

  return {
    present: true,
    state: readText(path.join(loopDir, "state.json")),
    events: readText(path.join(loopDir, "events.jsonl")),
    runs,
    hil,
    notes,
    control: readControl(loopDir),
  };
}

/** `.loop/control/` → { paused, pausedPhases, models } (invalid model files are ignored). */
function readControl(loopDir) {
  const dir = path.join(loopDir, "control");
  const control = { paused: false, pausedPhases: [], models: {} };
  for (const f of safeReaddir(dir)) {
    if (f === "pause") control.paused = true;
    else if (f.startsWith("pause-") && isSafeNoteKey(f.slice(6))) control.pausedPhases.push(f.slice(6));
    else if (f.startsWith("model-") && isSafeNoteKey(f.slice(6))) {
      const leg = (readText(path.join(dir, f)) ?? "").trim();
      if (MODEL_LEG.test(leg)) control.models[f.slice(6)] = leg;
    }
  }
  return control;
}

function isSafeNoteKey(key) {
  return /^[A-Za-z0-9._-]+$/.test(key) && key !== "." && key !== "..";
}

function isDir(p) {
  try {
    return !!p && fs.statSync(p).isDirectory();
  } catch {
    return false;
  }
}
function readText(p) {
  try {
    return fs.readFileSync(p, "utf8");
  } catch {
    return null;
  }
}
function mtimeMs(p) {
  try {
    return fs.statSync(p).mtimeMs;
  } catch {
    return null;
  }
}
function tail(p, bytes = TAIL_BYTES) {
  try {
    const fd = fs.openSync(p, "r");
    try {
      const size = fs.fstatSync(fd).size;
      const start = Math.max(0, size - bytes);
      const buf = Buffer.alloc(size - start);
      fs.readSync(fd, buf, 0, buf.length, start);
      return buf.toString("utf8");
    } finally {
      fs.closeSync(fd);
    }
  } catch {
    return null;
  }
}
function safeReaddir(p) {
  try {
    return fs.readdirSync(p);
  } catch {
    return [];
  }
}

// --- static SPA ----------------------------------------------------------------------

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".ico": "image/x-icon",
};

function serveStatic(req, res) {
  const urlPath = req.url.split("?")[0];
  const rel = urlPath === "/" ? "index.html" : decodeURIComponent(urlPath.replace(/^\/+/, ""));
  const filePath = path.join(DIST_DIR, rel);
  if (!isContained(path.resolve(filePath), path.resolve(DIST_DIR))) {
    res.writeHead(403).end("forbidden");
    return;
  }
  fs.readFile(filePath, (err, buf) => {
    if (err) {
      fs.readFile(path.join(DIST_DIR, "index.html"), (e2, idx) => {
        if (e2) {
          res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
          res.end(BUILD_HINT);
        } else {
          res.writeHead(200, { "Content-Type": MIME[".html"] });
          res.end(idx);
        }
      });
      return;
    }
    res.writeHead(200, { "Content-Type": MIME[path.extname(filePath)] ?? "application/octet-stream" });
    res.end(buf);
  });
}

const BUILD_HINT = `<!doctype html><meta charset="utf-8"><title>Loop Observatory</title>
<body style="font-family:ui-monospace,monospace;background:#0a0e12;color:#cfe;padding:3rem">
<h1>Loop Observatory</h1><p>UI not built yet. Run <code>npm run build</code> in <code>loop-web/</code>,
or use <code>npm run dev</code> for the dev server. The data endpoints
(<code>/events</code>, <code>/api/loops</code>) are live.</p></body>`;

// --- small http helpers --------------------------------------------------------------

function readBody(req, limit, cb) {
  let size = 0;
  const chunks = [];
  let done = false;
  req.on("data", (c) => {
    if (done) return;
    size += c.length;
    if (size > limit) {
      done = true;
      cb(new Error("body too large"));
      req.destroy();
      return;
    }
    chunks.push(c);
  });
  req.on("end", () => {
    if (!done) cb(null, Buffer.concat(chunks).toString("utf8"));
  });
  req.on("error", (e) => {
    if (!done) {
      done = true;
      cb(e);
    }
  });
}

function json(res, code, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(code, { "Content-Type": "application/json; charset=utf-8" });
  res.end(body);
}
function sendJsonText(res, text) {
  res.writeHead(200, { "Content-Type": "application/json; charset=utf-8" });
  res.end(text);
}
function safeParse(s) {
  if (s == null) return null;
  try {
    return JSON.parse(s);
  } catch {
    return null;
  }
}
function isContained(abs, base) {
  return abs === base || abs.startsWith(base + path.sep);
}

// --- main ----------------------------------------------------------------------------

let MODE = "static";

function main() {
  const cfg = resolveConfig();
  MODE = cfg.daemon ? "daemon" : "legacy";

  if (cfg.daemon) {
    STORE = createStore(cfg.storeDir);
    for (const [runId, record] of STORE.loadAll()) {
      const entry = getEntry(runId);
      entry.record = record;
      entry.seq = ++seqCounter;
      maybeStartWatcher(entry); // reattach a still-live loop; archived loops just carry lastSnapshot
      if (isDir(entry.record.loopDir)) reconcile(entry);
      else rematerialize(entry);
    }
    const shutdown = () => {
      try {
        STORE.flushAll();
      } finally {
        process.exit(0);
      }
    };
    process.on("SIGTERM", shutdown);
    process.on("SIGINT", shutdown);
  } else if (cfg.loopDir || cfg.planFile) {
    // Legacy single-loop: synthesize one registration from the discovered dir, then fold the
    // on-disk state + events at once so `--dir` renders immediately (not only after the first
    // 15s reconcile — otherwise a static dir would sit at `planned`/0-phases until then).
    const stateText = cfg.loopDir ? readText(path.join(cfg.loopDir, "state.json")) : null;
    const runId = safeParse(stateText)?.runId || "local";
    ingest(runId, {
      kind: "register",
      info: { runId, loopDir: cfg.loopDir, planFile: cfg.planFile, planText: cfg.planFile ? readText(cfg.planFile) : null },
    });
    if (cfg.loopDir && isDir(cfg.loopDir)) reconcile(getEntry(runId));
  }

  const server = http.createServer((req, res) => {
    try {
      handle(req, res);
    } catch (err) {
      json(res, 500, { error: String(err?.message ?? err) });
    }
  });

  server.listen(cfg.port, HOST, () => {
    console.log(`[loop-web] Loop Observatory (${MODE}) on http://localhost:${cfg.port}`);
    if (cfg.daemon) console.log(`[loop-web] store: ${cfg.storeDir} (${loops.size} loop(s) loaded)`);
    else if (cfg.loopDir) console.log(`[loop-web] loop: ${cfg.loopDir}`);
  });
}

main();
