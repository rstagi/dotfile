#!/usr/bin/env node
// Loop engineering — milestone retrospective facts collector. Zero dependencies, Node >= 20.
// It reads the artifacts of one milestone under a .loop directory and prints facts only.
// It never guesses a cause. An unknown value is null.
//
//   node loop-retro.mjs --dir .loop --milestone <N> [--json]
//
// Exit 0 with the facts on stdout · 1 usage error or an unreadable --dir.
import fs from "node:fs";
import path from "node:path";

const USAGE = "usage: loop-retro.mjs --dir <.loop> --milestone <N> [--json]";
// Setback kinds, in the order of the ship schema enum. Candidates sort by this order.
const KINDS = ["retry", "verify-fail", "timeout", "rate-limit", "model-escalation", "question", "hil",
  "merge-conflict", "review-round", "review-dispute", "replan", "environment", "other"];
// A progress entry is about a phase or a milestone when its subject (after the date) names it.
// The same rule as `loop-plan.sh archive` (plan-format.md → Archived milestones).
const LOG_SUBJECT = /^[-*]\s+(?:\S+\s+[—–-]\s+)?(?:Phase\s+(\d+)\b|Milestone\s+(\d+)\b|M(\d+)\b)/;

main(process.argv.slice(2));

function main(argv) {
  const args = parseArgs(argv);
  const facts = collect(args.dir, args.milestone);
  process.stdout.write(args.json ? `${JSON.stringify(facts, null, 2)}\n` : report(facts));
}

// collect(dir, milestone) -> the facts of one milestone.
function collect(dir, milestone) {
  const state = readJson(path.join(dir, "state.json")) ?? {};
  const events = readEvents(dir);
  const progress = progressEntries(readText(path.join(dir, "progress.md")) ?? "");
  const entries = Object.entries(state.phases ?? {});
  // A state without milestones is one implicit milestone 1.
  const hasMilestones = entries.some(([, entry]) => entry.milestone != null);
  const selected = entries.filter(([, entry]) => (hasMilestones ? entry.milestone === milestone : milestone === 1));
  const keys = new Set(selected.map(([key]) => key));
  const phases = selected
    .map(([key, entry]) => ({
      ...phaseFacts(dir, Number(key), entry, events.filter((e) => String(e.phase) === key)),
      progress: progress.filter((p) => p.phase === key).flatMap((p) => p.lines),
    }))
    .sort((a, b) => a.phase - b.phase);
  const window = milestoneWindow(events, milestone, keys);
  // Loop-level events (no phase) count only inside the milestone window.
  const loopEvents = events.filter((e) => !e.phase && withinWindow(e.ts, window));
  const replans = loopEvents.filter((e) => e.event === "plan.edit");
  const setbackCandidates = [
    ...phases.flatMap((phase) => phaseSetbacks(dir, phase)),
    ...replans.map((e) => ({ phase: null, attempt: null, kind: "replan", fact: `plan.edit at ${e.ts}: ${e.detail ?? "no detail"}`, evidence: ["events.jsonl"] })),
  ].sort(bySetbackOrder);
  const loopCounts = { replans: replans.length, subRecycles: loopEvents.filter((e) => e.event === "sub.recycle").length };
  return {
    milestone,
    window,
    totals: totals(phases, setbackCandidates, window, loopCounts),
    progress: progress.filter((p) => p.milestone === String(milestone)).flatMap((p) => p.lines),
    phases,
    setbackCandidates,
  };
}

// The plain-text form: window, totals, one line per phase (attempt lines for retried phases),
// then the setback candidates with their evidence.
function report(f) {
  const t = f.totals;
  const lines = [
    `Milestone ${f.milestone} · window ${f.window.start ?? "unknown"} -> ${f.window.end ?? "unknown"} (${t.wallTime ?? "unknown"})`,
    `Totals: phases ${t.phases} · attempts ${t.attempts} · retries ${t.retries} · escalations ${t.escalations} · questions ${t.questions}`
      + ` · HIL ${t.hil} · merge conflicts ${t.mergeConflicts} · review rounds ${t.reviewRounds} · fix rounds ${t.fixRounds}`
      + ` · disputed rounds ${t.disputedRounds} · verify failures ${t.verifyFailures} · timeouts ${t.timeouts} · rate limits ${t.rateLimits}`
      + ` · replans ${t.replans} · recycles ${t.subRecycles} · attempt minutes ${t.attemptMinutes} · wall time ${t.wallTime ?? "unknown"}`,
    "",
  ];
  for (const p of f.phases) lines.push(...phaseLines(p));
  lines.push("", `Setback candidates (${f.setbackCandidates.length}):`);
  for (const c of f.setbackCandidates) {
    const where = [c.phase == null ? "loop" : `P${c.phase}`, c.attempt == null ? null : `a${c.attempt}`].filter(Boolean).join(" ");
    lines.push(`  ${where} ${c.kind}: ${c.fact} (${c.evidence.join(", ")})`);
  }
  return `${lines.join("\n")}\n`;
}

function phaseLines(p) {
  const head = `P${p.phase} ${p.slug ?? "?"} · ${p.status ?? "unknown"}`;
  if (p.reviews.length) {
    return [`${head} · review ${p.reviews.map((r) => `${r.repository}: rounds ${r.outerRounds}, fixes ${r.fixRounds}, disputed ${r.disputedRounds}, final ${r.finalVerdict ?? "none"}`).join("; ")}`];
  }
  const minutes = Math.round(p.attempts.reduce((total, a) => total + (a.minutes ?? 0), 0) * 10) / 10;
  const count = `${p.attempts.length} ${p.attempts.length === 1 ? "attempt" : "attempts"}`;
  if (p.attempts.length === 1) return [`${head} · ${count} · ${minutes} min · ${attemptSummary(p.attempts[0])}`];
  return [`${head} · ${count} · ${minutes} min`, ...p.attempts.map((a) => `  a${a.attempt} ${a.chain ?? "-"} ${attemptLine(a)}`)];
}

function attemptLine(a) {
  return `${a.engine ?? "?"} ${a.model ?? "?"} · ${a.outcome ?? "no status"}${a.exit == null ? "" : ` · exit ${a.exit}`} · verify ${a.verify ?? "unknown"} · ${a.minutes ?? "?"} min`;
}

function attemptSummary(a) {
  return `${a.engine ?? "?"} ${a.model ?? "?"}${a.exit == null ? "" : ` · exit ${a.exit}`} · verify ${a.verify ?? "unknown"}`;
}

function parseArgs(argv) {
  const args = { json: false };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag === "--json") args.json = true;
    else if ((flag === "--dir" || flag === "--milestone") && argv[i + 1] !== undefined) args[flag.slice(2)] = argv[++i];
    else usage(`unknown argument ${flag}`);
  }
  if (!args.dir || !/^[1-9][0-9]*$/.test(args.milestone ?? "")) usage("--dir and --milestone <N> are required");
  if (!fs.statSync(args.dir, { throwIfNoEntry: false })?.isDirectory()) usage(`--dir ${args.dir} is not a directory`);
  return { ...args, dir: path.resolve(args.dir), milestone: Number(args.milestone) };
}

function usage(message) {
  process.stderr.write(`loop-retro: ${message}\n${USAGE}\n`);
  process.exit(1);
}

// events.jsonl lines as objects. A line that is not JSON is skipped.
function readEvents(dir) {
  return (readText(path.join(dir, "events.jsonl")) ?? "").split("\n").flatMap((line) => {
    try {
      return line.trim() ? [JSON.parse(line)] : [];
    } catch {
      return [];
    }
  });
}

// progress.md entries about a phase or a milestone: { phase, milestone, lines } with the
// indented continuation lines of each entry.
function progressEntries(text) {
  const entries = [];
  let current = null;
  for (const line of text.split("\n")) {
    const subject = line.match(LOG_SUBJECT);
    if (subject) {
      current = { phase: subject[1] ?? null, milestone: subject[2] ?? subject[3] ?? null, lines: [line] };
      entries.push(current);
    } else if (current && /^\s+\S/.test(line)) {
      current.lines.push(line);
    } else {
      current = null;
    }
  }
  return entries;
}

// The time span of milestone N in events.jsonl.
// Start: the `milestone.resumed` event of MN. Milestone 1 starts at the first event.
// End: the `milestone.awaiting-merge` event of MN, else the last review.finish of its phases,
// else the last event of its phases. A bound without evidence is null.
function milestoneWindow(events, milestone, keys) {
  const timed = events.filter((e) => Number.isFinite(Date.parse(e.ts)));
  const tagged = (name) => timed.filter((e) => e.event === name && new RegExp(`^M${milestone}\\b`).test(String(e.detail ?? "")));
  const own = timed.filter((e) => keys.has(String(e.phase)));
  const start = earliest(tagged("milestone.resumed")) ?? (milestone === 1 ? earliest(timed) : earliest(own));
  const end = earliest(tagged("milestone.awaiting-merge")) ?? latest(own.filter((e) => e.event === "review.finish")) ?? latest(own);
  return { start, end };
}

function earliest(events) {
  return events.reduce((best, e) => (best == null || Date.parse(e.ts) < Date.parse(best) ? e.ts : best), null);
}

function latest(events) {
  return events.reduce((best, e) => (best == null || Date.parse(e.ts) > Date.parse(best) ? e.ts : best), null);
}

function withinWindow(ts, window) {
  const t = Date.parse(ts);
  if (!Number.isFinite(t)) return false;
  return (window.start == null || t >= Date.parse(window.start)) && (window.end == null || t <= Date.parse(window.end));
}

function phaseFacts(dir, phase, entry, events) {
  const spawns = attemptSpawns(events);
  const attempts = entry.slug ? attemptDirs(dir, entry.slug).map((k) => attemptFacts(dir, entry.slug, k, spawns[k] ?? {}, events)) : [];
  const escalateEvents = events.filter((e) => e.event === "phase.escalate").map((e) => ({ ts: e.ts ?? null, detail: e.detail ?? null }));
  return {
    phase,
    slug: entry.slug ?? null,
    kind: entry.kind ?? null,
    status: entry.status ?? null,
    attempts,
    // A chain change to `escalate` or a phase.escalate event. A log without phase.spawn events
    // still counts its phase.escalate events.
    escalations: Math.max(escalatedAttempts(attempts).length, escalateEvents.length),
    escalateEvents,
    hil: hilFacts(dir, entry.slug, events),
    mergeConflicts: events.filter((e) => e.event === "merge.conflict").map((e) => ({ ts: e.ts ?? null, repository: e.repository ?? null, detail: e.detail ?? null })),
    mergeVerifyFailures: mergeVerifyFailures(events),
    reviews: reviewLedgers(dir, phase),
  };
}

// Attempt K -> { chain, ts }: chain `task` or `escalate` from a phase.spawn event, `resume` from a
// phase.resume event.
function attemptSpawns(events) {
  const spawns = {};
  for (const e of events) {
    const spawn = spawnOf(e);
    if (spawn) spawns[spawn.attempt] = { chain: spawn.chain, ts: e.ts ?? null };
  }
  return spawns;
}

// The attempt and the chain that a phase.spawn or phase.resume event starts, or null.
function spawnOf(e) {
  const spawn = e.event === "phase.spawn" && String(e.detail ?? "").match(/\battempt a(\d+) chain (\S+)/);
  if (spawn) return { attempt: Number(spawn[1]), chain: spawn[2] };
  const resume = e.event === "phase.resume" && String(e.detail ?? "").match(/\battempt a(\d+) resume\b/);
  return resume ? { attempt: Number(resume[1]), chain: "resume" } : null;
}

// The merge.verify-fail events of one phase: a merged-tree Verify failure after `loop-merge.sh`
// exit 4. The attempt is the one that the last phase.spawn or phase.resume event before the
// failure (in file order) started, or null.
function mergeVerifyFailures(events) {
  const failures = [];
  let attempt = null;
  for (const e of events) {
    const spawn = spawnOf(e);
    if (spawn) attempt = spawn.attempt;
    else if (e.event === "merge.verify-fail") failures.push({ ts: e.ts ?? null, repository: e.repository ?? null, attempt, detail: e.detail ?? null });
  }
  return failures;
}

// The K of every runs/<slug>-a<K>/ directory, in order.
function attemptDirs(dir, slug) {
  const pattern = new RegExp(`^${escapeRegExp(slug)}-a([1-9][0-9]*)$`);
  return listDir(path.join(dir, "runs"))
    .map((name) => Number(name.match(pattern)?.[1]))
    .filter((k) => k > 0)
    .sort((a, b) => a - b);
}

function attemptFacts(dir, slug, k, { chain = null, ts = null }, events) {
  const runDir = `runs/${slug}-a${k}`;
  const abs = path.join(dir, runDir);
  const meta = readJson(path.join(abs, "meta.json")) ?? {};
  const status = readJson(path.join(abs, "status.json")) ?? {};
  const spawn = readText(path.join(abs, "spawn.log"));
  const exit = runnerExit(spawn);
  const exit40Event = events.some((e) => e.event === "phase.exit40" && new RegExp(`^a${k}\\b`).test(String(e.detail ?? "")));
  return {
    attempt: k,
    runDir,
    chain,
    spawnedAt: ts,
    engine: meta.engine ?? null,
    model: meta.model ?? null,
    modelOverride: meta.modelOverride ?? null,
    outcome: status.outcome ?? null,
    question: status.outcome === "question" && status.question ? status.question : null,
    checkpoint: typeof status.checkpoint === "boolean" ? status.checkpoint : null,
    exit,
    engineExit: Number.isInteger(meta.engineExit) ? meta.engineExit : null,
    verify: verifyResult(abs, spawn),
    timedOut: typeof meta.timedOut === "boolean" ? meta.timedOut : null,
    fallbackReason: meta.routeFallbackReason ?? null,
    rateLimitedLegs: [...(spawn ?? "").matchAll(/rate\/usage limit on (\S+)/g)].map((m) => m[1]),
    exit40Event,
    headBefore: meta.headBefore ?? null,
    headAfter: meta.headAfter ?? null,
    minutes: attemptMinutes(abs),
    files: ["spawn.log", "verify.log", "meta.json", "status.json", "answer.md"].filter((name) => fs.existsSync(path.join(abs, name))),
  };
}

// The runner exit code from the RUNNER_EXIT=<n> line that the spawn command appends, or null.
function runnerExit(spawn) {
  const match = spawn?.match(/^RUNNER_EXIT=(\d+)\s*$/m);
  return match ? Number(match[1]) : null;
}

// "fail" when the runner logged a failed Verify or exit 12. "pass" when verify.log exists and the
// spawn log shows no failure. null when Verify did not run or the evidence is missing.
function verifyResult(abs, spawn) {
  if (spawn != null && (/claimed done but verify failed/.test(spawn) || runnerExit(spawn) === 12)) return "fail";
  if (spawn != null && fs.existsSync(path.join(abs, "verify.log"))) return "pass";
  return null;
}

// Minutes from prompt.md to the newest result file, one decimal, or null.
function attemptMinutes(abs) {
  const start = mtime(path.join(abs, "prompt.md"));
  const ends = ["meta.json", "status.json", "verify.log", "spawn.log"].map((name) => mtime(path.join(abs, name))).filter((t) => t != null);
  if (start == null || ends.length === 0) return null;
  return Math.round((Math.max(...ends) - start) / 6000) / 10;
}

// Attempts that moved to the escalate chain from another chain.
function escalatedAttempts(attempts) {
  return attempts.filter((a, i) => a.chain === "escalate" && attempts[i - 1]?.chain !== "escalate");
}

// HIL briefs and answers of one phase, with its hil.raise and hil.resolve events.
function hilFacts(dir, slug, events) {
  const pick = (name) => events.filter((e) => e.event === name).map((e) => ({ ts: e.ts ?? null, detail: e.detail ?? null }));
  const raised = pick("hil.raise");
  const files = slug ? [`hil/${slug}.md`, `hil/${slug}.answer.md`].filter((rel) => fs.existsSync(path.join(dir, rel))) : [];
  const briefExists = files.includes(`hil/${slug}.md`);
  return { count: Math.max(raised.length, briefExists ? 1 : 0), raised, resolved: pick("hil.resolve"), files };
}

// One summary for each runs/review-p<N>-<owner--repo>.ledger.tsv of review phase N.
// Ledger rows: a<k>, stage, role, provider, verdict, run dir (loop-protocol.md → Ledger and CLI).
function reviewLedgers(dir, phase) {
  const pattern = new RegExp(`^review-p${phase}-(.+)\\.ledger\\.tsv$`);
  return listDir(path.join(dir, "runs"))
    .filter((name) => pattern.test(name))
    .sort()
    .map((name) => {
      const ledger = `runs/${name}`;
      const rows = (readText(path.join(dir, ledger)) ?? "").split("\n").filter(Boolean).map((line) => line.split("\t"));
      const stages = rows.map(([, stage]) => stage);
      // A dispute row follows the last exchange of its round.
      const disputed = rows.flatMap(([, , role], i) => (role === "disputed" ? [rows[i - 1]?.[1] ?? null] : []));
      const disputedFile = ledger.replace(/\.tsv$/, ".disputed.md");
      return {
        repository: name.match(pattern)[1].replace("--", "/"),
        ledger,
        runs: rows.length,
        outerRounds: new Set(stages.filter((stage) => /^round\d+$/.test(stage))).size,
        fixRounds: rows.filter(([, , role]) => role === "fix").length,
        fixes: rows.filter(([, , role]) => role === "fix").map(([, stage, , , verdict, runDir]) => ({ stage, verdict, runDir })),
        disputedRounds: disputed.length,
        disputed,
        finalVerdict: rows.find(([, , role]) => role === "final")?.[4] ?? null,
        disputedFile: fs.existsSync(path.join(dir, disputedFile)) ? disputedFile : null,
      };
    });
}

// The setback candidates of one phase: one entry for each process cost, with its evidence paths.
function phaseSetbacks(dir, p) {
  const out = [];
  const add = (kind, attempt, fact, evidence, extra = {}) => out.push({ phase: p.phase, attempt, kind, ...extra, fact, evidence });
  if (p.attempts.length > 1) add("retry", null, `${p.attempts.length} attempts`, p.attempts.map((a) => a.runDir));
  for (let i = 0; i < p.hil.count; i += 1) {
    const [raise, resolve] = [p.hil.raised[i], p.hil.resolved[i]];
    const fact = [raise ? `HIL raised at ${raise.ts}: ${raise.detail ?? "no detail"}` : `HIL brief ${p.hil.files[0]}`,
      resolve ? `Resolved at ${resolve.ts}: ${resolve.detail ?? "no detail"}` : null].filter(Boolean).join(". ");
    add("hil", null, fact, [...p.hil.files, ...(p.hil.raised.length ? ["events.jsonl"] : [])]);
  }
  for (const c of p.mergeConflicts) add("merge-conflict", null, `merge conflict${inRepository(c)}: ${c.detail ?? "no detail"}`, ["events.jsonl"]);
  for (const r of p.reviews) {
    for (const fix of r.fixes) add("review-round", null, `${r.repository} ${fix.stage}: ${fix.verdict}`, [r.ledger, fix.runDir]);
    for (const round of r.disputed) {
      add("review-dispute", null, `${r.repository} ${round} reached the inner cap without agreement`, [r.ledger, ...(r.disputedFile ? [r.disputedFile] : [])]);
    }
  }
  const escalated = new Set(escalatedAttempts(p.attempts).map((a) => a.attempt));
  for (let i = escalated.size; i < p.escalations; i += 1) add("model-escalation", null, "phase.escalate event without a chain change", ["events.jsonl"]);
  // A runner Verify failure and the first merged-tree failure of the same attempt are one
  // failure: one candidate. Each other merged-tree failure is a candidate of its own.
  const mergedTree = [...p.mergeVerifyFailures];
  p.attempts.forEach((a, i) => {
    const file = (name) => (a.files.includes(name) ? [`${a.runDir}/${name}`] : []);
    if (a.verify === "fail") {
      const index = mergedTree.findIndex((f) => f.attempt === a.attempt);
      const same = index === -1 ? null : mergedTree.splice(index, 1)[0];
      add("verify-fail", a.attempt, `a${a.attempt} claimed done, then Verify failed`
        + (same ? `. The merged-tree Verify failed too${inRepository(same)}: ${same.detail ?? "no detail"}` : ""),
      [...file("spawn.log"), ...file("verify.log"), ...(same ? ["events.jsonl"] : [])], { stage: "runner" });
    }
    if (a.timedOut || a.exit === 124 || a.engineExit === 124) add("timeout", a.attempt, `a${a.attempt} timed out`, [...file("spawn.log"), ...file("meta.json")]);
    if (a.rateLimitedLegs.length || a.exit === 40 || a.engineExit === 40 || a.exit40Event) {
      const legs = a.rateLimitedLegs.length ? `usage limit on ${a.rateLimitedLegs.join(", ")}` : "usage limit";
      add("rate-limit", a.attempt, `a${a.attempt} ${legs}${a.exit === 40 || a.engineExit === 40 ? ", model chain exhausted (exit 40)" : ""}`,
        [...file("spawn.log"), ...(a.engineExit === 40 ? file("meta.json") : []), ...(a.exit40Event ? ["events.jsonl"] : [])]);
    }
    if (escalated.has(a.attempt)) {
      const before = p.attempts[i - 1];
      // The phase.escalate event logged after the previous spawn and up to this spawn.
      const logged = p.escalateEvents.filter((e) => (!before?.spawnedAt || Date.parse(e.ts) > Date.parse(before.spawnedAt))
        && (!a.spawnedAt || Date.parse(e.ts) <= Date.parse(a.spawnedAt))).at(-1);
      add("model-escalation", a.attempt, `a${a.attempt} moved to the escalate chain (${engineModel(before)} -> ${engineModel(a)})`
        + (logged ? `. phase.escalate: ${logged.detail ?? "no detail"}` : ""), ["events.jsonl", ...file("meta.json")]);
    }
    if (a.outcome === "question") {
      add("question", a.attempt, `a${a.attempt} asked a question${a.checkpoint ? " at the phase checkpoint" : ""}`,
        [...file("status.json"), ...file("answer.md"), ...answerFiles(dir, p.phase, a.attempt)]);
    }
    if (a.outcome === "blocked" || a.exit === 50) add("other", a.attempt, `a${a.attempt} ${a.exit === 50 ? "crashed (exit 50)" : "blocked"}`, file("status.json").length ? file("status.json") : file("spawn.log"));
  });
  for (const f of mergedTree) {
    add("verify-fail", f.attempt, `${f.attempt == null ? "the" : `a${f.attempt} merged, then the`} merged-tree Verify failed${inRepository(f)}: ${f.detail ?? "no detail"}`,
      ["events.jsonl"], { stage: "merged-tree" });
  }
  return out;
}

function inRepository({ repository }) {
  return repository ? ` (${repository})` : "";
}

function engineModel(a) {
  return a ? `${a.engine ?? "?"} ${a.model ?? "?"}` : "none";
}

// answers/p<N>-a<K>-*.md: orchestrator answers that the loop keeps outside the run directory.
function answerFiles(dir, phase, k) {
  return listDir(path.join(dir, "answers")).filter((name) => name.startsWith(`p${phase}-a${k}-`)).sort().map((name) => `answers/${name}`);
}

function bySetbackOrder(a, b) {
  return (a.phase ?? Infinity) - (b.phase ?? Infinity) || (a.attempt ?? 0) - (b.attempt ?? 0) || KINDS.indexOf(a.kind) - KINDS.indexOf(b.kind);
}

function totals(phases, candidates, window, loopCounts) {
  const attempts = phases.flatMap((p) => p.attempts);
  const reviews = phases.flatMap((p) => p.reviews);
  const count = (kind) => candidates.filter((c) => c.kind === kind).length;
  const add = (list, value) => list.reduce((total, item) => total + value(item), 0);
  const wallMinutes = window.start && window.end ? Math.round((Date.parse(window.end) - Date.parse(window.start)) / 60_000) : null;
  return {
    phases: phases.length,
    attempts: attempts.length,
    retries: add(phases, (p) => Math.max(0, p.attempts.length - 1)),
    escalations: add(phases, (p) => p.escalations),
    questions: attempts.filter((a) => a.outcome === "question").length,
    hil: add(phases, (p) => p.hil.count),
    mergeConflicts: add(phases, (p) => p.mergeConflicts.length),
    reviewRounds: add(reviews, (r) => r.outerRounds),
    fixRounds: add(reviews, (r) => r.fixRounds),
    disputedRounds: add(reviews, (r) => r.disputedRounds),
    verifyFailures: count("verify-fail"),
    timeouts: count("timeout"),
    rateLimits: count("rate-limit"),
    ...loopCounts,
    attemptMinutes: Math.round(add(attempts, (a) => a.minutes ?? 0)),
    wallMinutes,
    wallTime: wallMinutes == null ? null : duration(wallMinutes),
  };
}

function duration(minutes) {
  const hours = Math.floor(minutes / 60);
  return hours ? `${hours}h ${minutes % 60}m` : `${minutes}m`;
}

function mtime(file) {
  return fs.statSync(file, { throwIfNoEntry: false })?.mtimeMs ?? null;
}

function readJson(file) {
  const text = readText(file);
  if (text == null) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function readText(file) {
  try {
    return fs.readFileSync(file, "utf8");
  } catch {
    return null;
  }
}

function listDir(dir) {
  try {
    return fs.readdirSync(dir);
  } catch {
    return [];
  }
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
