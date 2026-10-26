import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SKILL = path.join(import.meta.dirname, "..", ".claude", "skills", "visual-explainer");
const RUN_GRAPH = path.join(SKILL, "scripts", "run-graph.mjs");
const EXPLORE = path.join(SKILL, "scripts", "explore.mjs");
const TEMPLATE = fs.readFileSync(path.join(SKILL, "template", "explore.html"), "utf8");
const { parseTimeline } = await import(path.join(SKILL, "scripts", "timeline.mjs"));
const { buildRun } = await import(path.join(SKILL, "scripts", "run-model.mjs"));
const { parseExplore } = await import(path.join(SKILL, "scripts", "explore-spec.mjs"));
const { renderExplore } = await import(EXPLORE);
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "visual-explainer-run-"));
process.on("exit", () => fs.rmSync(TMP, { recursive: true, force: true }));

const T = (hhmm) => `2026-10-26T${hhmm}:00Z`;
const run = (...args) => spawnSync(process.execPath, [RUN_GRAPH, ...args], { encoding: "utf8" });
const timelineOf = (html) => JSON.parse(html.match(/<script type="application\/json" id="timeline">([^<]*)<\/script>/)[1]);

// One milestone in the normalized form that both engine adapters produce.
function facts(over = {}) {
  return {
    milestone: 2,
    title: "Ship the demo",
    repo: "acme/demo",
    phases: [
      { n: "11", title: "Add the parser", lane: "A", kind: "work", dependsOn: ["3"], doneWhen: "the parser reads a file." },
      { n: "12", title: "Add the printer", lane: "A", kind: "work", dependsOn: ["11"], doneWhen: "the printer prints." },
      { n: "13", title: "Add the CLI", lane: "B", kind: "work", dependsOn: ["11"], doneWhen: "the CLI runs." },
      { n: "14", title: "Review milestone 2", lane: "review", kind: "pr-review", dependsOn: ["12", "13"], doneWhen: "a verdict." },
    ],
    start: T("10:00"),
    end: T("13:00"),
    events: [
      { at: T("10:01"), phase: "11", status: "active", kind: "run", text: "started a1" },
      { at: T("10:30"), phase: "11", status: "done", kind: "merge", text: "merged as `aaa1111`", sha: "aaa1111" },
      { at: T("10:31"), phase: "12", status: "active", kind: "run", text: "started a1" },
      { at: T("10:31"), phase: "13", status: "active", kind: "run", text: "started a1" },
      // The same second: the failure comes before the retry that it caused.
      { at: T("10:50"), phase: "12", status: "active", kind: "run", text: "started a2" },
      { at: T("10:50"), phase: "12", status: "failed", kind: "setback", text: "Verify failed: test x", short: "test x" },
      { at: T("11:20"), phase: "12", status: "done", kind: "merge", text: "merged as `bbb2222`", sha: "bbb2222" },
      { at: T("11:40"), phase: "13", status: "done", kind: "merge", text: "merged" },
    ],
    attempts: { 11: [{ attempt: 1, model: "m1", minutes: 29, verify: "pass" }], 12: [{ attempt: 1, minutes: 19, verify: "fail" }, { attempt: 2, minutes: 30, verify: "pass" }] },
    review: {
      phase: "14", startAt: T("12:00"), endAt: T("12:50"), finalVerdict: "APPROVE",
      rows: [{ stage: "round1", role: "review", provider: "gpt", verdict: "APPROVE" }, { stage: "final", role: "final", provider: "claude", verdict: "APPROVE" }],
      converge: { status: "converged", reason: "all met" }, convergeRows: [{ phase: "12", state: "partial", criterion: "prints" }],
    },
    gate: { prUrl: "https://github.com/acme/demo/pull/7", reviewUrl: "https://github.com/acme/demo/pull/7#r1", shipLink: "index.html", summary: "Verdict: APPROVE.", deviations: [], handTest: [{ title: "Run it", command: "demo run" }], rollback: ["git revert"] },
    checkpoints: [{ k: 1, at: T("11:00"), href: "https://example.test/cp1", done: 1, total: 3 }],
    ...over,
  };
}

test("the run graph puts each phase in its lane and the gate at the end", () => {
  const { graph } = buildRun(facts(), {});
  const ids = graph.nodes.map((n) => n.id);
  assert.deepEqual(ids, ["m", "lane:A", "lane:B", "p:11", "p:12", "p:13", "gate"]);
  assert.equal(graph.nodes.find((n) => n.id === "p:12").group, "lane:A");
  assert.equal(graph.nodes.find((n) => n.id === "p:12").tag, "#12 ⚠1");
  assert.match(graph.nodes.find((n) => n.id === "p:12").details, /Verify failed: test x/);
  assert.match(graph.nodes.find((n) => n.id === "p:12").details, /\*\*Converge:\*\* partial — prints/);
  assert.deepEqual(graph.nodes.find((n) => n.id === "p:11").links, [{ label: "Commit aaa1111", href: "https://github.com/acme/demo/commit/aaa1111" }]);
  const gate = graph.nodes.find((n) => n.id === "gate");
  assert.match(gate.details, /round1 review \(gpt\): APPROVE/);
  assert.deepEqual(gate.links.map((l) => l.label), ["Pull request", "Final review", "Ship report page", "Checkpoint 1 page"]);
  // Phase 3 is in another milestone: no edge. Lane A -> lane B is a cross-lane wait.
  assert.deepEqual(graph.edges.map((e) => `${e.from}>${e.to}:${e.kind}`), ["p:11>p:12:flow", "p:11>p:13:depends", "p:12>gate:flow", "p:13>gate:flow"]);
  assert.equal(parseExplore(JSON.stringify(graph)).warnings.length, 0);
});

test("the timeline orders same-second statuses, marks each checkpoint and aligns the steps", () => {
  const { graph, timeline } = buildRun(facts(), {});
  assert.deepEqual(timeline.nodes["p:12"].map((x) => x.status), ["active", "failed", "active", "done"]);
  assert.deepEqual(timeline.nodes.gate, [{ at: T("12:00"), status: "review" }, { at: T("12:50"), status: "done" }]);
  assert.deepEqual(timeline.marks.map((m) => m.label), ["Start", "CP1", "Review", "Shipped"]);
  assert.equal(timeline.marks[1].href, "https://example.test/cp1");
  assert.match(timeline.marks[1].text, /1 of 3 phases merged/);
  assert.deepEqual(timeline.merges, [T("10:30"), T("11:20"), T("11:40")]);
  assert.equal(timeline.total, 3);
  assert.equal(timeline.stepsAt.length, graph.steps.length);
  assert.equal(timeline.end, T("13:00"));
});

test("the ship mark takes the verdict of the ship summary over a COMMENT on an own PR", () => {
  const { timeline } = buildRun(facts({ review: { ...facts().review, finalVerdict: "COMMENT" } }), {});
  assert.match(timeline.marks.at(-1).text, /Final review: APPROVE\./);
  const bare = buildRun(facts({ gate: { ...facts().gate, summary: null }, review: { ...facts().review, finalVerdict: "COMMENT" } }), {});
  assert.match(bare.timeline.marks.at(-1).text, /Final review: COMMENT\./);
});

test("notes add the lane reasons and the findings, and the title follows the milestone", () => {
  const notes = { title: "Demo, as it ran", lanes: { A: { what: "core", why: "it owns the core" } }, findings: { 12: "F1 (deferred): x" }, gate: "Merge it." };
  const { graph } = buildRun(facts(), notes);
  assert.equal(graph.title, "Demo, as it ran");
  assert.equal(graph.nodes.find((n) => n.id === "lane:A").label, "Lane A · core");
  assert.equal(graph.nodes.find((n) => n.id === "p:12").emphasis, true);
  assert.ok(graph.steps.some((s) => s.title === "Finding on Phase 12"));
  assert.equal(graph.steps.at(-1).text, "Merge it.");
  assert.equal(buildRun(facts(), {}).graph.title, "Milestone 2, as it ran");
});

test("without timestamps the run graph has no timeline but keeps the outcomes", () => {
  const { graph, timeline } = buildRun(facts({ start: null, end: null, events: [], checkpoints: [], review: { ...facts().review, startAt: null, endAt: null } }), {});
  assert.equal(timeline, null);
  assert.equal(graph.nodes.find((n) => n.id === "gate").status, "todo");
  // The final status of each phase stays on the node.
  const merged = buildRun(facts({ events: [{ phase: "11", status: "done", kind: "merge", text: "merged" }] }), {}).graph;
  assert.equal(merged.nodes.find((n) => n.id === "p:11").status, "done");
});

test("parseTimeline keeps known nodes and safe links only", () => {
  const out = parseTimeline({
    start: T("10:00"), end: T("12:00"),
    nodes: { a: [{ at: T("10:10"), status: "active" }, { at: "never", status: "done" }, { at: T("10:20"), status: "exploded" }], ghost: [{ at: T("10:10"), status: "done" }] },
    marks: [{ at: T("10:30"), label: "CP1", href: "javascript:alert(1)" }, { at: T("11:00"), label: "CP2", href: "https://x.test/cp2" }, { at: "bad", label: "X" }],
    stepsAt: [T("10:30"), null, "bad"], merges: [T("10:20"), "bad"], total: 3,
  }, new Set(["a"]));
  assert.deepEqual(out.timeline.nodes, { a: [{ at: T("10:10"), status: "active" }] });
  assert.deepEqual(out.timeline.marks.map((m) => [m.label, m.href]), [["CP1", undefined], ["CP2", "https://x.test/cp2"]]);
  assert.deepEqual(out.timeline.stepsAt, [T("10:30"), null, null]);
  assert.deepEqual(out.timeline.merges, [T("10:20")]);
  assert.ok(out.warnings.some((w) => /ghost/.test(w)));
  assert.equal(parseTimeline({ start: "x", end: T("10:00") }, new Set()).timeline, null);
  assert.equal(parseTimeline(null, new Set()).timeline, null);
});

test("renderExplore injects an escaped timeline, and no timeline keeps the bar hidden", () => {
  const { graph } = parseExplore(JSON.stringify({ nodes: [{ id: "a", label: "A", status: "todo" }] }));
  const opts = { template: TEMPLATE, elk: "/*elk*/" };
  const plain = renderExplore(graph, opts);
  assert.match(plain, /<script type="application\/json" id="timeline">null<\/script>/);
  assert.match(plain, /<div class="tl needs-js" id="tl" hidden>/);
  const timeline = { start: T("10:00"), end: T("11:00"), nodes: { a: [{ at: T("10:30"), status: "done" }] },
    marks: [{ at: T("10:30"), label: "CP1", text: "</script><script>alert(1)</script> $&  " }], stepsAt: [], merges: [] };
  const html = renderExplore(graph, { ...opts, timeline });
  assert.ok(!html.includes("</script><script>alert(1)"));
  assert.ok(!html.includes(" "));
  assert.deepEqual(timelineOf(html).marks[0].text, timeline.marks[0].text);
});

test("explore.mjs --timeline renders a graph with its timeline", () => {
  const g = path.join(TMP, "g.json");
  const t = path.join(TMP, "t.json");
  fs.writeFileSync(g, JSON.stringify({ nodes: [{ id: "a", label: "A", status: "todo" }] }));
  fs.writeFileSync(t, JSON.stringify({ start: T("10:00"), end: T("11:00"), nodes: { a: [{ at: T("10:30"), status: "done" }], b: [] }, marks: [] }));
  const out = spawnSync(process.execPath, [EXPLORE, "--graph", g, "--timeline", t, "--out", path.join(TMP, "e")], { encoding: "utf8" });
  assert.equal(out.status, 0, out.stderr);
  assert.match(out.stderr, /timeline: "b" is not a node/);
  assert.deepEqual(timelineOf(fs.readFileSync(path.join(TMP, "e", "explore.html"), "utf8")).nodes, { a: [{ at: T("10:30"), status: "done" }] });
});

// ---- the CLI on fixture loop dirs ----

const PLAN_PHASES = `### Phase 11 — Add the parser \`[lane: A]\` \`[status: done]\`
- **Repository:** \`acme/demo\`
- **Done when:** the parser reads a file.

### Phase 12 — Add the printer \`[lane: B]\` \`[status: done]\`
- **Repository:** \`acme/demo\`
- **Done when:** the printer prints.

### Phase 14 — Review the milestone \`[lane: review]\` \`[status: done]\` \`[kind: pr-review]\`
- **Depends on:** Phase 11, Phase 12
- **Done when:** the PR has a verdict.
`;

function write(dir, files) {
  for (const [rel, content] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
    fs.writeFileSync(path.join(dir, rel), typeof content === "string" ? content : JSON.stringify(content));
  }
  return dir;
}

// The old engine: an archived milestone 2, events.jsonl, retro facts, a ledger and a checkpoint.
function oldLoop() {
  const ev = (ts, event, phase, detail) => JSON.stringify({ ts, event, phase, repository: null, detail });
  return write(path.join(TMP, "old"), {
    "plan.md": "# Demo\n\n## Milestone 2 — Ship the demo `[archived]`\n\nArchived.\n",
    "milestones/m2.md": `# Demo — Milestone 2 archive\n\n## Raw log\n\n## Milestone 2 — Ship the demo\n- **PR:** https://github.com/acme/demo/pull/7\n\n${PLAN_PHASES}`,
    "state.json": { phases: { 11: { milestone: 2, slug: "p11-parser", status: "done", mergedAt: "zzz" }, 12: { milestone: 2, slug: "p12-printer", status: "done" }, 14: { milestone: 2, kind: "pr-review" } }, converge: { 14: { status: "converged" } } },
    "events.jsonl": [
      ev(T("09:00"), "milestone.resumed", "", "M1"),
      ev(T("10:00"), "milestone.resumed", "", "M2"),
      ev(T("10:01"), "phase.spawn", "11", "attempt a1 chain task pid 1"),
      ev(T("10:01"), "phase.spawn", "12", "attempt a1 chain task pid 2"),
      ev(T("10:20"), "phase.verify-fail", "12", "a1 exit 12: test <b>x</b> failed"),
      ev(T("10:20"), "phase.resume", "12", "attempt a2 resume"),
      ev(T("12:00"), "review.start", "14", "standard"),
      ev(T("12:50"), "review.finish", "14", "rstagi/dotfile=APPROVE"),
      ev(T("12:50"), "milestone.awaiting-merge", "", "M2 acme/demo=https://github.com/acme/demo/pull/7"),
    ].join("\n"),
    "explainers/m2-ship/retro-facts.json": { window: { start: T("10:00"), end: T("12:50") }, phases: [
      { phase: 11, attempts: [{ attempt: 1, model: "gpt", minutes: 29, verify: "pass", spawnedAt: T("10:01") }], progress: [] },
      { phase: 12, attempts: [{ attempt: 1, minutes: 19, verify: "fail", spawnedAt: T("10:01") }, { attempt: 2, minutes: 30, verify: "pass", spawnedAt: T("10:20") }], progress: ["- Phase 12 — done. Merged at abc1234."] },
    ] },
    "explainers/m2-ship/props.json": { reviews: [{ pr: "https://github.com/acme/demo/pull/7", url: "https://github.com/acme/demo/pull/7#r1", summary: "Verdict: APPROVE." }], handTest: [{ title: "Run", command: "demo" }], rollback: [{ step: "revert" }] },
    "explainers/m2-ship/link.txt": "https://claude.ai/artifact/ship\n",
    "explainers/m2-cp1/props.json": { meta: { checkpoint: { number: 1, phasesDone: 1, phasesTotal: 2 } } },
    "explainers/m2-cp1/link.txt": "https://claude.ai/artifact/cp1\n",
    "runs/explainer-m2-cp1-a1/meta.json": { startedAt: T("10:40") },
    "runs/review-p14-acme--demo.ledger.tsv": "a1\tround1\treview\tgpt\tAPPROVE\truns/review-p14-acme--demo-a1\na2\tfinal\tfinal\tclaude\tAPPROVE\truns/review-p14-acme--demo-a2\n",
    "converge/p14.md": "Verdict: converged\n\n| P11.1 parser reads | 11 | met | x |\n",
  });
}

test("run-graph.mjs renders an archived old-engine milestone with its timeline", () => {
  const loop = oldLoop();
  const out = path.join(TMP, "old-out");
  const res = run("--loop", loop, "--milestone", "2", "--out", out);
  assert.equal(res.status, 0, res.stderr);
  assert.equal(res.stdout.trim(), path.join(out, "run.html"));
  const html = fs.readFileSync(path.join(out, "run.html"), "utf8");
  const timeline = timelineOf(html);
  assert.equal(timeline.start, T("10:00"));
  assert.deepEqual(timeline.nodes["p:12"].map((x) => x.status), ["active", "failed", "active", "done"]);
  // Phase 12's merge has no git commit here: the end of its last attempt stands in.
  assert.equal(timeline.nodes["p:12"].at(-1).at, T("10:50"));
  assert.deepEqual(timeline.marks.map((m) => m.label), ["Start", "CP1", "Review", "Shipped"]);
  assert.equal(timeline.marks[1].href, "https://claude.ai/artifact/cp1");
  const graph = JSON.parse(fs.readFileSync(path.join(out, "run.graph.json"), "utf8"));
  assert.match(graph.nodes.find((n) => n.id === "gate").details, /final final \(claude\): APPROVE/);
  assert.ok(!html.includes("<b>x</b>"));
});

// The skills engine: a current milestone 1 in plan.md and the saved `ws log` output.
function wsLoop() {
  const e = (hhmm, type, body) => ({ ts_ms: Date.parse(T(hhmm)), type, body });
  return write(path.join(TMP, "ws"), {
    "plan.md": `# Demo\n\n## Phases\n\n${PLAN_PHASES}`,
    "explainers/m1-ship/loop-log.json": { session_id: "L", entries: [
      e("10:00", "loop.milestone", { milestone: 1 }),
      e("10:01", "loop.attempt", { n: 11, attempt: 1, session: "s1", state: "started" }),
      e("10:01", "loop.attempt", { n: 12, attempt: 1, session: "s2", state: "started" }),
      e("10:20", "loop.attempt", { n: 12, attempt: 1, session: "s2", state: "result", outcome: "done", verify: "fail" }),
      e("10:21", "loop.attempt", { n: 12, attempt: 2, session: "s3", state: "started" }),
      e("10:30", "loop.attempt", { n: 11, attempt: 1, session: "s1", state: "result", outcome: "done", verify: "pass" }),
      e("10:31", "loop.merge", { n: 11, repo: "acme/demo", head: "aaa1111", verify: "pass" }),
      e("10:50", "loop.attempt", { n: 12, attempt: 2, session: "s3", state: "result", outcome: "done", verify: "pass" }),
      e("10:52", "loop.merge", { n: 12, repo: "acme/demo", head: "bbb2222", verify: "pass" }),
      e("11:00", "loop.attempt", { n: 14, attempt: 1, session: "x", state: "started" }),
      e("12:00", "loop.review", { phase: 14, repo: "acme/demo", key: "k", role: "review", round: 1, state: "started", session: "r1" }),
      e("12:10", "loop.review", { phase: 14, repo: "acme/demo", key: "k", role: "review", round: 1, state: "finished", session: "r1", provider: "gpt", verdict: "APPROVE" }),
      e("12:40", "loop.review", { phase: 14, repo: "acme/demo", key: "k", role: "final", round: 2, state: "finished", session: "r2", provider: "claude", verdict: "APPROVE", commentUrl: "https://github.com/acme/demo/pull/7#r9" }),
      e("12:45", "loop.gate", { kind: "merge", state: "open", milestone: 1, prs: { "acme/demo": "https://github.com/acme/demo/pull/7" } }),
    ] },
  });
}

test("run-graph.mjs reads a skills-engine milestone from the saved ws log", () => {
  const loop = wsLoop();
  const out = path.join(TMP, "ws-out");
  const res = run("--loop", loop, "--milestone", "1", "--out", out);
  assert.equal(res.status, 0, res.stderr);
  const timeline = JSON.parse(fs.readFileSync(path.join(out, "run.timeline.json"), "utf8"));
  assert.equal(timeline.start, T("10:00"));
  assert.deepEqual(timeline.nodes["p:12"].map((x) => [x.at.slice(11, 16), x.status]), [["10:01", "active"], ["10:20", "failed"], ["10:21", "active"], ["10:52", "done"]]);
  assert.deepEqual(timeline.nodes.gate.map((x) => x.status), ["review", "done"]);
  assert.deepEqual(timeline.marks.map((m) => m.label), ["Start", "Review", "Shipped"]);
  const graph = JSON.parse(fs.readFileSync(path.join(out, "run.graph.json"), "utf8"));
  const gate = graph.nodes.find((n) => n.id === "gate");
  assert.deepEqual(gate.links.map((l) => l.href), ["https://github.com/acme/demo/pull/7", "https://github.com/acme/demo/pull/7#r9"]);
  assert.match(graph.nodes.find((n) => n.id === "p:11").sublabel, /1 attempt · 29 min/);
});

test("run-graph.mjs without any event source draws the graph with no timeline", () => {
  const loop = write(path.join(TMP, "bare"), { "plan.md": `# Demo\n\n## Phases\n\n${PLAN_PHASES}` });
  const out = path.join(TMP, "bare-out");
  const res = run("--loop", loop, "--milestone", "1", "--out", out);
  assert.equal(res.status, 0, res.stderr);
  assert.match(res.stderr, /no timestamps/);
  assert.match(fs.readFileSync(path.join(out, "run.html"), "utf8"), /id="timeline">null</);
});

test("run-graph.mjs rejects bad usage and an unknown milestone", () => {
  assert.equal(run("--loop", TMP).status, 2);
  assert.equal(run("--loop", TMP, "--milestone", "x", "--out", TMP).status, 2);
  const loop = write(path.join(TMP, "bare2"), { "plan.md": `# Demo\n\n## Phases\n\n${PLAN_PHASES}` });
  const res = run("--loop", loop, "--milestone", "9", "--out", path.join(TMP, "none"));
  assert.equal(res.status, 1);
  assert.match(res.stderr, /milestone 9 has no phases/);
});
