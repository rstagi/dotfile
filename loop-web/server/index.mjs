// Loop Observatory server — a pure read-only observer.
//
// Zero external deps. Does ALL filesystem I/O, then hands file *contents* to the tested
// pure model in ../src/model (Node strips the TS types at runtime). Watches the loop dir
// with a 2s debounce AND a mandatory 15s reconcile (a hung runner emits no fs event, yet
// its heartbeat must still flip live→flatline on a timer). Streams full snapshots over SSE.
//
// It NEVER writes to .kestral/loop or the plan — observation only.

import http from "node:http";
import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildSnapshot, parsePlan, parseLoop } from "../src/model/index.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DIST_DIR = path.join(__dirname, "..", "dist");

const DEBOUNCE_MS = 2000;
const RECONCILE_MS = 15000;
const TAIL_BYTES = 64 * 1024;

// --- configuration & path resolution -------------------------------------------------

function parseArgs(argv) {
  const out = {};
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dir") out.dir = argv[++i];
    else if (a === "--plan") out.plan = argv[++i];
    else if (a === "--port") out.port = argv[++i];
  }
  return out;
}

/** Walk up from `start` looking for a `.kestral/` directory; return its path or null. */
function discoverKestral(start) {
  let dir = path.resolve(start);
  for (;;) {
    const candidate = path.join(dir, ".kestral");
    if (isDir(candidate)) return candidate;
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

function resolveConfig() {
  const args = parseArgs(process.argv);
  const env = process.env;
  const port = Number(args.port ?? env.LOOP_WEB_PORT ?? 7717);
  let loopDir = args.dir ?? env.LOOP_WEB_DIR ?? null;
  let planFile = args.plan ?? env.LOOP_WEB_PLAN ?? null;

  if (!loopDir && !planFile) {
    const kestral = discoverKestral(process.cwd());
    if (kestral) {
      loopDir = path.join(kestral, "loop");
      planFile = path.join(kestral, "plan.md");
    }
  }
  // The plan sits beside the loop dir at .kestral/plan.md unless overridden.
  if (loopDir && !planFile) planFile = path.join(path.dirname(loopDir), "plan.md");

  loopDir = loopDir ? path.resolve(loopDir) : null;
  planFile = planFile ? path.resolve(planFile) : null;
  return { port, loopDir, planFile };
}

// --- filesystem readers (I/O lives here; the model layer stays pure) -----------------

function isDir(p) {
  try {
    return fs.statSync(p).isDirectory();
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
/** Read only the last TAIL_BYTES of a file (logs/transcripts can be large). */
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

/** Build the model's LoopInput by reading the loop dir. Every read is defensive. */
function readLoopInput(loopDir) {
  if (!loopDir || !isDir(loopDir)) return { present: false, state: null, events: null, runs: [], hil: [] };

  const runsDir = path.join(loopDir, "runs");
  const runs = [];
  if (isDir(runsDir)) {
    for (const name of safeReaddir(runsDir)) {
      const d = path.join(runsDir, name);
      if (!isDir(d)) continue;
      runs.push({
        name,
        meta: readText(path.join(d, "meta.json")),
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

  return {
    present: true,
    state: readText(path.join(loopDir, "state.json")),
    events: readText(path.join(loopDir, "events.jsonl")),
    runs,
    hil,
  };
}

function safeReaddir(p) {
  try {
    return fs.readdirSync(p);
  } catch {
    return [];
  }
}

// --- snapshot computation ------------------------------------------------------------

function computeSnapshot(cfg) {
  const planText = cfg.planFile ? readText(cfg.planFile) : null;
  const plan = parsePlan(planText ?? "# No plan found — Multi-Phase Plan\n");
  if (planText == null && cfg.planFile) {
    plan.warnings.push(`Plan file not found: ${cfg.planFile}`);
  }
  const runtime = cfg.loopDir ? parseLoop(readLoopInput(cfg.loopDir)) : null;
  const now = Date.now();
  return buildSnapshot(plan, runtime, { now, nowIso: new Date(now).toISOString() });
}

// --- SSE hub -------------------------------------------------------------------------

const clients = new Set();
let lastSnapshotJson = "null";

function broadcast(cfg) {
  let snap;
  try {
    snap = computeSnapshot(cfg);
    lastSnapshotJson = JSON.stringify(snap);
  } catch (err) {
    // A transient parse error must never take the server down; keep the last good snapshot.
    console.error("[loop-web] snapshot error:", err?.message ?? err);
    return;
  }
  const frame = `event: snapshot\ndata: ${lastSnapshotJson}\n\n`;
  for (const res of clients) res.write(frame);
}

// --- watching ------------------------------------------------------------------------

function startWatching(cfg, onChange) {
  const watchers = [];
  let debounce = null;
  const schedule = () => {
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(onChange, DEBOUNCE_MS);
  };

  const watchPath = (p, opts) => {
    try {
      watchers.push(fs.watch(p, opts, schedule));
    } catch {
      /* the path may not exist yet — the reconcile timer still covers it */
    }
  };
  if (cfg.loopDir) watchPath(cfg.loopDir, { recursive: true });
  if (cfg.planFile) watchPath(cfg.planFile, {});

  // Mandatory reconcile: a hung runner emits no fs event, but its stale heartbeat must
  // still surface as a flatline. This tick recomputes and rebroadcasts unconditionally.
  const timer = setInterval(onChange, RECONCILE_MS);
  timer.unref?.();
  return () => {
    for (const w of watchers) w.close();
    clearInterval(timer);
    if (debounce) clearTimeout(debounce);
  };
}

// --- HTTP ----------------------------------------------------------------------------

function handleAttempt(cfg, req, res) {
  const m = req.url.match(/^\/api\/attempt\/(.+)\/(\d+)\/?$/);
  if (!m || !cfg.loopDir) return json(res, 404, { error: "not found" });
  const slug = decodeURIComponent(m[1]);
  const k = m[2];
  const runDir = path.join(cfg.loopDir, "runs", `${slug}-a${k}`);
  // Guard against path traversal in the slug segment (require containment under runs/).
  if (!isContained(path.resolve(runDir), path.resolve(path.join(cfg.loopDir, "runs")))) {
    return json(res, 400, { error: "bad path" });
  }
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

function handleSse(cfg, req, res) {
  res.writeHead(200, {
    "Content-Type": "text/event-stream",
    "Cache-Control": "no-cache, no-transform",
    Connection: "keep-alive",
    "X-Accel-Buffering": "no",
  });
  res.write("retry: 3000\n\n");
  res.write(`event: snapshot\ndata: ${lastSnapshotJson}\n\n`);
  clients.add(res);
  req.on("close", () => clients.delete(res));
}

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
  let rel = urlPath === "/" ? "index.html" : decodeURIComponent(urlPath.replace(/^\/+/, ""));
  const filePath = path.join(DIST_DIR, rel);
  if (!isContained(path.resolve(filePath), path.resolve(DIST_DIR))) {
    res.writeHead(403).end("forbidden");
    return;
  }
  fs.readFile(filePath, (err, buf) => {
    if (err) {
      // SPA fallback → index.html (or a build hint if dist is missing).
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
(<code>/events</code>, <code>/api/model</code>) are live.</p></body>`;

function json(res, code, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(code, { "Content-Type": "application/json; charset=utf-8" });
  res.end(body);
}
function safeParse(s) {
  if (s == null) return null;
  try {
    return JSON.parse(s);
  } catch {
    return null;
  }
}
/** True iff `abs` is `base` itself or lives under it — the trailing sep stops a sibling
 * like `<base>-backup` from passing a naive startsWith check. */
function isContained(abs, base) {
  return abs === base || abs.startsWith(base + path.sep);
}

// --- main ----------------------------------------------------------------------------

function main() {
  const cfg = resolveConfig();
  broadcast(cfg); // prime lastSnapshotJson before the first client connects
  const onChange = () => broadcast(cfg);
  startWatching(cfg, onChange);

  const server = http.createServer(async (req, res) => {
    try {
      if (req.url === "/events") return handleSse(cfg, req, res);
      if (req.url === "/api/model" || req.url === "/api/snapshot") {
        res.writeHead(200, { "Content-Type": "application/json; charset=utf-8" });
        return res.end(lastSnapshotJson);
      }
      if (req.url.startsWith("/api/attempt/")) return handleAttempt(cfg, req, res);
      if (req.url.startsWith("/api/")) return json(res, 404, { error: "unknown endpoint" });
      return serveStatic(req, res);
    } catch (err) {
      json(res, 500, { error: String(err?.message ?? err) });
    }
  });

  server.listen(cfg.port, () => {
    const mode = cfg.loopDir && isDir(cfg.loopDir) ? "live" : "static";
    console.log(`[loop-web] Loop Observatory (${mode}) on http://localhost:${cfg.port}`);
    if (cfg.planFile) console.log(`[loop-web] plan: ${cfg.planFile}`);
    if (cfg.loopDir) console.log(`[loop-web] loop: ${cfg.loopDir}`);
  });
}

main();
