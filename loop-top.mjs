#!/usr/bin/env node
// loop-top — terminal dashboard + controls for Loop runs. Infers the loop from cwd (coordinator
// worktree, phase worktree, or git branch), renders its phases live from the central daemon
// (SSE → 2s poll → ~/.loop/loops store fallback). Steering (notes, pause, model override) goes
// through the daemon's /note and /control endpoints. Zero-dep, Node ≥22.
//
//   loop-top [runId]                 interactive (alt screen); keys in LIST_HINTS
//   loop-top --once [--worktrees] [--graph]   print one frame to stdout and exit
//   loop-top archive [runId] [--yes] hide a loop from selectors (default: the inferred one;
//                                    worktrees untouched) · loop-top unarchive <runId>
//
// Status comes from the daemon's materialized snapshot (never re-derived here). Phase worktree
// paths aren't in the snapshot, so they're read from the coordinator's .loop/state.json, else
// the store record's lastState.
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";

const DAEMON_URL = process.env.LOOP_DAEMON_URL || "http://localhost:7717";
const STORE_DIR = process.env.LOOP_STORE_DIR || path.join(os.homedir(), ".loop", "loops");
const HTTP_TIMEOUT_MS = 1500;
const POLL_MS = 2000;
const SELF = new URL(import.meta.url).pathname;
const MODELS_CONF = process.env.LOOP_MODELS_CONF || path.join(path.dirname(new URL(import.meta.url).pathname), "loop-models.conf");
const DEFAULT_CHAIN = "default (loop-models.conf chain)";
const LIST_HINTS = "↑↓ select · ⏎ details · n note · x pause · X pause loop · m model · p plan · g graph · w worktrees · l loops · q quit";
const DETAIL_HINTS = "esc back · ↑↓ prev/next · n note · x pause · X pause loop · m model · p plan";
const ICONS = { done: "✓", running: "◐", awaiting: "⏸", paused: "‖", problem: "✗", blocked: "✗", todo: "·" };
const WAITING_TEXT = "waiting on you (orchestrator)";
const ESC = "\x1b[";
const STYLE = { dim: "2", bold: "1", green: "32", yellow: "33", red: "31", cyan: "36" };
const ICON_STYLE = { "✓": STYLE.green, "◐": STYLE.cyan, "⏸": STYLE.yellow, "‖": STYLE.yellow, "✗": STYLE.red, "·": STYLE.dim };

if (isMain()) {
  main(process.argv.slice(2)).catch((err) => {
    restoreTerminal();
    console.error(`loop-top: ${err.message}`);
    process.exit(1);
  });
}

async function main(argv) {
  if (argv.includes("--picker-rows")) return console.log(pickerRows((await loadLoops()).loops).join("\n"));
  if (argv[0] === "archive" || argv[0] === "unarchive") return archiveCommand(argv[0] === "archive", argv.slice(1));
  const once = argv.includes("--once");
  const showWorktrees = argv.includes("--worktrees");
  const graph = argv.includes("--graph");
  const explicit = argv.find((a) => !a.startsWith("--")) ?? null;

  const { loops, online } = await loadLoops();
  const runId = explicit ?? inferLoop(cwdContext(), loops);

  if (once) {
    if (!runId) throw new Error("no loop inferred from cwd — pass a runId (see `loop-top` picker)");
    const frame = await fetchFrame(runId, loops, online);
    process.stdout.write(renderFrame(frame, { width: process.stdout.columns || 120, showWorktrees, graph, color: false }) + "\n");
    return;
  }
  await interactive(runId, loops, { showWorktrees, graph });
}

/** `loop-top archive|unarchive`: daemon flag when it's up, else the store record directly
 * (the daemon reloads the store on start). */
async function archiveCommand(archive, argv) {
  const verb = archive ? "archive" : "unarchive";
  const explicit = argv.find((a) => !a.startsWith("--")) ?? null;
  const { loops, online } = await loadLoops();
  const runId = explicit ?? (archive ? inferLoop(cwdContext(), loops) : null);
  if (!runId) throw new Error(`no loop inferred from cwd — pass a runId (loop-top ${verb} <runId>)`);
  const status = loops.find((l) => l.runId === runId)?.status ?? "unknown";
  if (archive && !argv.includes("--yes") && !(await confirm(`archive ${runId} (${status})?`))) {
    console.log("not archived");
    return;
  }
  if (online) {
    const res = await postJson(`/api/loops/${encodeURIComponent(runId)}/${verb}`, {});
    if (!res.ok) throw new Error(`${res.error}: ${runId}`);
  } else {
    const record = readStoreRecord(runId);
    if (!record) throw new Error(`no such loop: ${runId}`);
    writeStoreRecord({ ...record, archived: archive });
  }
  console.log(`${verb}d ${runId}`);
}

/** cwd → runId. Rules in order, first hit wins; ties → newest updatedAt. */
export function inferLoop({ cwd, localRunId = null, branch = null, originSlug = null }, loops) {
  const candidates = loops.filter((l) => l.status !== "archived");
  const rules = [
    (l) => l.runId === localRunId || isInside(cwd, l.coordinatorDir),
    (l) => l.worktrees.some((w) => isInside(cwd, w)),
    (l) =>
      Boolean(branch) &&
      (l.integrationBranch === branch || l.branches.includes(branch)) &&
      l.repositories.includes(originSlug),
  ];
  for (const rule of rules) {
    const hit = newest(candidates.filter(rule));
    if (hit) return hit.runId;
  }
  return null;
}

/** Snapshot graph nodes (+ coordinator state for worktrees) → ordered display rows. */
export function phaseRows(snapshot, state) {
  const nodes = snapshot?.graph?.nodes ?? [];
  const labelOf = new Map(nodes.map((n) => [n.id, n.phase ?? n.id]));
  const depEdges = (snapshot?.graph?.edges ?? []).filter((e) => e.kind === "depends");
  const plannedLeg = firstTaskLeg();
  return nodes
    .filter((n) => n.kind === "phase" || n.kind === "pr-review")
    .sort((a, b) => comparePhase(a.phase ?? a.id, b.phase ?? b.id))
    .map((n) => {
      const ps = state?.phases?.[n.id] ?? {};
      const waiting = Boolean(n.runtime?.hilOpen || n.runtime?.awaiting) || n.ui === "awaiting";
      const incoming = depEdges.filter((e) => e.target === n.id);
      const label = (e) => labelOf.get(e.source) ?? e.source;
      const pending = incoming.filter((e) => e.blocking).map(label).sort(comparePhase);
      return {
        id: n.id,
        label: n.phase ?? n.id,
        title: n.title,
        lane: n.lane,
        model: actualModel(n.runtime),
        modelOverride: n.modelOverride ?? null,
        planned: n.kind === "phase" && n.ui !== "done" ? plannedLeg : null,
        paused: n.paused === true,
        noted: Boolean(n.noteMarkdown?.trim()),
        review: n.kind === "pr-review" ? (n.review ?? null) : null,
        started: (n.status != null && n.status !== "todo") || (n.runtime?.attempt ?? 0) > 0,
        deps: incoming.map(label).sort(comparePhase),
        icon: waiting ? ICONS.awaiting : n.paused && n.ui !== "done" ? ICONS.paused : (ICONS[n.ui] ?? ICONS.todo),
        state: waiting ? WAITING_TEXT : n.ui === "todo" && pending.length ? `waiting on ${pending.join(", ")}` : stateText(n),
        branch: ps.branch ?? n.runtime?.branch ?? null,
        worktree: ps.worktree ?? null,
      };
    });
}

// ---------------------------------------------------------------------------------------
// Interactive mode
// ---------------------------------------------------------------------------------------

async function interactive(initialRunId, initialLoops, { showWorktrees, graph }) {
  if (!process.stdin.isTTY || !process.stdout.isTTY) throw new Error("interactive mode needs a TTY (use --once)");
  const ui = {
    loops: initialLoops, runId: initialRunId, frame: null, showWorktrees, graph, live: null,
    selected: null, details: false, confirm: null, message: null, messageAt: 0,
  };

  if (!ui.runId) ui.runId = pickLoop(ui.loops);
  if (!ui.runId) return;

  enterTerminal();
  const draw = () => {
    if (!ui.frame) return;
    const width = process.stdout.columns || 100;
    const height = process.stdout.rows || 40;
    const rows = displayRows(ui.frame, ui.graph);
    if (!rows.some((r) => r.id === ui.selected)) ui.selected = (rows.find((r) => r.icon !== ICONS.done) ?? rows[0])?.id ?? null;
    if (ui.message && Date.now() - ui.messageAt > 5000) ui.message = null;
    const banner = ui.confirm?.prompt ?? ui.message;
    const opts = { width, height, showWorktrees: ui.showWorktrees, graph: ui.graph, color: true, keys: true, selected: ui.selected, banner };
    const body = ui.details ? renderDetails(ui.frame, ui.selected, opts) : renderFrame(ui.frame, opts);
    process.stdout.write(`${ESC}H` + body.split("\n").map((l) => l + `${ESC}K`).join("\n") + `${ESC}J`);
  };
  const say = (msg) => {
    ui.message = msg;
    ui.messageAt = Date.now();
    draw();
  };
  const follow = () => {
    ui.live?.stop();
    ui.live = followLoop(ui.runId, () => ui.loops, (frame) => {
      ui.frame = frame;
      draw();
    });
  };
  // Run a full-screen child (fzf, less, $EDITOR) with the terminal handed over, then redraw.
  const handOver = (fn) => {
    leaveTerminal();
    try {
      return fn();
    } finally {
      enterTerminal();
      draw();
    }
  };
  const selectedRow = () => (ui.frame ? displayRows(ui.frame, ui.graph).find((r) => r.id === ui.selected) : null);
  const moveCursor = (delta) => {
    const rows = ui.frame ? displayRows(ui.frame, ui.graph) : [];
    const i = rows.findIndex((r) => r.id === ui.selected);
    const next = rows[Math.min(rows.length - 1, Math.max(0, i + delta))];
    if (next) ui.selected = next.id;
    draw();
  };
  follow();

  const tick = setInterval(draw, 1000); // keeps elapsed time fresh between snapshots
  const refreshLoops = setInterval(async () => {
    ui.loops = (await loadLoops()).loops;
  }, 10_000);
  process.stdout.on("resize", draw);

  const actions = {
    g: () => ((ui.graph = !ui.graph), draw()),
    w: () => ((ui.showWorktrees = !ui.showWorktrees), draw()),
    k: () => moveCursor(-1),
    j: () => moveCursor(1),
    [`${ESC}A`]: () => moveCursor(-1),
    [`${ESC}B`]: () => moveCursor(1),
    "\r": () => ((ui.details = !ui.details), draw()),
    "\x1b": () => ((ui.details = false), draw()),
    l: () => {
      const picked = handOver(() => pickLoop(ui.loops));
      if (picked && picked !== ui.runId) {
        Object.assign(ui, { runId: picked, frame: null, selected: null, details: false });
        follow();
      }
    },
    p: async () => {
      const plan = await loadPlanText(ui.runId);
      if (!plan) return say("no plan text for this loop");
      handOver(() => spawnSync(process.env.PAGER || "less", ["-R"], { input: plan, stdio: ["pipe", "inherit", "inherit"] }));
    },
    n: async () => {
      const row = selectedRow();
      if (!row) return;
      const current = ui.frame.snapshot?.graph?.nodes?.find((n) => n.id === row.id)?.noteMarkdown ?? "";
      const edited = handOver(() => editText(current, `loop-note-${row.label}`));
      if (edited == null || edited.trim() === current.trim()) return say("note unchanged");
      const body = edited.trim() ? { key: row.id, markdown: edited } : { key: row.id, clear: true };
      const res = await postJson(`/api/loops/${encodeURIComponent(ui.runId)}/note`, body);
      say(res.ok ? (edited.trim() ? `note saved for ${row.label}` : `note cleared for ${row.label}`) : `note failed: ${res.error}`);
    },
    x: async () => {
      const row = selectedRow();
      if (!row) return;
      const action = row.paused ? "resume" : "pause";
      const res = await postJson(`/api/loops/${encodeURIComponent(ui.runId)}/control`, { action, phase: row.id });
      say(res.ok ? `${action === "pause" ? "paused" : "resumed"} phase ${row.label}` : `${action} failed: ${res.error}`);
    },
    X: () => {
      const action = ui.frame?.snapshot?.paused ? "resume" : "pause";
      ui.confirm = {
        prompt: `${action === "pause" ? "Pause the WHOLE loop (orchestrator + all runners)" : "Resume the whole loop"}? [y/N]`,
        run: async () => {
          const res = await postJson(`/api/loops/${encodeURIComponent(ui.runId)}/control`, { action });
          say(res.ok ? `loop ${action === "pause" ? "paused" : "resumed"}` : `${action} failed: ${res.error}`);
        },
      };
      draw();
    },
    m: async () => {
      const row = selectedRow();
      if (!row) return;
      if (row.started) return say(`phase ${row.label} already started — ${row.review ? "tier" : "model"} can only change before it starts`);
      if (row.review) {
        const tier = handOver(() => pickTier(row));
        if (!tier) return;
        const res = await postJson(`/api/loops/${encodeURIComponent(ui.runId)}/control`, { action: "review", phase: row.id, tier });
        return say(res.ok ? `phase ${row.label} will review at ${tier}` : `tier failed: ${res.error}`);
      }
      const leg = handOver(() => pickModel(row));
      if (leg === undefined) return;
      const res = await postJson(`/api/loops/${encodeURIComponent(ui.runId)}/control`, { action: "model", phase: row.id, leg });
      say(res.ok ? (leg ? `phase ${row.label} will run on ${leg}` : `phase ${row.label} back to the default chain`) : `model failed: ${res.error}`);
    },
  };

  await new Promise((resolve) => {
    process.stdin.setRawMode(true);
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", async (key) => {
      if (key === "\u0003") return resolve();
      if (ui.confirm) {
        const { run } = ui.confirm;
        ui.confirm = null;
        if (key === "y" || key === "Y") await run();
        else say("cancelled");
        return;
      }
      if (key === "q") return ui.details ? actions["\x1b"]() : resolve();
      await actions[key]?.();
    });
  });

  clearInterval(tick);
  clearInterval(refreshLoops);
  ui.live?.stop();
  restoreTerminal();
  process.exit(0);
}

/** fzf over the model legs in loop-models.conf; typed text is accepted as a custom leg.
 * Returns a leg, null (back to default chain), or undefined (cancelled). */
/** fzf over the review tiers (rounds + models from loop-models.conf) → tier, or null. */
function pickTier(row) {
  const res = spawnSync("fzf", ["--prompt", `phase ${row.label} review tier> `, "--height", "40%", "--reverse", "--no-sort"], {
    input: tierChoices(readFileSafe(MODELS_CONF), row.review?.tier).join("\n"),
    encoding: "utf8", stdio: ["pipe", "pipe", "inherit"],
  });
  if (res.error || res.status !== 0) return null;
  return res.stdout.trim().split(/\s+/)[0] || null;
}

/** One picker line per review tier: `tier ×rounds review <adversaries> · fix <m> · final <m>`. */
export function tierChoices(confText, current = null) {
  const first = (key) => confText.match(new RegExp(`^CHAIN_REVIEW_${key}=\\(\\s*"([^"]+)"`, "m"))?.[1] ?? null;
  return ["shallow", "medium", "max"].map((tier) => {
    const T = tier.toUpperCase();
    const rounds = confText.match(new RegExp(`^LOOP_REVIEW_ROUNDS_${T}=(\\d+)`, "m"))?.[1] ?? { shallow: 1, medium: 3, max: 3 }[tier];
    const reviewers = [first(`${T}_ADV_A`), first(`${T}_ADV_B`)].filter(Boolean).join(" + ");
    const line = `${tier.padEnd(8)} ×${rounds}  review ${reviewers} · fix ${first(`${T}_FIX`)} · final ${first(`${T}_FINAL`)}`;
    return tier === current ? `${line}  (current)` : line;
  });
}

function pickModel(row) {
  const legs = [...new Set([...readFileSafe(MODELS_CONF).matchAll(/"((?:codex|claude):[^"\s]+)"/g)].map((m) => m[1]))];
  const input = [DEFAULT_CHAIN, ...legs].join("\n");
  const res = spawnSync("fzf", ["--prompt", `phase ${row.label} model> `, "--height", "40%", "--reverse", "--print-query",
    "--header", "pick a leg, or type engine:model (codex|claude) and press enter"], {
    input, encoding: "utf8", stdio: ["pipe", "pipe", "inherit"],
  });
  if (res.error || res.status === 130) return undefined;
  const [query = "", pick = ""] = res.stdout.split("\n");
  const choice = (pick || query).trim();
  if (!choice) return undefined;
  return choice === DEFAULT_CHAIN ? null : choice;
}

/** Open $EDITOR on `text`; returns the saved text, or null if the editor failed. */
function editText(text, name) {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "loop-top-")), `${name}.md`);
  fs.writeFileSync(file, text);
  const editor = process.env.VISUAL || process.env.EDITOR || "vi";
  const res = spawnSync(process.env.SHELL || "/bin/sh", ["-c", `${editor} "$1"`, "sh", file], { stdio: "inherit" });
  const out = res.status === 0 ? readFileSafe(file) : null;
  fs.rmSync(path.dirname(file), { recursive: true, force: true });
  return out;
}

async function loadPlanText(runId) {
  const plan = await getJson(`/api/loops/${encodeURIComponent(runId)}/plan`);
  return plan?.planText ?? readStoreRecord(runId)?.planText ?? null;
}

/** Live feed for one loop: SSE first, 2s snapshot poll if the stream drops, store if offline. */
function followLoop(runId, getLoops, onFrame) {
  let stopped = false;
  let req = null;
  let timer = null;

  const poll = async () => {
    if (stopped) return;
    onFrame(await fetchFrame(runId, getLoops(), null));
    timer = setTimeout(poll, POLL_MS);
  };

  // Immediate first frame (also covers a loop the daemon streams as `null`: store fallback).
  fetchFrame(runId, getLoops(), null).then((f) => !stopped && onFrame(f), () => {});

  req = http.get(`${DAEMON_URL}/events?runId=${encodeURIComponent(runId)}`, (res) => {
    if (res.statusCode !== 200) return fallback();
    res.setEncoding("utf8");
    let buf = "";
    res.on("data", (chunk) => {
      buf += chunk;
      let idx;
      while ((idx = buf.indexOf("\n\n")) >= 0) {
        const block = buf.slice(0, idx);
        buf = buf.slice(idx + 2);
        const snapshot = parseSseSnapshot(block);
        if (snapshot) onFrame(frameFrom(runId, getLoops(), snapshot, "live"));
      }
    });
    res.on("end", fallback);
    res.on("error", fallback);
  });
  req.on("error", fallback);

  function fallback() {
    if (stopped || timer) return;
    req?.destroy();
    poll();
  }

  return {
    stop() {
      stopped = true;
      req?.destroy();
      clearTimeout(timer);
    },
  };
}

/** fzf over non-archived loops; returns the chosen runId or null. Opens in navigation mode
 * (no search box): letters are commands (`a` archives + reloads, `q` quits, j/k move) until
 * `/` shows the search box; esc then clears + hides it again. */
function pickLoop(loops) {
  const rows = pickerRows(loops);
  if (rows.length === 0) {
    console.error("loop-top: no active or finished loops");
    return null;
  }
  const self = `${shq(process.execPath)} ${shq(SELF)}`;
  // a letter types into the search box when it's shown, else runs its command
  const key = (k, cmd) => `${k}:transform:[ "$FZF_INPUT_STATE" = enabled ] && echo put:${k} || echo ${shq(cmd)}`;
  const res = spawnSync("fzf", [
    "--prompt", "loop> ", "--height", "40%", "--reverse", "--no-sort", "--no-input",
    "--header", "⏎ open · a archive · / search · q quit",
    "--bind", "/:show-input",
    "--bind", `esc:transform:[ "$FZF_INPUT_STATE" = enabled ] && echo clear-query+hide-input || echo abort`,
    "--bind", key("a", `execute-silent(${self} archive {2} --yes)+reload(${self} --picker-rows)`),
    "--bind", key("q", "abort"),
    "--bind", key("j", "down"),
    "--bind", key("k", "up"),
  ], { input: rows.join("\n"), encoding: "utf8", stdio: ["pipe", "pipe", "inherit"] });
  if (res.error) {
    console.error("loop-top: fzf not found — pass a runId instead");
    return null;
  }
  return res.stdout.trim().split(/\s+/)[1] ?? null;
}

/** Picker lines: `status  runId  updatedAt`, active first, archived hidden. */
export function pickerRows(loops) {
  const rank = (l) => (l.status === "active" ? 0 : 1);
  return loops
    .filter((l) => l.status !== "archived")
    .sort((a, b) => rank(a) - rank(b))
    .map((l) => `${l.status.padEnd(9)}  ${l.runId}  ${l.updatedAt ?? ""}`);
}

// ---------------------------------------------------------------------------------------
// Data: daemon first, store fallback
// ---------------------------------------------------------------------------------------

/** Loop refs (daemon summary + store record) for inference/picker; `online` = daemon reachable. */
async function loadLoops() {
  const summaries = await getJson("/api/loops");
  const records = new Map(readStoreRecords().map((r) => [r.runId, r]));
  if (summaries) return { loops: summaries.map((s) => loopRef(s, records.get(s.runId))), online: true };
  const loops = [...records.values()].map((r) => loopRef(storeSummary(r), r));
  loops.sort((a, b) => String(b.updatedAt ?? "").localeCompare(String(a.updatedAt ?? "")));
  return { loops, online: false };
}

/** One renderable frame: snapshot from the daemon (or store), worktrees from coordinator state. */
async function fetchFrame(runId, loops, online) {
  const snapshot = online === false ? null : await getJson(`/api/loops/${encodeURIComponent(runId)}/snapshot`);
  if (snapshot) return frameFrom(runId, loops, snapshot, "live");
  const record = readStoreRecord(runId);
  if (!record) throw new Error(`no such loop: ${runId}`);
  return frameFrom(runId, loops, record.lastSnapshot, "store");
}

function frameFrom(runId, loops, snapshot, source) {
  const ref = loops.find((l) => l.runId === runId) ?? loopRef(storeSummary(readStoreRecord(runId) ?? { runId }), null);
  return { ref, snapshot, state: readPhaseState(ref), source, at: new Date() };
}

/** Normalize a daemon summary + store record into what inferLoop/picker/header need. */
function loopRef(summary, record) {
  const phases = Object.values(record?.lastState?.phases ?? {});
  const repos = Object.values(record?.repositories ?? {});
  const loopDir = record?.loopDir ?? null;
  return {
    runId: summary.runId,
    status: summary.status ?? "unknown",
    updatedAt: summary.updatedAt ?? null,
    startedAt: summary.startedAt ?? null,
    finishedAt: summary.finishedAt ?? null,
    integrationBranch: summary.integrationBranch ?? null,
    repositories: (summary.repositories ?? Object.keys(record?.repositories ?? {})).map((r) => r?.slug ?? r),
    loopDir,
    coordinatorDir: loopDir ? realpath(path.dirname(loopDir)) : null,
    worktrees: [...phases.map((p) => p.worktree), ...repos.map((r) => r.integrationWorktree)].filter(Boolean).map(realpath),
    branches: [...phases.map((p) => p.branch), ...repos.map((r) => r.integrationBranch)].filter(Boolean),
  };
}

/** The daemon's listLoops() summary shape, rebuilt from a store record (daemon offline). */
function storeSummary(record) {
  let status = record.status ?? "unknown";
  if (status !== "planned" && !isDir(record.loopDir)) status = "archived";
  if (record.archived) status = "archived";
  return {
    runId: record.runId,
    status,
    updatedAt: record.updatedAt ?? null,
    startedAt: record.startedAt ?? null,
    finishedAt: record.finishedAt ?? null,
    integrationBranch: record.integrationBranch ?? null,
    repositories: Object.keys(record.repositories ?? {}),
  };
}

/** Live coordinator state (fresher) if the worktree still exists, else the store's last copy. */
function readPhaseState(ref) {
  if (ref.loopDir) {
    const live = readJson(path.join(ref.loopDir, "state.json"));
    if (live?.phases) return live;
  }
  return readStoreRecord(ref.runId)?.lastState ?? null;
}

function readStoreRecords() {
  let names = [];
  try {
    names = fs.readdirSync(STORE_DIR).filter((n) => n.endsWith(".json"));
  } catch {
    return [];
  }
  return names.map((n) => readJson(path.join(STORE_DIR, n))).filter((r) => r?.runId);
}

function readStoreRecord(runId) {
  return readJson(path.join(STORE_DIR, `${runId}.json`));
}

/** Atomic rewrite (tmp + rename), like the daemon's store. */
function writeStoreRecord(record) {
  const dest = path.join(STORE_DIR, `${record.runId}.json`);
  const tmp = `${dest}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(record));
  fs.renameSync(tmp, dest);
}

/** y/N on the terminal; non-interactive stdin counts as "no" (use --yes). */
async function confirm(question) {
  if (!process.stdin.isTTY) return false;
  const { createInterface } = await import("node:readline/promises");
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  const ans = await rl.question(`${question} [y/N] `);
  rl.close();
  return /^y(es)?$/i.test(ans.trim());
}

/** What cwd says about which loop we're in: nearest .loop/state.json runId, git branch, origin. */
function cwdContext() {
  const cwd = realpath(process.cwd());
  return { cwd, localRunId: nearestLocalRunId(cwd), branch: git(cwd, "rev-parse", "--abbrev-ref", "HEAD"), originSlug: originSlug(cwd) };
}

function nearestLocalRunId(dir) {
  for (let d = dir; ; d = path.dirname(d)) {
    const state = readJson(path.join(d, ".loop", "state.json"));
    if (state?.runId) return state.runId;
    if (path.basename(d) === ".loop") {
      const own = readJson(path.join(d, "state.json"));
      if (own?.runId) return own.runId;
    }
    if (d === path.dirname(d)) return null;
  }
}

/** git@github.com:owner/repo.git | https://github.com/owner/repo(.git) → owner/repo. */
function originSlug(cwd) {
  const url = git(cwd, "remote", "get-url", "origin");
  const m = url && /[:/]([^/:]+\/[^/]+?)(?:\.git)?\/?$/.exec(url);
  return m ? m[1] : null;
}

function git(cwd, ...args) {
  try {
    return execFileSync("git", ["-C", cwd, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim() || null;
  } catch {
    return null;
  }
}

function getJson(urlPath) {
  return new Promise((resolve) => {
    const req = http.get(`${DAEMON_URL}${urlPath}`, { timeout: HTTP_TIMEOUT_MS }, (res) => {
      let body = "";
      res.setEncoding("utf8");
      res.on("data", (c) => (body += c));
      res.on("end", () => {
        if (res.statusCode !== 200) return resolve(null);
        try {
          resolve(JSON.parse(body));
        } catch {
          resolve(null);
        }
      });
    });
    req.on("timeout", () => req.destroy());
    req.on("error", () => resolve(null));
  });
}

/** POST JSON to the daemon → { ok, status, body, error } (error carries the daemon's message). */
function postJson(urlPath, payload) {
  return new Promise((resolve) => {
    const data = JSON.stringify(payload);
    const req = http.request(`${DAEMON_URL}${urlPath}`, {
      method: "POST", timeout: HTTP_TIMEOUT_MS,
      headers: { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(data) },
    }, (res) => {
      let body = "";
      res.setEncoding("utf8");
      res.on("data", (c) => (body += c));
      res.on("end", () => {
        let parsed = null;
        try {
          parsed = JSON.parse(body);
        } catch {}
        const ok = res.statusCode === 200;
        resolve({ ok, status: res.statusCode, body: parsed, error: ok ? null : (parsed?.error ?? `HTTP ${res.statusCode}`) });
      });
    });
    req.on("timeout", () => req.destroy(new Error("timeout")));
    req.on("error", (e) => resolve({ ok: false, status: 0, body: null, error: `daemon unreachable (${e.message})` }));
    req.end(data);
  });
}

/** One SSE block → snapshot object, or null (comments, retry lines, `null` payloads). */
function parseSseSnapshot(block) {
  const lines = block.split("\n");
  if (!lines.includes("event: snapshot")) return null;
  const data = lines.filter((l) => l.startsWith("data: ")).map((l) => l.slice(6)).join("\n");
  try {
    return JSON.parse(data);
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------------------

function renderFrame(frame, { width, height = null, showWorktrees, graph = false, color, keys = false, selected = null, banner = null }) {
  const paint = (style, s) => (color ? `${ESC}${style}m${s}${ESC}0m` : s);
  const { ref, snapshot, state, source, at } = frame;
  const elapsed = ref.startedAt ? formatAge(((ref.finishedAt ? Date.parse(ref.finishedAt) : Date.now()) - Date.parse(ref.startedAt)) / 1000) : "—";
  const out = [...renderHeader(frame, width, paint), ""];

  const rows = phaseRows(snapshot, state);
  const layout = graph ? graphLayout(rows, snapshot?.graph?.edges ?? []) : null;
  const railW = layout ? layout.width : 0;
  const ordered = layout ? layout.entries.map((e) => e.row) : rows;
  const labelW = Math.max(1, ...rows.map((r) => r.label.length));
  const laneW = Math.max(1, ...rows.map((r) => (r.lane ?? "—").length));
  const modelText = (r) =>
    r.model ?? (r.modelOverride ? `→ ${r.modelOverride}` : r.review ? reviewText(r.review) : r.planned ? `(${r.planned})` : "—");
  const engineW = Math.min(28, Math.max(1, ...rows.map((r) => modelText(r).length)));
  const depsText = (r) => (r.deps.length ? `← ${r.deps.join(", ")}` : "");
  const depsW = graph ? 0 : Math.min(16, Math.max(0, ...rows.map((r) => depsText(r).length)));
  const stateW = Math.min(32, Math.max(...rows.map((r) => r.state.length), 4));
  const titleW = Math.max(10, width - (4 + railW + labelW + 2 + 2 + laneW + 2 + engineW + 2 + (depsW ? depsW + 2 : 0) + stateW));
  // Rails are dim; the node glyph keeps its status colour.
  const rails = (prefix, glyph = null) => {
    const padded = prefix.padEnd(railW);
    const at = glyph ? padded.indexOf(glyph) : -1;
    if (at < 0) return paint(STYLE.dim, padded);
    return paint(STYLE.dim, padded.slice(0, at)) + paint(ICON_STYLE[glyph], glyph) + paint(STYLE.dim, padded.slice(at + glyph.length));
  };
  const groups = ordered.map((r, i) => {
    const entry = layout?.entries[i];
    const cols = [
      r.label.padEnd(labelW),
      truncate(rowTitle(r), titleW).padEnd(titleW),
      (r.lane ?? "—").padEnd(laneW),
      truncate(modelText(r), engineW).padEnd(engineW),
      ...(depsW ? [paint(STYLE.dim, truncate(depsText(r), depsW).padEnd(depsW))] : []),
      truncate(r.state, stateW),
    ];
    const isSelected = r.id === selected;
    if (isSelected) cols[1] = paint(STYLE.bold, cols[1]);
    const cursor = isSelected ? paint(STYLE.cyan, "▸ ") : "  ";
    const lines = [];
    if (entry) {
      lines.push(...entry.before.map((l) => "  " + rails(l)));
      lines.push(`${cursor}${rails(entry.node, r.icon)}${cols.join("  ")}`);
    } else {
      lines.push(`${cursor}${paint(ICON_STYLE[r.icon], r.icon)} ${cols.join("  ")}`);
    }
    if (showWorktrees && (r.branch || r.worktree)) {
      const wt = r.worktree ? r.worktree.replace(os.homedir(), "~") : "—";
      const lead = `${entry ? entry.mid.padEnd(railW) : "    "}  ${r.branch ?? "—"} → `;
      lines.push(paint(STYLE.dim, "  " + lead + truncateLeft(wt, width - lead.length - 2)));
    }
    if (entry) lines.push(...entry.after.map((l) => "  " + rails(l)));
    return { done: r.icon === ICONS.done, selected: isSelected, lines };
  });
  out.push(...fitRows(groups, height ? height - out.length - (keys ? 3 : 2) : Infinity, paint));
  if (rows.length === 0) out.push(paint(STYLE.dim, "  (no phases yet)"));

  out.push("", ...renderFooter(frame, { width, keys, banner, paint, hints: LIST_HINTS }));
  return out.join("\n");
}

/** Phase detail pane for the selected row: everything the snapshot + state know about it. */
function renderDetails(frame, id, { width, height, color, banner }) {
  const paint = (style, s) => (color ? `${ESC}${style}m${s}${ESC}0m` : s);
  const { snapshot, state } = frame;
  const node = snapshot?.graph?.nodes?.find((n) => n.id === id);
  const row = phaseRows(snapshot, state).find((r) => r.id === id);
  const out = [...renderHeader(frame, width, paint), ""];
  if (!node || !row) {
    out.push(paint(STYLE.dim, "  (phase not found)"));
  } else {
    const rt = node.runtime ?? {};
    const field = (k, v) => out.push(truncate(`  ${paint(STYLE.dim, k.padEnd(11))} ${v ?? "—"}`, width + (color ? 12 : 0)));
    out.push(`  ${paint(ICON_STYLE[row.icon], row.icon)} ${paint(STYLE.bold, truncate(`${row.label}  ${rowTitle(row)}`, width - 6))}`, "");
    field("state", `${row.state}${node.status && node.status !== node.ui ? ` (${node.status})` : ""}`);
    field("lane", node.lane);
    field("repository", node.repository);
    field("model", row.model);
    field("override", row.modelOverride);
    if (row.review) field("review", `${reviewText(row.review)} (tier: shallow | medium | max)`);
    field("depends on", row.deps.join(", ") || null);
    field("branch", row.branch);
    field("worktree", row.worktree?.replace(os.homedir(), "~"));
    field("attempt", rt.attempt);
    field("heartbeat", rt.lastHeartbeatAgeSec != null ? `${formatAge(rt.lastHeartbeatAgeSec)} ago` : null);
    field("run dir", rt.runDir);
    if (rt.attempts?.length) {
      out.push("", paint(STYLE.dim, "  attempts"));
      for (const a of rt.attempts) {
        const leg = [a.engine, a.model].filter(Boolean).join(":") || "—";
        out.push(truncate(`    #${a.k}  ${leg.padEnd(28)} ${a.outcome ?? (a.ended ? "ended" : "running")}${a.problem ? ` · ${a.problem}` : ""}`, width));
      }
    }
    const block = (title, md) => {
      if (!md) return;
      out.push("", paint(STYLE.dim, `  ${title}`), ...wrapLines(md, width - 4).map((l) => `    ${l}`));
    };
    block("waiting on you (answer via the orchestrating agent)", rt.hilOpen ? rt.hilMarkdown : null);
    block("note", node.noteMarkdown);
  }
  const footer = renderFooter(frame, { width, keys: true, banner, paint, hints: DETAIL_HINTS });
  const room = (height ?? Infinity) - footer.length - 1;
  if (out.length > room) out.splice(room - 1, out.length, paint(STYLE.dim, "  … (enlarge the terminal)"));
  return [...out, "", ...footer].join("\n");
}

function renderHeader({ ref, snapshot }, width, paint) {
  const elapsed = ref.startedAt ? formatAge(((ref.finishedAt ? Date.parse(ref.finishedAt) : Date.now()) - Date.parse(ref.startedAt)) / 1000) : "—";
  const head = truncate([ref.runId, ref.status, elapsed, ref.integrationBranch].filter(Boolean).join(" · "), width);
  return [
    paint(STYLE.bold, head) + (snapshot?.paused ? "  " + paint(STYLE.yellow, "‖ LOOP PAUSED") : ""),
    truncate(`repos: ${ref.repositories.join(", ") || "—"}`, width),
  ];
}

/** Status line (or a transient banner/confirm prompt), plus key hints in interactive mode. */
function renderFooter({ source, at }, { width, keys, banner, paint, hints }) {
  const status = `updated ${at.toTimeString().slice(0, 8)} · ${source === "live" ? "live" : "daemon offline · from store"}`;
  if (!keys) return [status];
  const left = banner ? paint(STYLE.yellow, truncate(banner, width - status.length - 2)) : "";
  const pad = " ".repeat(Math.max(2, width - (banner ? Math.min(banner.length, width - status.length - 2) : 0) - status.length));
  return [paint(STYLE.dim, truncate(hints, width)), left + pad + paint(STYLE.dim, status)];
}

/** Rows in on-screen order (graph mode reorders topologically) — what the cursor walks. */
function displayRows(frame, graph) {
  const rows = phaseRows(frame.snapshot, frame.state);
  return graph ? graphLayout(rows, frame.snapshot?.graph?.edges ?? []).entries.map((e) => e.row) : rows;
}

/** git-log-style rails: phases in topological order (ties → phase number), one column per
 * open dependency edge, `├─╮` forks and `├─╯` joins. Transitive edges are dropped so the
 * rails show structure, not every dependency (the list view's deps column has those). */
function graphLayout(rows, edges) {
  const byId = new Map(rows.map((r) => [r.id, r]));
  const deps = edges.filter((e) => e.kind === "depends" && byId.has(e.source) && byId.has(e.target));
  const children = new Map(rows.map((r) => [r.id, []]));
  for (const e of deps) children.get(e.source).push(e.target);
  const reduced = new Map([...children].map(([id, kids]) => [id, kids.filter((k) => !reachableAvoiding(children, id, k))]));
  const order = topoOrder(rows, deps);
  const pos = new Map(order.map((r, i) => [r.id, i]));

  const cols = [];
  const entries = [];
  for (const row of order) {
    const before = [];
    const matches = cols.flatMap((t, i) => (t === row.id ? [i] : []));
    let c = matches[0];
    if (c === undefined) {
      c = firstFree(cols);
      cols[c] = row.id;
    } else if (matches.length > 1) {
      before.push(connector(cols, c, matches.slice(1), "join"));
      for (const m of matches.slice(1)) cols[m] = null;
    }
    const node = railCells(cols, c, row.icon);

    const kids = reduced.get(row.id).sort((a, b) => pos.get(a) - pos.get(b));
    cols[c] = kids[0] ?? null;
    const mid = railCells(cols, -1);
    const extra = kids.slice(1).map((k) => {
      const j = firstFree(cols, c);
      cols[j] = k;
      return j;
    });
    const after = extra.length ? [connector(cols, c, extra, "fork")] : [];
    while (cols.length && cols[cols.length - 1] == null) cols.pop();
    entries.push({ row, before, node, mid, after });
  }
  const width = Math.max(0, ...entries.flatMap((e) => [...e.before, e.node, ...e.after].map((l) => l.length)));
  return { entries, width };
}

/** Kahn's algorithm, always taking the ready phase with the lowest number; cycles append in order. */
function topoOrder(rows, deps) {
  const indeg = new Map(rows.map((r) => [r.id, 0]));
  for (const e of deps) indeg.set(e.target, indeg.get(e.target) + 1);
  const sorted = [...rows].sort((a, b) => comparePhase(a.label, b.label));
  const out = [];
  const done = new Set();
  while (out.length < rows.length) {
    const next = sorted.find((r) => !done.has(r.id) && indeg.get(r.id) === 0) ?? sorted.find((r) => !done.has(r.id));
    done.add(next.id);
    out.push(next);
    for (const e of deps) if (e.source === next.id) indeg.set(e.target, indeg.get(e.target) - 1);
  }
  return out;
}

/** Is `to` reachable from `from` without using the direct edge from → to? */
function reachableAvoiding(children, from, to) {
  const stack = children.get(from).filter((k) => k !== to);
  const seen = new Set();
  while (stack.length) {
    const id = stack.pop();
    if (id === to) return true;
    if (seen.has(id)) continue;
    seen.add(id);
    stack.push(...children.get(id));
  }
  return false;
}

function firstFree(cols, skip = -1) {
  const i = cols.findIndex((t, j) => t == null && j !== skip);
  return i >= 0 ? i : Math.max(cols.length, skip + 1);
}

/** One rail row: the node glyph at `c`, `│` for every other open column. */
function railCells(cols, c, glyph = null) {
  return cols.map((t, i) => (i === c ? glyph : t != null ? "│" : " ") + " ").join("");
}

/** Fork (`├─╮`) or join (`├─╯`) between column `c` and `others`; crossings draw `┼`. */
function connector(cols, c, others, kind) {
  const lo = Math.min(c, ...others);
  const hi = Math.max(c, ...others);
  const n = Math.max(cols.length, hi + 1);
  let line = "";
  for (let i = 0; i < n; i++) {
    let ch;
    if (i === c) {
      const left = others.some((o) => o < c);
      const right = others.some((o) => o > c);
      ch = left && right ? "┼" : right ? "├" : "┤";
    } else if (others.includes(i)) {
      ch = kind === "fork" ? (i > c ? "╮" : "╭") : i > c ? "╯" : "╰";
    } else if (i > lo && i < hi) {
      ch = cols[i] != null ? "┼" : "─";
    } else {
      ch = cols[i] != null ? "│" : " ";
    }
    line += ch + (i >= lo && i < hi ? "─" : " ");
  }
  return line;
}

/** Fit row groups into `avail` lines: hide the earliest done phases first (with a summary
 * line), then scroll a window that keeps the selected row visible. */
function fitRows(groups, avail, paint) {
  const count = (gs) => gs.reduce((n, g) => n + g.lines.length, 0);
  if (count(groups) <= avail) return groups.flatMap((g) => g.lines);
  const kept = [...groups];
  let hidden = 0;
  while (count(kept) + 1 > avail) {
    const i = kept.findIndex((g) => g.done && !g.selected);
    if (i < 0) break;
    kept.splice(i, 1);
    hidden++;
  }
  const head = hidden ? [paint(STYLE.dim, `  ${ICONS.done} ${hidden} earlier done phase(s) hidden`)] : [];
  const room = avail - head.length;
  if (count(kept) <= room) return [...head, ...kept.flatMap((g) => g.lines)];

  const budget = room - 2; // "… N above" + "… N below"
  const sel = Math.max(0, kept.findIndex((g) => g.selected));
  let start = sel;
  let end = sel + 1;
  let used = kept[sel].lines.length;
  while (true) {
    if (end < kept.length && used + kept[end].lines.length <= budget) used += kept[end++].lines.length;
    else if (start > 0 && used + kept[start - 1].lines.length <= budget) used += kept[--start].lines.length;
    else break;
  }
  const lines = [...head];
  if (start > 0) lines.push(paint(STYLE.dim, `  … ${start} more above`));
  lines.push(...kept.slice(start, end).flatMap((g) => g.lines));
  if (end < kept.length) lines.push(paint(STYLE.dim, `  … ${kept.length - end} more below`));
  return lines;
}

/** "max review ×2"; an untagged round count falls back to LOOP_REVIEW_ROUNDS_<TIER> in loop-models.conf. */
function reviewText({ tier, rounds }) {
  const conf = readFileSafe(MODELS_CONF).match(new RegExp(`^LOOP_REVIEW_ROUNDS_${tier.toUpperCase()}=(\\d+)`, "m"));
  const fallback = { shallow: 1, medium: 3, max: 3 }[tier];
  return `${tier} review ×${rounds ?? (conf ? Number(conf[1]) : fallback)}`;
}

/** Title as displayed: 📝 marks a phase with a steering note (emoji = 2 cols = 2 UTF-16 units). */
function rowTitle(r) {
  return r.noted ? `📝 ${r.title}` : r.title;
}

/** "engine:model" that really ran. A `--resume` leg records model "resume", so fall back to
 * the last attempt with the same engine that named a real model, else the bare engine. */
/** First leg of the default task chain — what a not-yet-run phase will start on (the
 * light/default route is only decided at launch, so this is labelled "planned"). */
function firstTaskLeg() {
  return readFileSafe(MODELS_CONF).match(/^CHAIN_TASK=\(\s*"([^"]+)"/m)?.[1] ?? null;
}

function actualModel(rt) {
  if (!rt?.engine) return null;
  let model = rt.model;
  if (model === "resume") {
    model = [...(rt.attempts ?? [])].reverse().find((a) => a.engine === rt.engine && a.model && a.model !== "resume")?.model ?? null;
  }
  return model ? `${rt.engine}:${model}` : rt.engine;
}

function stateText(n) {
  const r = n.runtime;
  if (n.ui === "running") {
    const head = (r?.attempt ?? 1) > 1 ? `attempt ${r.attempt}` : "running";
    return r?.lastHeartbeatAgeSec != null ? `${head} · hb ${formatAge(r.lastHeartbeatAgeSec)}` : head;
  }
  if (n.ui === "problem") return r?.problem ?? "problem";
  if (n.paused && n.ui !== "done") return "paused";
  return n.ui;
}

// ---------------------------------------------------------------------------------------
// Terminal + small helpers
// ---------------------------------------------------------------------------------------

function enterTerminal() {
  process.stdout.write(`${ESC}?1049h${ESC}?25l${ESC}2J`);
  if (process.stdin.isTTY) process.stdin.setRawMode(true);
}

function leaveTerminal() {
  if (process.stdin.isTTY) process.stdin.setRawMode(false);
  process.stdout.write(`${ESC}?25h${ESC}?1049l`);
}

function restoreTerminal() {
  if (process.stdout.isTTY) leaveTerminal();
}

function isInside(child, parent) {
  if (!parent) return false;
  const rel = path.relative(parent, child);
  return rel === "" || (!rel.startsWith("..") && !path.isAbsolute(rel));
}

function newest(loops) {
  return [...loops].sort((a, b) => String(b.updatedAt ?? "").localeCompare(String(a.updatedAt ?? "")))[0] ?? null;
}

/** "2a" < "2b" < "10": numeric prefix first, then the suffix. */
function comparePhase(a, b) {
  const [, na = "", sa = ""] = /^(\d*)(.*)$/.exec(String(a));
  const [, nb = "", sb = ""] = /^(\d*)(.*)$/.exec(String(b));
  return (Number(na || Infinity) - Number(nb || Infinity)) || sa.localeCompare(sb);
}

/** 30 → "30s", 480 → "8m", 7800 → "2h10m". */
function formatAge(sec) {
  if (sec < 60) return `${Math.round(sec)}s`;
  const m = Math.floor(sec / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h}h${String(m % 60).padStart(2, "0")}m` : `${Math.floor(h / 24)}d${h % 24}h`;
}

/** POSIX single-quote for a shell word (fzf runs bind commands through $SHELL). */
function shq(s) {
  return `'${String(s).replaceAll("'", `'\\''`)}'`;
}

function truncate(s, n) {
  return s.length <= n ? s : s.slice(0, Math.max(0, n - 1)) + "…";
}

/** Keeps the tail (the informative end of a path): "…/acme--api/lane-a". */
function truncateLeft(s, n) {
  return s.length <= n ? s : "…" + s.slice(s.length - Math.max(0, n - 1));
}

function readFileSafe(file) {
  try {
    return fs.readFileSync(file, "utf8");
  } catch {
    return "";
  }
}

/** Hard-wrap markdown/plain text to `width` columns, keeping blank lines. */
function wrapLines(text, width) {
  const out = [];
  for (const line of String(text).split("\n")) {
    if (line.length <= width) out.push(line);
    else for (let i = 0; i < line.length; i += width) out.push(line.slice(i, i + width));
  }
  return out;
}

function readJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return null;
  }
}

function realpath(p) {
  try {
    return fs.realpathSync(p);
  } catch {
    return p;
  }
}

function isDir(p) {
  try {
    return Boolean(p) && fs.statSync(p).isDirectory();
  } catch {
    return false;
  }
}

function isMain() {
  return Boolean(process.argv[1]) && realpath(process.argv[1]) === realpath(new URL(import.meta.url).pathname);
}
