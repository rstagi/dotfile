import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SKILL = path.join(import.meta.dirname, "..", ".claude", "skills", "visual-explainer");
const RENDER = path.join(SKILL, "scripts", "render.mjs");
const { buildGraph } = await import(path.join(SKILL, "scripts", "graph.mjs"));
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "visual-explainer-graph-"));
process.on("exit", () => fs.rmSync(TMP, { recursive: true, force: true }));

const planProps = (over = {}) => ({
  mode: "plan",
  meta: { title: "Widget effort plan", subtitle: "Two milestones, one lane", date: "2026-10-18" },
  goal: "Serve widgets over HTTP.",
  architecture: {
    summary: "A thin HTTP layer over a store.",
    layers: [
      { name: "Interface", components: [{ name: "HTTP API", role: "Routes `GET /widgets`", status: "new" }] },
      { name: "Storage", components: [{ name: "Widget store", role: "SQLite tables" }] },
    ],
    flows: [{ from: "HTTP API", to: "Widget store", label: "reads widgets" }],
  },
  milestones: [
    {
      number: 1, title: "Read path", outcome: "Widgets are readable.",
      phases: [
        { number: 1, title: "Add the widget store", kind: "work", components: ["Widget store"] },
        { number: 2, title: "Serve GET /widgets", kind: "work", dependsOn: [1], components: ["HTTP API", "Widget store"] },
        { number: 3, title: "Review milestone 1", kind: "pr-review", gate: true, dependsOn: [2] },
      ],
    },
    { number: 2, title: "Write path", phases: [{ number: 4, title: "Accept POST /widgets", kind: "work", dependsOn: [3] }] },
  ],
  decisions: [{
    decision: "Use SQLite", why: "One file, no server.", phases: [1], components: ["Widget store"],
    rejected: [{ option: "Postgres", reason: "Needs a server." }],
  }],
  risks: [
    { risk: "Lock contention", impact: "high", mitigation: "WAL mode.", mitigatedBy: [1], components: ["Widget store"] },
    { risk: "Schema drift", impact: "low", mitigation: "Migrations in tests." },
  ],
  ...over,
});

const shipProps = (over = {}) => ({
  mode: "ship",
  meta: { title: "Widget ship report", subtitle: "Milestone 1 of the widget effort", date: "2026-10-18" },
  summary: "The widget service answers `GET /widgets`.",
  changes: [{ area: "Read API", how: "A route and a handler.", why: "Clients read widgets.", phases: [2] }],
  delivered: [
    { phase: 1, criterion: "The store has tests", result: "12 tests", status: "met" },
    { phase: 2, criterion: "GET /widgets returns 200", result: "Handler and route added", status: "partial" },
  ],
  deviations: [{ planned: "One handler", actual: "Two handlers", reason: "Pagination.", phases: [2] }],
  process: { attempts: 3 },
  setbacks: [{ phase: 2, kind: "retry", title: "Flaky test", what: "A port clash.", resolution: "A random port." }],
  reviews: [{
    verdict: "COMMENT", summary: "One finding.", validation: [{ command: "npm test", result: "pass" }],
    findings: [{ id: "F1", severity: "minor", title: "Missing index", status: "open", phases: [1], changes: ["Read API"] }],
  }],
  handTest: [
    { command: "npm start", ref: "src/server.js:12", why: "The server listens.", phases: [2] },
    { command: "curl localhost:3000/widgets", ref: "src/routes.js:4", why: "The route answers." },
  ],
  rollback: [{ step: "Revert the merge commit." }, { step: "Drop the table." }],
  ...over,
});

const ids = (graph) => graph.nodes.map((n) => n.id);
const edgeSet = (graph, type) => graph.edges.filter((e) => e.type === type).map((e) => `${e.from}>${e.to}`).sort();

function check(props, args = ["--check"]) {
  const file = path.join(fs.mkdtempSync(path.join(TMP, "case-")), "props.json");
  fs.writeFileSync(file, JSON.stringify(props));
  return spawnSync(process.execPath, [RENDER, "--props", file, ...args], { encoding: "utf8" });
}

// ---- nodes ----

test("graph (plan): one node for the goal, each milestone, phase, layer, component, decision and risk, with stable ids", () => {
  const graph = buildGraph(planProps());
  assert.deepEqual(ids(graph).sort(), [
    "component-http-api", "component-widget-store", "decision-1", "goal", "layer-1", "layer-2",
    "milestone-1", "milestone-2", "phase-1", "phase-2", "phase-3", "phase-4", "risk-1", "risk-2",
  ]);
  const byId = Object.fromEntries(graph.nodes.map((n) => [n.id, n]));
  assert.deepEqual([byId.goal.kind, byId["phase-2"].kind, byId["component-http-api"].kind, byId["risk-1"].kind], ["goal", "phase", "component", "risk"]);
  assert.equal(byId["phase-2"].title, "Serve GET /widgets");
  assert.equal(byId["milestone-1"].summary, "Widgets are readable.");
  assert.equal(byId["component-http-api"].summary, "Routes `GET /widgets`");
});

test("graph (plan): each node carries every fact of its item", () => {
  const graph = buildGraph(planProps());
  const facts = (id) => JSON.stringify(graph.nodes.find((n) => n.id === id).facts);
  for (const text of ["Use SQLite", "One file, no server.", "Postgres", "Needs a server."]) assert.ok(facts("decision-1").includes(text), text);
  for (const text of ["Lock contention", "high", "WAL mode."]) assert.ok(facts("risk-1").includes(text), text);
  for (const text of ["Serve widgets over HTTP.", "A thin HTTP layer over a store.", "2026-10-18"]) assert.ok(facts("goal").includes(text), text);
  assert.ok(facts("component-http-api").includes("reads widgets"), "the flow label is a fact of its component");
});

test("graph (ship): nodes for the outcome, process, changes, delivered phases, deviations, setbacks, reviews, findings and steps", () => {
  const graph = buildGraph(shipProps());
  assert.deepEqual(ids(graph).sort(), [
    "change-1", "deviation-1", "finding-f1", "phase-1", "phase-2", "process", "review-1",
    "rollback-1", "rollback-2", "setback-1", "step-1", "step-2", "summary",
  ]);
  const phase2 = graph.nodes.find((n) => n.id === "phase-2");
  assert.equal(phase2.kind, "delivered");
  assert.equal(phase2.tone, "warn", "a partial row");
});

test("graph (ship): delivered rows of one phase share a node, rows without a phase get their own node", () => {
  const graph = buildGraph(shipProps({
    delivered: [
      { phase: 7, criterion: "A", result: "a", status: "met" },
      { phase: 7, criterion: "B", result: "b", status: "missed" },
      { criterion: "C", result: "c", status: "met" },
    ],
    changes: [], deviations: [], setbacks: [], handTest: [{ command: "x", ref: "a.js:1", why: "w" }],
    reviews: [{ verdict: "APPROVE", summary: "s", validation: [{ command: "t", result: "pass" }], findings: [] }],
  }));
  const delivered = graph.nodes.filter((n) => n.kind === "delivered");
  assert.deepEqual(delivered.map((n) => n.id), ["phase-7", "delivered-3"]);
  assert.equal(delivered[0].tone, "bad", "the worst status of the phase");
  assert.ok(JSON.stringify(delivered[0].facts).includes('"B"'));
});

// ---- edges ----

test("graph (plan): structural edges need no extra fields", () => {
  const graph = buildGraph(planProps());
  assert.deepEqual(edgeSet(graph, "contains"), [
    "goal>milestone-1", "goal>milestone-2", "layer-1>component-http-api", "layer-2>component-widget-store",
    "milestone-1>phase-1", "milestone-1>phase-2", "milestone-1>phase-3", "milestone-2>phase-4",
  ]);
  assert.deepEqual(edgeSet(graph, "depends"), ["phase-1>phase-2", "phase-2>phase-3", "phase-3>phase-4"], "from the dependency to the dependent phase");
  assert.deepEqual(edgeSet(graph, "flow"), ["component-http-api>component-widget-store"]);
  assert.equal(graph.edges.find((e) => e.type === "flow").label, "reads widgets");
});

test("graph (plan): relation fields add edges", () => {
  const graph = buildGraph(planProps());
  assert.deepEqual(edgeSet(graph, "touches"), ["phase-1>component-widget-store", "phase-2>component-http-api", "phase-2>component-widget-store"]);
  assert.deepEqual(edgeSet(graph, "affects"), ["decision-1>component-widget-store", "decision-1>phase-1", "risk-1>component-widget-store"]);
  assert.deepEqual(edgeSet(graph, "mitigates"), ["phase-1>risk-1"]);
});

test("graph (ship): relation fields and step order add edges", () => {
  const graph = buildGraph(shipProps());
  assert.deepEqual(edgeSet(graph, "delivers"), ["phase-2>change-1"]);
  assert.deepEqual(edgeSet(graph, "concerns"), ["deviation-1>phase-2"]);
  assert.deepEqual(edgeSet(graph, "during"), ["setback-1>phase-2"]);
  assert.deepEqual(edgeSet(graph, "about"), ["finding-f1>change-1", "finding-f1>phase-1"]);
  assert.deepEqual(edgeSet(graph, "checks"), ["step-1>phase-2"]);
  assert.deepEqual(edgeSet(graph, "then"), ["rollback-1>rollback-2", "step-1>step-2"]);
  assert.deepEqual(edgeSet(graph, "contains"), ["review-1>finding-f1", "summary>review-1"]);
});

test("graph: legacy references to unknown items make no edge and stay valid", () => {
  const props = planProps();
  props.milestones[0].phases[0].dependsOn = [55];
  props.architecture.flows.push({ from: "Browser", to: "HTTP API", label: "calls" });
  const graph = buildGraph(props);
  assert.ok(!graph.edges.some((e) => e.from === "phase-55"), "no edge to a phase outside the props");
  assert.ok(JSON.stringify(graph.nodes.find((n) => n.id === "phase-1").facts).includes("P55"), "the fact stays");
  assert.ok(JSON.stringify(graph.nodes.find((n) => n.id === "component-http-api").facts).includes("Browser"));
  assert.equal(check(props).status, 0);
});

// ---- the props check: unknown ids ----

test("check: an unknown id in a relation field fails with its field path and a clear message", () => {
  const props = planProps();
  props.milestones[0].phases[0].components = ["Widget store", "Cache"];
  props.decisions[0].phases = [9];
  props.risks[0].mitigatedBy = [1, 42];
  const r = check(props);
  assert.equal(r.status, 1, r.stderr);
  assert.deepEqual(r.stderr.trim().split("\n"), [
    '$.milestones[0].phases[0].components[1]: no component "Cache" in the architecture',
    "$.decisions[0].phases[0]: no phase 9 in the milestones",
    "$.risks[0].mitigatedBy[1]: no phase 42 in the milestones",
  ]);
});

test("check (ship): a phase reference needs a delivered row of that phase, a change reference needs its area", () => {
  const props = shipProps();
  props.handTest[1].phases = [5];
  props.reviews[0].findings[0].changes = ["Write API"];
  const r = check(props);
  assert.equal(r.status, 1, r.stderr);
  assert.deepEqual(r.stderr.trim().split("\n"), [
    '$.reviews[0].findings[0].changes[0]: no change with the area "Write API"',
    "$.handTest[1].phases[0]: no delivered row has phase 5",
  ]);
});

test("check: a phase number or a milestone number that appears twice fails", () => {
  const props = planProps();
  props.milestones[1].phases[0].number = 2;
  const r = check(props);
  assert.equal(r.status, 1);
  assert.match(r.stderr, /^\$\.milestones\[1\]\.phases\[0\]\.number: phase 2 appears more than once$/m);
});

test("--ids lists every node id and group id with its kind and title", () => {
  const r = check(planProps(), ["--ids"]);
  assert.equal(r.status, 0, r.stderr);
  const lines = r.stdout.trim().split("\n");
  assert.ok(lines.includes("phase-2\tphase\tServe GET /widgets"), r.stdout);
  assert.ok(lines.includes("all\tgroup\tThe whole graph"));
  assert.ok(lines.some((l) => l.startsWith("group-timeline\tgroup\t")));
});

// ---- layout ----

const many = (n, make) => Array.from({ length: n }, (_, i) => make(i + 1));

function bigPlan(phases = 48) {
  const components = many(12, (i) => ({ name: `Component ${i}`, role: `Role ${i} with a longer description of the work` }));
  return planProps({
    architecture: {
      summary: "Big.",
      layers: [{ name: "Top", components: components.slice(0, 5) }, { name: "Middle", components: components.slice(5, 9) }, { name: "Bottom", components: components.slice(9) }],
      flows: many(6, (i) => ({ from: `Component ${i}`, to: `Component ${i + 3}`, label: "calls" })),
    },
    milestones: [
      { number: 1, title: "First", phases: many(phases / 2, (i) => ({ number: i, title: `Phase ${i} does a thing that has a long title for wrapping`, kind: "work", dependsOn: i > 1 ? [i - 1] : [], components: [`Component ${(i % 12) + 1}`] })) },
      { number: 2, title: "Second", phases: many(phases / 2, (i) => ({ number: phases / 2 + i, title: `Phase ${phases / 2 + i}`, kind: i === phases / 2 ? "pr-review" : "work", dependsOn: [phases / 2 + i - 1] })) },
    ],
    decisions: many(10, (i) => ({ decision: `Decision ${i}`, why: "Why.", rejected: [{ option: "O", reason: "R" }], phases: [i] })),
    risks: many(10, (i) => ({ risk: `Risk ${i}`, impact: "medium", mitigation: "M.", mitigatedBy: [i * 2] })),
  });
}

function overlaps(a, b, gap = 0) {
  return a.x < b.x + b.w + gap && b.x < a.x + a.w + gap && a.y < b.y + b.h + gap && b.y < a.y + a.h + gap;
}

test("layout: the same props always give the same positions", () => {
  const props = bigPlan();
  const first = buildGraph(props);
  const second = buildGraph(JSON.parse(JSON.stringify(props)));
  assert.deepEqual(second, first);
  assert.ok(first.nodes.every((n) => [n.x, n.y, n.w, n.h].every(Number.isFinite)));
});

for (const [label, props] of [["the plan fixture", planProps()], ["the ship fixture", shipProps()], ["48 phases", bigPlan(48)], ["80 phases", bigPlan(80)]]) {
  test(`layout (${label}): no two node boxes overlap, each node sits inside its group frame, frames do not overlap`, () => {
    const graph = buildGraph(props);
    for (let i = 0; i < graph.nodes.length; i += 1) {
      for (let j = i + 1; j < graph.nodes.length; j += 1) {
        assert.ok(!overlaps(graph.nodes[i], graph.nodes[j], 12), `${graph.nodes[i].id} and ${graph.nodes[j].id} overlap or touch`);
      }
    }
    const groups = Object.fromEntries(graph.groups.map((g) => [g.id, g]));
    for (const node of graph.nodes.filter((n) => n.group)) {
      const g = groups[node.group];
      assert.ok(node.x >= g.x && node.y >= g.y && node.x + node.w <= g.x + g.w && node.y + node.h <= g.y + g.h, `${node.id} inside ${g.id}`);
    }
    for (let i = 0; i < graph.groups.length; i += 1) {
      for (let j = i + 1; j < graph.groups.length; j += 1) assert.ok(!overlaps(graph.groups[i], graph.groups[j], 24), `${graph.groups[i].id} and ${graph.groups[j].id}`);
    }
    assert.ok(graph.nodes.every((n) => n.x >= 0 && n.y >= 0 && n.x + n.w <= graph.width && n.y + n.h <= graph.height), "inside the bounds");
  });
}

test("layout: a plan with 48 phases stays compact and close to a screen shape", () => {
  const graph = buildGraph(bigPlan(48));
  const ratio = graph.width / graph.height;
  assert.ok(ratio > 0.8 && ratio < 2.6, `width/height ${ratio.toFixed(2)}`);
  const timeline = graph.groups.find((g) => g.id === "group-timeline");
  assert.ok(timeline.w / timeline.h < 3 && timeline.h / timeline.w < 3, "the phases form a block, not a strip");
  const area = graph.nodes.reduce((sum, n) => sum + n.w * n.h, 0);
  assert.ok(area / (graph.width * graph.height) > 0.22, `nodes fill ${(area / (graph.width * graph.height)).toFixed(2)} of the canvas`);
});

test("layout: phases keep plan order in reading order, and each title fits its box in at most 2 lines", () => {
  const graph = buildGraph(bigPlan(48));
  const phases = graph.nodes.filter((n) => n.kind === "phase" && n.id !== "phase-25");
  const first = phases.filter((n) => Number(n.id.slice(6)) <= 24);
  const order = [...first].sort((a, b) => a.y - b.y || a.x - b.x).map((n) => n.id);
  assert.deepEqual(order, first.map((n) => n.id));
  for (const node of graph.nodes) {
    assert.ok(node.lines.length >= 1 && node.lines.length <= (node.kind === "goal" ? 3 : 2), node.id);
    assert.ok(node.lines.every((l) => l.length <= 26), `${node.id}: ${node.lines.join(" | ")}`);
  }
});

test("layout: every edge has a path from its source box to its target box", () => {
  const graph = buildGraph(planProps());
  for (const edge of graph.edges) assert.match(edge.d, /^M-?[\d.]+ -?[\d.]+ [QL]/, `${edge.from}>${edge.to}`);
});
