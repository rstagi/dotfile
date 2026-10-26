// Run graph facts: reads one milestone of a Loop dir into the engine-neutral facts of
// run-model.mjs. Read-only: it never writes to the Loop dir.
//
//   readRunFacts(loopDir, milestone, { wsLog, outDir }) -> { facts, engine, warnings }
//
// The phases come from milestones/m<N>.md when the milestone is archived, else from plan.md
// (loop-plan-parse.mjs). What happened comes from one of two engines:
//   skills  the saved `ws log <loop> --type loop.` output: --ws-log, or
//           explainers/m<N>-ship/loop-log.json. Times come from `ts_ms`.
//   old     events.jsonl, the retro facts (explainers/m<N>-ship/retro-facts.json, else
//           loop-retro.mjs), state.json, the review ledger, converge/p<N>.md and git commit times.
// Both engines share the checkpoints (explainers/m<N>-cp<k>/) and the ship props.
// Without either source, the facts have no events and the page has no timeline.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const ROOT = path.join(import.meta.dirname, "..", "..", "..", "..");
const PARSER = path.join(ROOT, "loop-plan-parse.mjs");
const RETRO = path.join(ROOT, "loop-retro.mjs");
const SHA = /^[0-9a-f]{7,40}$/;

export function readRunFacts(loop, m, { wsLog, outDir } = {}) {
  const warnings = [];
  const plan = readPlan(loop, m);
  const phases = plan.phases.filter((p) => p.milestone === m).map((p) => ({
    n: String(p.phase), title: p.title, lane: p.lane ?? "main", kind: p.kind, dependsOn: (p.dependsOn ?? []).map(String), doneWhen: p.doneWhen,
  }));
  if (!phases.length) throw new RunFactsError(`milestone ${m} has no phases in ${plan.file}`);
  const shipDir = path.join(loop, "explainers", `m${m}-ship`);
  const ship = readShip(shipDir, outDir);
  const milestone = plan.milestones?.find((x) => x.number === m) ?? {};
  const planPr = Object.values(milestone.prs ?? {}).find((u) => /^https?:\/\//.test(u ?? ""));
  const base = {
    milestone: m, title: milestone.title ?? "", phases, start: null, end: null, events: [], attempts: {},
    review: { phase: phases.find((p) => p.kind === "pr-review")?.n ?? null, rows: [], convergeRows: [], converge: null },
    gate: { ...ship, prUrl: ship.prUrl ?? planPr ?? null },
    checkpoints: readCheckpoints(loop, m),
  };
  const logFile = wsLog ?? path.join(shipDir, "loop-log.json");
  let engine = "none";
  let found = {};
  if (fs.existsSync(logFile)) {
    engine = "skills";
    found = fromWsLog(readWsLog(logFile), m, phases, base.review.phase);
  } else if (fs.existsSync(path.join(loop, "events.jsonl"))) {
    engine = "old";
    found = fromOldEngine(loop, m, phases, base.review.phase, shipDir);
  }
  const facts = {
    ...base, ...found,
    review: { ...base.review, ...found.review },
    gate: { ...base.gate, prUrl: base.gate.prUrl ?? found.prUrl ?? null, reviewUrl: base.gate.reviewUrl ?? found.review?.reviewUrl ?? null },
  };
  facts.repo = found.repo ?? repoOf(facts.gate.prUrl);
  delete facts.prUrl;
  if (!facts.events.some((e) => e.at) && !facts.review.startAt) warnings.push(`no timestamps for milestone ${m} (engine: ${engine}): the page has no timeline`);
  return { facts, engine, warnings };
}

export class RunFactsError extends Error {}

// ---- the skills engine: loop.* events of the loop session ----

function fromWsLog(entries, m, phases, gateN) {
  const work = new Set(phases.filter((p) => p.kind !== "pr-review").map((p) => p.n));
  const events = [];
  const attempts = {};
  const open = new Map();
  const exits = new Map();
  const rows = [];
  const review = { rows };
  let start = null;
  let end = null;
  let prUrl = null;
  let repo = null;
  for (const { at, type, body } of entries) {
    const n = String(body.n ?? body.phase ?? "");
    if (type === "loop.milestone" && body.milestone === m) start ??= at;
    else if (type === "loop.attempt" && work.has(n)) {
      const key = `${n}:${body.attempt}`;
      if (body.state === "started") {
        open.set(key, at);
        events.push({ at, phase: n, status: "active", kind: "run", text: `started a${body.attempt}` });
      } else if (body.state === "exited") {
        exits.set(key, { at, exit: body.exit, n, attempt: body.attempt });
      } else if (body.state === "result") {
        exits.delete(key);
        addAttempt(attempts, n, body.attempt, open.get(key), at, body.verify);
        const line = resultLine(body);
        if (line) events.push({ at, phase: n, ...line });
      }
    } else if (type === "loop.merge" && work.has(n)) {
      repo ??= body.repo ?? null;
      if (body.verify === "fail") events.push({ at, phase: n, status: "failed", kind: "setback", text: `merged-tree Verify failed at \`${body.head}\``, short: "merged-tree Verify" });
      else events.push({ at, phase: n, status: "done", kind: "merge", text: `merged as \`${body.head}\``, sha: SHA.test(body.head ?? "") ? body.head : undefined });
    } else if (type === "loop.review" && n === gateN) {
      review.startAt ??= at;
      if (body.state === "finished") {
        rows.push({ stage: body.role === "final" ? "final" : `round${body.round}`, role: body.role, provider: body.provider, verdict: body.verdict });
        if (body.role === "final") Object.assign(review, { endAt: at, finalVerdict: body.verdict, reviewUrl: body.commentUrl ?? null });
      }
    } else if (type === "loop.gate" && body.kind === "merge" && body.milestone === m && body.state === "open") {
      end ??= at;
      prUrl ??= Object.values(body.prs ?? {}).find((u) => typeof u === "string") ?? null;
    }
  }
  for (const x of exits.values()) {
    addAttempt(attempts, x.n, x.attempt, open.get(`${x.n}:${x.attempt}`), x.at, null);
    if (x.exit !== 0) events.push({ at: x.at, phase: x.n, status: "failed", kind: "setback", text: `a${x.attempt} exited ${x.exit}`, short: `exit ${x.exit}` });
  }
  const own = events.map((e) => e.at).sort();
  return { start: start ?? own[0] ?? review.startAt ?? null, end: end ?? review.endAt ?? own.at(-1) ?? null, events, attempts, review, prUrl, repo };
}

function resultLine(body) {
  if (body.verify === "fail") return { status: "failed", kind: "setback", text: `a${body.attempt} Verify failed (${body.outcome})`, short: "Verify failed" };
  if (body.outcome === "question") return { status: "review", kind: "setback", text: `a${body.attempt} asked a question`, short: "question" };
  if (body.outcome !== "done") return { status: "failed", kind: "setback", text: `a${body.attempt} ${body.outcome}`, short: body.outcome };
  return null;
}

function addAttempt(attempts, n, attempt, startAt, endAt, verify) {
  const minutes = startAt ? Math.round((Date.parse(endAt) - Date.parse(startAt)) / 6000) / 10 : null;
  (attempts[n] ??= []).push({ attempt, minutes, verify: verify ?? null });
}

// `ws log --json` prints { session_id, entries }. Also accept a bare array or JSON lines.
function readWsLog(file) {
  const text = fs.readFileSync(file, "utf8");
  let list;
  try {
    const raw = JSON.parse(text);
    list = Array.isArray(raw) ? raw : raw.entries ?? [];
  } catch {
    list = text.split("\n").flatMap((l) => { try { return l.trim() ? [JSON.parse(l)] : []; } catch { return []; } });
  }
  return list.filter((e) => e && typeof e.type === "string" && e.body && Number.isFinite(e.ts_ms))
    .map((e) => ({ at: isoOf(e.ts_ms), type: e.type, body: e.body }))
    .sort((a, b) => a.at.localeCompare(b.at));
}

// ---- the old engine: events.jsonl and the run directories ----

function fromOldEngine(loop, m, phases, gateN, shipDir) {
  const nums = new Set(phases.map((p) => p.n));
  const state = readJson(path.join(loop, "state.json")) ?? {};
  const all = readText(path.join(loop, "events.jsonl")).split("\n").flatMap((l) => { try { return l.trim() ? [JSON.parse(l)] : []; } catch { return []; } });
  const retro = readJson(path.join(shipDir, "retro-facts.json")) ?? runRetro(loop, m) ?? { phases: [] };
  const tagged = (name) => all.find((e) => e.event === name && new RegExp(`^M${m}\\b`).test(String(e.detail ?? "")))?.ts;
  const start = retro.window?.start ?? tagged("milestone.resumed") ?? all.find((e) => nums.has(String(e.phase)))?.ts ?? null;
  const own = all.filter((e) => nums.has(String(e.phase)) && (!start || e.ts >= start));
  const runs = new Map((retro.phases ?? []).map((r) => [String(r.phase), r]));
  const events = [];
  const attempts = {};
  for (const e of own) {
    if (String(e.phase) === gateN) continue;
    const line = eventLine(e);
    if (line) events.push({ at: e.ts, phase: String(e.phase), ...line });
  }
  const repos = repoDirs(loop, state);
  for (const p of phases.filter((x) => x.kind !== "pr-review")) {
    const r = runs.get(p.n);
    attempts[p.n] = (r?.attempts ?? []).map((a) => ({ attempt: a.attempt, model: a.model ?? a.engine ?? null, minutes: a.minutes ?? null, verify: a.verify ?? a.outcome ?? null }));
    const merged = mergeOf(p.n, r, state.phases?.[p.n], repos);
    if (merged) events.push({ at: merged.at, phase: p.n, status: "done", kind: "merge", text: merged.sha ? `merged as \`${merged.sha}\`` : "merged", sha: merged.sha });
  }
  const ledger = ledgerRows(loop, gateN);
  const final = ledger.find((l) => l.role === "final");
  const gateEvents = own.filter((e) => String(e.phase) === gateN);
  const convergeRun = runStart(loop, `runs/converge-p${gateN}-a1`);
  const review = {
    rows: ledger.map(({ stage, role, provider, verdict }) => ({ stage, role, provider, verdict })),
    startAt: convergeRun ?? gateEvents.find((e) => e.event === "review.start")?.ts ?? ledger[0]?.start ?? null,
    endAt: gateEvents.filter((e) => e.event === "review.finish").at(-1)?.ts ?? final?.end ?? null,
    finalVerdict: final?.verdict ?? null,
    converge: state.converge?.[gateN] ? { status: state.converge[gateN].status, reason: state.converge[gateN].reason } : null,
    convergeRows: convergeRows(readText(path.join(loop, "converge", `p${gateN}.md`))),
  };
  return { start, end: retro.window?.end ?? tagged("milestone.awaiting-merge") ?? null, events, attempts, review };
}

// One events.jsonl entry -> a status and a line of text, or null for noise.
function eventLine(e) {
  const d = oneLine(e.detail ?? "");
  const short = clip(d.replace(/^a\d+ exit \d+: /, ""), 90);
  switch (e.event) {
    case "phase.spawn": return { status: "active", kind: "run", text: `started ${d.match(/attempt a\d+/)?.[0] ?? "an attempt"}` };
    case "phase.resume": return { status: "active", kind: "run", text: `resumed as ${d.match(/attempt a\d+/)?.[0] ?? "a new attempt"}` };
    case "phase.verify-fail": return { status: "failed", kind: "setback", text: `Verify failed: ${clip(d, 200)}`, short };
    case "merge.verify-fail": return { status: "failed", kind: "setback", text: `merged-tree Verify failed: ${clip(d, 200)}`, short: `merge ${short}` };
    case "merge.conflict": return { status: "failed", kind: "setback", text: `merge conflict: ${clip(d, 200)}`, short: "merge conflict" };
    case "phase.question": return { status: "review", kind: "setback", text: `question: ${clip(d, 200)}`, short: "question" };
    case "phase.answer": return { status: "active", kind: "run", text: `answered: ${clip(d, 160)}` };
    case "phase.escalate": return { status: "active", kind: "setback", text: `escalated: ${clip(d, 160)}`, short: "escalated" };
    case "hil.raise": return { status: "review", kind: "setback", text: `asked the human: ${clip(d, 160)}`, short: "HIL" };
    default: return null;
  }
}

// The merge of a done phase: the "Merged at <sha>" progress line or the state sha, at its git
// commit time. Without a commit time, the end of the last attempt stands in.
function mergeOf(n, retroPhase, statePhase, repos) {
  const sha = (retroPhase?.progress ?? []).join(" ").match(/Merged at ([0-9a-f]{7,40})/)?.[1]
    ?? (SHA.test(statePhase?.mergedAt ?? "") ? statePhase.mergedAt : null);
  if (!sha && statePhase?.status !== "done") return null;
  const at = (sha && gitTime(sha, repos)) ?? lastAttemptEnd(retroPhase);
  return at ? { at, sha: sha ?? undefined } : null;
}

function lastAttemptEnd(retroPhase) {
  const a = (retroPhase?.attempts ?? []).filter((x) => x.spawnedAt).at(-1);
  return a ? isoOf(Date.parse(a.spawnedAt) + (a.minutes ?? 0) * 60_000) : null;
}

function gitTime(sha, repos) {
  for (const dir of repos) {
    try {
      const out = execFileSync("git", ["-C", dir, "show", "-s", "--format=%cI", sha], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
      if (out) return isoOf(Date.parse(out));
    } catch {
      // Not in this checkout: try the next one.
    }
  }
  return null;
}

function repoDirs(loop, state) {
  const dirs = [state.integrationWorktree, ...Object.values(state.repositories ?? {}).map((r) => r?.integrationWorktree), state.runtimeRoot, path.dirname(loop)];
  return [...new Set(dirs.filter((d) => typeof d === "string" && fs.existsSync(d)))];
}

// runs/review-p<N>-<owner--repo>.ledger.tsv rows: a<k>, stage, role, provider, verdict, run dir.
function ledgerRows(loop, gateN) {
  if (!gateN) return [];
  const pattern = new RegExp(`^review-p${gateN}-.+\\.ledger\\.tsv$`);
  return listDir(path.join(loop, "runs")).filter((f) => pattern.test(f)).sort().flatMap((f) => readText(path.join(loop, "runs", f)).split("\n").filter(Boolean).map((l) => {
    const [, stage, role, provider, verdict, dir] = l.split("\t");
    return { stage, role, provider, verdict, start: dir ? runStart(loop, dir) : null, end: dir ? mtime(path.join(loop, dir, "meta.json")) : null };
  }));
}

// The converge table: "| P137.6 text | 137 | partial | evidence |".
function convergeRows(md) {
  return md.split("\n").map((l) => l.match(/^\|\s*(P\d+\.\d+)\s+(.+?)\s*\|\s*(\d+)\s*\|\s*(\w+)\s*\|/)).filter(Boolean)
    .map(([, , criterion, phase, state]) => ({ phase, state, criterion: clip(criterion, 90) }));
}

function runRetro(loop, m) {
  try {
    return JSON.parse(execFileSync(process.execPath, [RETRO, "--dir", loop, "--milestone", String(m), "--json"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }));
  } catch {
    return null;
  }
}

// ---- shared: plan, ship props, checkpoints ----

function readPlan(loop, m) {
  const archive = path.join(loop, "milestones", `m${m}.md`);
  const current = path.join(loop, "plan.md");
  for (const file of [archive, current]) {
    if (!fs.existsSync(file)) continue;
    const plan = parsePlan(file);
    if (plan.phases.some((p) => p.milestone === m)) return { ...plan, file };
  }
  return { phases: [], milestones: [], file: fs.existsSync(archive) ? archive : current };
}

function parsePlan(file) {
  try {
    return JSON.parse(execFileSync(process.execPath, [PARSER, "--plan", file], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }));
  } catch (err) {
    throw new RunFactsError(`cannot parse ${file}: ${oneLine(err.stderr || err.message)}`);
  }
}

function readShip(shipDir, outDir) {
  const props = readJson(path.join(shipDir, "props.json")) ?? {};
  const review = props.reviews?.[0] ?? {};
  const link = firstLine(readText(path.join(shipDir, "link.txt")));
  const sameDir = outDir && path.resolve(outDir) === path.resolve(shipDir) && fs.existsSync(path.join(shipDir, "index.html"));
  return {
    prUrl: review.pr ?? null,
    reviewUrl: review.url ?? null,
    summary: review.summary ?? null,
    shipLink: link || (sameDir ? "index.html" : null),
    deviations: (props.deviations ?? []).map((d) => d.actual ?? d.planned).filter(Boolean),
    handTest: (props.handTest ?? []).map((h) => ({ title: h.title ?? "", command: h.command ?? "" })),
    rollback: (props.rollback ?? []).map((r) => r.step ?? r).filter((r) => typeof r === "string"),
  };
}

// explainers/m<N>-cp<k>/ for k = 1, 2, … The time: the checkpoint runner start, else the
// checkpoints.json reservation, else the time of its props.
function readCheckpoints(loop, m) {
  const reserved = readJson(path.join(loop, "explainers", "checkpoints.json")) ?? {};
  const out = [];
  for (let k = 1; fs.existsSync(path.join(loop, "explainers", `m${m}-cp${k}`)); k += 1) {
    const dir = path.join(loop, "explainers", `m${m}-cp${k}`);
    const cp = readJson(path.join(dir, "props.json"))?.meta?.checkpoint ?? {};
    const last = reserved.milestone === m && reserved.last?.number === k ? reserved.last.at : null;
    out.push({
      k,
      at: runStart(loop, `runs/explainer-m${m}-cp${k}-a1`) ?? last ?? mtime(path.join(dir, "props.json")),
      href: firstLine(readText(path.join(dir, "link.txt"))) || null,
      done: cp.phasesDone ?? null,
      total: cp.phasesTotal ?? null,
    });
  }
  return out;
}

// ---- helpers ----

function runStart(loop, dir) {
  const at = readJson(path.join(loop, dir, "meta.json"))?.startedAt ?? readJson(path.join(loop, dir, "leg.json"))?.startedAt;
  return at && Number.isFinite(Date.parse(at)) ? isoOf(Date.parse(at)) : null;
}

function repoOf(url) {
  return String(url ?? "").match(/github\.com\/([^/]+\/[^/#]+)\/pull\//)?.[1] ?? null;
}

function isoOf(ms) { return new Date(ms).toISOString().replace(/\.\d{3}Z$/, "Z"); }
function mtime(file) { try { return isoOf(fs.statSync(file).mtimeMs); } catch { return null; } }
function readJson(file) { try { return JSON.parse(fs.readFileSync(file, "utf8")); } catch { return null; } }
function readText(file) { try { return fs.readFileSync(file, "utf8"); } catch { return ""; } }
function listDir(dir) { try { return fs.readdirSync(dir); } catch { return []; } }
function firstLine(text) { return text.split("\n")[0].trim(); }
function oneLine(s) { return String(s ?? "").replace(/\s+/g, " ").trim(); }
function clip(s, n) { s = String(s ?? ""); return s.length <= n ? s : `${s.slice(0, n - 1).trimEnd()}…`; }
