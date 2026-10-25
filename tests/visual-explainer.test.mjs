import { after, test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SKILL = path.join(import.meta.dirname, "..", ".claude", "skills", "visual-explainer");
const CLI = path.join(SKILL, "scripts", "render.mjs");
const { buildGraph } = await import(path.join(SKILL, "scripts", "graph.mjs"));
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "visual-explainer-"));
after(() => fs.rmSync(TMP, { recursive: true, force: true }));

const shipProps = (over = {}) => ({
  mode: "ship",
  meta: { title: "Widget ship report", subtitle: "Milestone 1 of the widget effort", date: "2026-10-18" },
  summary: "The widget service answers `GET /widgets`.",
  delivered: [{ criterion: "GET /widgets returns 200", result: "Handler and route added", status: "met" }],
  deviations: [],
  reviews: [{
    verdict: "APPROVE",
    summary: "No open findings.",
    validation: [{ command: "npm test", result: "pass" }],
    findings: [],
  }],
  handTest: [
    { command: "npm start", ref: "src/server.js:12", why: "The server listens on port 3000." },
  ],
  rollback: [{ step: "Revert the merge commit.", command: "git revert -m1 <merge-sha>" }],
  ...over,
});

const planProps = (over = {}) => ({
  mode: "plan",
  meta: { title: "Widget effort plan", subtitle: "Two milestones, one lane", date: "2026-10-18", planId: "loop-widgets-2026-10-18-1200" },
  goal: "Serve widgets over HTTP.",
  architecture: {
    summary: "A thin HTTP layer over a store.",
    layers: [
      { name: "Interface", components: [{ name: "HTTP API", role: "Routes `GET /widgets`" }] },
      { name: "Storage", components: [{ name: "Widget store", role: "SQLite tables" }] },
    ],
    flows: [{ from: "HTTP API", to: "Widget store", label: "reads widgets" }],
  },
  milestones: [
    {
      number: 1, title: "Read path", outcome: "Widgets are readable.",
      phases: [
        { number: 1, title: "Add the widget store", kind: "work", lane: "A", repository: "acme/widgets", estimate: "15-20 min", doneWhen: "Store tests pass" },
        { number: 2, title: "Serve GET /widgets", kind: "work", lane: "A", dependsOn: [1] },
        { number: 3, title: "Review milestone 1", kind: "pr-review", gate: true, dependsOn: [2] },
      ],
    },
    { number: 2, title: "Write path", phases: [{ number: 4, title: "Accept POST /widgets", kind: "work", dependsOn: [3] }] },
  ],
  decisions: [{
    decision: "Use SQLite", why: "One file, no server.",
    rejected: [{ option: "Postgres", reason: "Needs a server for a local tool." }],
  }],
  risks: [
    { risk: "Schema drift", impact: "low", mitigation: "Migrations in tests." },
    { risk: "Lock contention", impact: "high", likelihood: "medium", mitigation: "WAL mode." },
  ],
  ...over,
});

// render(props) -> the CLI result plus `report` (index.html) and `html` (graph.html, the concept graph).
// `files` maps relative paths to Buffers written beside props.json (media fixtures).
function render(props, { args = [], files = {}, propsFile } = {}) {
  const dir = fs.mkdtempSync(path.join(TMP, "case-"));
  if (!propsFile) {
    propsFile = path.join(dir, "props.json");
    fs.writeFileSync(propsFile, typeof props === "string" ? props : JSON.stringify(props));
  }
  for (const [rel, data] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
    fs.writeFileSync(path.join(dir, rel), data);
  }
  const out = path.join(dir, "out");
  const target = args.includes("--check") ? [] : ["--out", out];
  const res = spawnSync(process.execPath, [CLI, "--props", propsFile, ...target, ...args.map((a) => a.replace("<dir>", dir))], { encoding: "utf8" });
  const page = path.join(out, "index.html");
  const graphPage = path.join(out, "graph.html");
  const read = (file) => (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : null);
  return { ...res, dir, out, page, graphPage, report: read(page), html: read(graphPage) };
}

// The graph data of the page: the JSON in <script type="application/json" id="ve-data">.
function pageData(html) {
  const match = /<script type="application\/json" id="ve-data">([^<]*)<\/script>/.exec(html);
  assert.ok(match, "the page has its graph data");
  return JSON.parse(match[1]);
}

// The outline entry of one node: from its <li> to the next one.
function outlineEntry(html, id) {
  const start = html.indexOf(`<li class="o-node" id="o-${id}"`);
  assert.ok(start > 0, `outline entry of ${id}`);
  const end = html.indexOf("</li>\n", start);
  return html.slice(start, end);
}

test("ship: valid props write index.html (report) and graph.html, print both paths and exit 0", () => {
  const r = render(shipProps());
  assert.equal(r.status, 0, r.stderr);
  assert.ok(r.report && r.html, "both pages exist");
  assert.match(r.report, /<title>Widget ship report<\/title>/);
  assert.match(r.html, /<title>Widget ship report<\/title>/);
  assert.deepEqual(r.stdout.trim().split("\n"), [r.page, r.graphPage]);
  assert.match(r.report, /<a class="graph-link" href="graph\.html">Explore the concept graph →<\/a>/, "the report links to the graph");
  assert.match(r.html, /<a class="btn" href="index\.html">← Report<\/a>/, "the graph links back to the report");
});

test("ship: a hand-test step without a code reference is rejected with its field path", () => {
  const r = render(shipProps({ handTest: [{ command: "npm start", why: "The server listens." }] }));
  assert.equal(r.status, 1);
  assert.match(r.stderr, /\$\.handTest\[0\]\.ref: required/);
  assert.equal(r.html, null, "no page is written for invalid props");
  assert.equal(r.report, null, "no report is written for invalid props");
});

test("validation: prints every error on its own line with its field path", () => {
  const props = shipProps({
    meta: { title: "Ship", subtitle: "s", date: "18/10/2026" },
    delivered: [{ criterion: "c", result: "r", status: "done" }],
    handTest: [{ command: "npm start", ref: "src/server.js", why: "w" }],
    extra: true,
  });
  props.reviews[0].findings = [{ severity: "major", title: 42, status: "open" }];
  const r = render(props);
  assert.equal(r.status, 1);
  const lines = r.stderr.trim().split("\n");
  assert.deepEqual(lines.map((l) => l.split(":")[0]), [
    "$.meta.title",
    "$.meta.date",
    "$.delivered[0].status",
    "$.reviews[0].findings[0].title",
    "$.handTest[0].ref",
    "$.extra",
  ]);
  assert.match(r.stderr, /\$\.meta\.title: must match .*2 to 4 words/);
  assert.match(r.stderr, /\$\.delivered\[0\]\.status: must be one of met, partial, missed/);
  assert.match(r.stderr, /\$\.reviews\[0\]\.findings\[0\]\.title: must be string, got integer/);
  assert.match(r.stderr, /\$\.handTest\[0\]\.ref: must match .*path:line/);
  assert.match(r.stderr, /\$\.extra: unknown field/);
});

// The page was a report with fixed sections. Now it is the concept graph: these tests replace the
// section-order tests of the report.
test("page: the page is the graph, not a report: graph data, one svg node per graph node, no report sections", () => {
  const props = shipProps({ deviations: [{ planned: "Fix F4 in this milestone", actual: "F4 moved to Phase 71", reason: "The fix needs the tmux driver rework." }] });
  const r = render(props);
  assert.equal(r.status, 0, r.stderr);
  const data = pageData(r.html);
  const graph = buildGraph(props);
  assert.deepEqual(data.nodes.map((n) => [n.id, n.kind, n.x, n.y, n.w, n.h]), graph.nodes.map((n) => [n.id, n.kind, n.x, n.y, n.w, n.h]), "the layout of graph.mjs");
  assert.deepEqual(data.edges.map((e) => `${e.from}>${e.to}`), graph.edges.map((e) => `${e.from}>${e.to}`));
  assert.deepEqual([data.width, data.height], [graph.width, graph.height]);
  for (const node of graph.nodes) {
    assert.match(r.html, new RegExp(`<g class="n k-${node.kind}[^"]*" data-id="${node.id}" transform="translate\\(${node.x} ${node.y}\\)"`), node.id);
  }
  assert.match(r.html, new RegExp(`<svg class="graph" id="graph" viewBox="0 0 ${graph.width} ${graph.height}"`));
  assert.ok(!/class="scene"|<nav class="toc"|<h2>Outcome<\/h2>|<h2>Hand test<\/h2>/.test(r.html), "no report sections");
});

test("page: every fact of the props is in the outline, escaped, with inline code and links", () => {
  const props = shipProps({
    deviations: [{ planned: "Fix F4 in this milestone", actual: "F4 moved to Phase 71", reason: "The fix needs the tmux driver rework." }],
    handTest: [
      { command: "./wsd migrate --dry-run", ref: "wsd/src/import.rs:50", why: "The dry-run writes nothing." },
      { command: "./install.sh ws", ref: "install.sh:412", why: "The install starts the daemon, which imports at first start." },
    ],
  });
  props.reviews[0] = {
    repository: "acme/widgets",
    pr: "https://github.com/acme/widgets/pull/7",
    verdict: "COMMENT",
    url: "https://github.com/acme/widgets/pull/7#pullrequestreview-1",
    summary: "One major finding stays open.",
    validation: [{ command: "npm test", result: "pass", note: "212 tests" }],
    findings: [
      { id: "F2", severity: "minor", title: "Socket files pile up", status: "fixed" },
      { id: "F4", severity: "major", title: "Window lookup fails under load", status: "deferred", fix: "Phase 71" },
    ],
  };
  const r = render(props);
  assert.equal(r.status, 0, r.stderr);
  const outline = r.html.slice(r.html.indexOf('id="outline"'));
  for (const text of [
    "GET /widgets returns 200", "F4 moved to Phase 71", "The fix needs the tmux driver rework.", "One major finding stays open.",
    "Window lookup fails under load", "212 tests", "git revert -m1 &lt;merge-sha&gt;", "./wsd migrate --dry-run",
    "wsd/src/import.rs:50", "install.sh:412", "The dry-run writes nothing.", "Phase 71",
  ]) assert.ok(outline.includes(text), `the outline shows ${text}`);
  assert.match(outline, /<code>GET \/widgets<\/code>/, "backticks become inline code");
  assert.match(outline, /href="https:\/\/github\.com\/acme\/widgets\/pull\/7#pullrequestreview-1"/);
  assert.ok(outline.indexOf('id="o-finding-f4"') < outline.indexOf('id="o-finding-f2"'), "the major finding comes before the minor one");
  assert.ok(outline.indexOf('id="o-step-1"') < outline.indexOf('id="o-step-2"'), "hand-test steps keep their order");
});

test("page: deep links, and an outline of every node grouped by kind in legend order", () => {
  const props = planProps();
  const r = render(props);
  assert.equal(r.status, 0, r.stderr);
  const graph = buildGraph(props);
  for (const node of graph.nodes) {
    const entry = outlineEntry(r.html, node.id);
    assert.ok(entry.includes(`href="#node=${node.id}"`), `${node.id} has a deep link`);
  }
  const kinds = [...r.html.matchAll(/<section class="o-kind" id="o-k-([a-z]+)"/g)].map((m) => m[1]);
  assert.deepEqual(kinds, ["goal", "milestone", "phase", "layer", "component", "decision", "risk"]);
  assert.match(r.html, /<section class="outline" id="outline" aria-label="Outline">/);
  const phase2 = outlineEntry(r.html, "phase-2");
  assert.match(phase2, /<dt>Depends on<\/dt><dd><a class="near-link k-phase" href="#node=phase-1">/, "a connection is a link to its node");
  assert.match(phase2, /<dt>Unblocks<\/dt><dd><a class="near-link k-phase" href="#node=phase-3">/);
  assert.match(phase2, /<dt>Part of<\/dt><dd><a class="near-link k-milestone" href="#node=milestone-1">/);
  assert.match(r.html, /addEventListener\("hashchange"/, "the script follows the URL hash");
});

test("page: the plan graph marks gates, statuses and risk impact on its nodes", () => {
  const r = render(planProps());
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.html, /<g class="n k-phase t-warn" data-id="phase-3"[^>]*>[\s\S]*?P3 · review · merge gate/, "the gate phase");
  assert.match(r.html, /<g class="n k-risk t-bad" data-id="risk-2"/, "a high-impact risk");
  for (const text of ["Serve widgets over HTTP.", "Widget store", "reads widgets", "Read path", "Widgets are readable.", "Add the widget store",
    "Store tests pass", "15-20 min", "acme/widgets", "Postgres", "Needs a server for a local tool.", "WAL mode.", "loop-widgets-2026-10-18-1200",
  ]) assert.ok(r.html.includes(text), `page shows ${text}`);
  const risks = [...r.html.matchAll(/<li class="o-node" id="o-(risk-\d)"/g)].map((m) => m[1]);
  assert.deepEqual(risks, ["risk-2", "risk-1"], "high-impact risks come first");
});

test("page: without a narration, the tour visits the root and each group between two overviews", () => {
  const r = render(planProps());
  assert.equal(r.status, 0, r.stderr);
  const { stops } = pageData(r.html);
  assert.deepEqual(stops.map((s) => s.overview), [true, false, false, false, false, false, true]);
  assert.deepEqual(stops[1].nodes, ["goal"]);
  assert.deepEqual(stops.slice(2, 6).map((s) => s.title), ["Timeline", "Architecture", "Decisions", "Risks"]);
  assert.ok(stops.every((s) => s.text.length > 0), "every stop has a caption");
  assert.match(r.html, /<button type="button" class="btn" data-tour>Play tour<\/button>/);
});

const processProps = {
  phases: 36, attempts: 44, retries: 9, escalations: 2, questions: 3, hil: 1,
  mergeConflicts: 0, reviewRounds: 2, fixRounds: 1, wallTime: "20h 21m",
};
const setbackProps = [
  {
    phase: 21, kind: "hil", title: "Verify flaked under load",
    what: "The notifications tests failed in about half of the full Verify runs.",
    cost: "6 attempts, 134 min", resolution: "The human chose deterministic tests.",
    lesson: "Gate wall-clock tests on events, not on sleeps.",
  },
  { kind: "replan", title: "Plan edit", what: "Phase 71 was added.", resolution: "The plan moved F4 to milestone 3." },
];

test("ship: the process numbers and each setback are nodes with all their facts", () => {
  const r = render(shipProps({ process: processProps, setbacks: setbackProps }));
  assert.equal(r.status, 0, r.stderr);
  const process = outlineEntry(r.html, "process");
  for (const stat of ["36 phases", "44 attempts", "9 retries", "2 escalations", "3 questions", "1 HIL", "0 merge conflicts", "2 review rounds", "1 fix rounds", "20h 21m wall time"]) {
    assert.ok(process.includes(stat), `stat ${stat}`);
  }
  const first = outlineEntry(r.html, "setback-1");
  for (const text of ["Verify flaked under load", "P21", "6 attempts, 134 min", "The notifications tests failed in about half of the full Verify runs.",
    "The human chose deterministic tests.", "Gate wall-clock tests on events, not on sleeps."]) assert.ok(first.includes(text), `setback shows ${text}`);
  assert.ok(outlineEntry(r.html, "setback-2").includes("Phase 71 was added."));
  assert.match(r.html, /<g class="n k-setback t-info" data-id="setback-1"/, "a HIL setback");
  assert.ok(pageData(r.html).groups.some((g) => g.id === "group-setbacks"));
});

test("ship: without process and setbacks there is no setbacks group", () => {
  const r = render(shipProps());
  assert.equal(r.status, 0, r.stderr);
  assert.ok(!pageData(r.html).groups.some((g) => g.id === "group-setbacks"));
  assert.ok(!r.html.includes('id="o-k-setback"') && !r.html.includes('id="o-process"') && !r.html.includes('data-id="process"'));
  const numbersOnly = render(shipProps({ process: { attempts: 3 } }));
  assert.equal(numbersOnly.status, 0, numbersOnly.stderr);
  assert.ok(outlineEntry(numbersOnly.html, "process").includes("3 attempts"));
  assert.ok(!outlineEntry(numbersOnly.html, "process").includes("phases"), "an absent number is not shown");
});

test("ship: setbacks and process are checked against the schema", () => {
  const r = render(shipProps({
    process: { attempts: -1, retries: 1.5, extra: 1 },
    setbacks: [{ kind: "flake", title: "t", what: "w" }, { phase: 2, kind: "retry", title: "t", what: "w", resolution: "r", cause: "x" }],
  }));
  assert.equal(r.status, 1);
  assert.deepEqual(r.stderr.trim().split("\n").map((l) => l.split(":")[0]), [
    "$.process.attempts", "$.process.retries", "$.process.extra",
    "$.setbacks[0].resolution", "$.setbacks[0].kind", "$.setbacks[1].cause",
  ]);
  assert.match(r.stderr, /\$\.setbacks\[0\]\.kind: must be one of retry, verify-fail, timeout, rate-limit, model-escalation, question, hil, merge-conflict, review-round, review-dispute, replan, environment, other/);
});

test("escaping: props text cannot inject HTML, script or template slots", () => {
  const evil = `<script>alert(1)</script>"><img src=x onerror=alert(2)>`;
  const props = shipProps({
    meta: { title: "Evil <b>ship</b> report", subtitle: `${evil} $& $' {{SCENES}}`, date: "2026-10-18" },
    summary: `Run \`</code><script>alert(3)</script>\` now`,
    deviations: [{ planned: evil, actual: "x' onmouseover='alert(4)", reason: "{{HERO}}" }],
    process: { wallTime: evil },
    setbacks: [{ kind: "other", title: evil, what: evil, cost: evil, resolution: evil, lesson: evil }],
    handTest: [{ title: evil, command: `echo "${evil}"`, ref: "src/a.js:1", why: evil, expect: evil }],
  });
  props.reviews[0].findings = [{ id: "F1", severity: "major", title: evil, location: evil, detail: evil, fix: evil, status: "open" }];
  props.reviews[0].validation = [{ command: evil, result: "fail", note: evil }];
  const r = render(props);
  assert.equal(r.status, 0, r.stderr);
  assert.equal((r.html.match(/<script/g) ?? []).length, 3, "only the template scripts and the data block exist");
  assert.ok(!/<img/i.test(r.html), "no injected element");
  assert.ok(!/<b>ship<\/b>/.test(r.html), "markup in a title stays text");
  assert.ok(!r.html.includes("' onmouseover='"), "quotes cannot break out of an attribute or text");
  assert.ok(r.html.includes("&lt;script&gt;alert(1)&lt;/script&gt;"), "the hostile text is shown, escaped");
  assert.ok(r.html.includes("&lt;/code&gt;&lt;script&gt;alert(3)&lt;/script&gt;"), "inline code content is escaped");
  assert.ok(r.html.includes("$&amp; $&#39; {{SCENES}}"), "replacement patterns and slot names stay literal");
  assert.ok(r.html.includes("{{HERO}}"), "a slot name inside props is never expanded");
  const data = pageData(r.html);
  assert.ok(data.nodes.some((n) => n.title.includes(evil)), "the data block keeps the text, with no markup in the page source");
});

test("escaping: link URLs must be http or https", () => {
  const props = shipProps({ meta: { title: "Link test page", subtitle: "s", date: "2026-10-18", links: [{ label: "x", url: "javascript:alert(1)" }] } });
  const r = render(props);
  assert.equal(r.status, 1);
  assert.match(r.stderr, /\$\.meta\.links\[0\]\.url: must match .*http or https URL/);
});

test("plan: a decision without a rejected option is invalid", () => {
  const props = planProps({ decisions: [{ decision: "Use SQLite", why: "Simple.", rejected: [] }] });
  const r = render(props);
  assert.equal(r.status, 1);
  assert.match(r.stderr, /\$\.decisions\[0\]\.rejected: must have at least 1 item/);
});

for (const [mode, props] of [["plan", planProps()], ["ship", shipProps()]]) {
  test(`page contract (${mode}): self-contained, themed, phone-safe, motion-aware`, () => {
    const r = render(props);
    assert.equal(r.status, 0, r.stderr);
    assert.ok(!r.html.includes("narration"), "the graph page does not read a narration");
    const title = /<title>([^<]*)<\/title>/.exec(r.html)[1];
    const words = title.split(" ").length;
    assert.ok(words >= 2 && words <= 4, `title has 2-4 words: "${title}"`);
    assert.match(r.html, /<meta name="viewport" content="width=device-width, initial-scale=1">/);
    assert.match(r.html, /:root \{[^}]*--bg:/, "color tokens on :root");
    assert.match(r.html, /@media \(prefers-color-scheme: dark\) \{\s*:root:not\(\[data-theme="light"\]\) \{[^}]*--bg:/, "dark tokens under the media query");
    assert.match(r.html, /:root\[data-theme="dark"\] \{[^}]*--bg:/, "dark tokens for an explicit dark theme");
    assert.match(r.html, /\nbody \{[^}]*background: var\(--bg\);/, "explicit body background");
    assert.match(r.html, /\.wrap \{[^}]*padding: 0 16px;/, "16px side gutter");
    assert.match(r.html, /@media \(prefers-reduced-motion: no-preference\)/, "motion only when allowed");
    assert.match(r.html, /@media \(max-width: 640px\)/, "a phone layout");
    assert.ok(!/<script[^>]+src=|<link\b|@import|url\(\s*["']?https?:/i.test(r.html), "no external scripts, styles or assets");
    assert.ok(!/\{\{[A-Z_]+\}\}/.test(r.html), "every template slot is filled");
  });
}

test("--check validates without writing a page", () => {
  const ok = render(shipProps(), { args: ["--check"] });
  assert.equal(ok.status, 0, ok.stderr);
  assert.equal(ok.html, null, "no page is written");
  const bad = render(shipProps({ rollback: [] }), { args: ["--check"] });
  assert.equal(bad.status, 1);
  assert.match(bad.stderr, /\$\.rollback: must have at least 1 item/);
});

test("usage errors exit 2", () => {
  const res = spawnSync(process.execPath, [CLI], { encoding: "utf8" });
  assert.equal(res.status, 2);
  assert.match(res.stderr, /usage: render\.mjs --props <file> \(--out <dir> \| --check \| --ids\)/);
});

test("the committed example props stay valid and render, one per mode at least", () => {
  const modes = new Set();
  for (const file of fs.readdirSync(path.join(SKILL, "examples")).filter((f) => f.endsWith(".props.json"))) {
    const props = JSON.parse(fs.readFileSync(path.join(SKILL, "examples", file), "utf8"));
    const r = render(null, { propsFile: path.join(SKILL, "examples", file) });
    assert.equal(r.status, 0, `${file}: ${r.stderr}`);
    modes.add(props.mode);
  }
  assert.deepEqual([...modes].sort(), ["plan", "ship"]);
});

test("validation: an unknown mode or unreadable JSON exits 1 with a clear reason", () => {
  const badMode = render({ mode: "video" });
  assert.equal(badMode.status, 1);
  assert.match(badMode.stderr, /\$\.mode: must be one of plan, ship/);
  const badJson = render("{ not json");
  assert.equal(badJson.status, 1);
  assert.match(badJson.stderr, /^\$: invalid JSON/);
});

// ---- the report page (index.html) ----

const { verdict } = await import(path.join(SKILL, "scripts", "report.mjs"));
const SHOOT = path.join(SKILL, "scripts", "shoot.mjs");
// A valid 1x1 PNG.
const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==", "base64");

function shots(html) {
  return [...html.matchAll(/data-shot="([a-z0-9-]+)"/g)].map((m) => m[1]);
}

function sectionTitles(html) {
  return [...html.matchAll(/<section class="sec" id="[a-z-]+" data-shot="[a-z-]+"><h2><span class="n">(\d+)<\/span>([^<]*)<\/h2>/g)].map((m) => `${m[1]} ${m[2]}`);
}

const sequence = (over = {}) => ({
  participants: [{ name: "Client", status: "existing" }, { name: "API", status: "new" }, { name: "Store", status: "changed" }],
  messages: [
    { from: "Client", to: "API", label: "GET /widgets", kind: "call" },
    { from: "API", to: "Store", label: "select *" },
    { from: "Store", to: "API", label: "rows", kind: "return" },
    { from: "API", to: "API", label: "paginate", kind: "self" },
  ],
  ...over,
});

test("report: both committed examples render every section with its data-shot id, in reading order", () => {
  const plan = render(null, { propsFile: path.join(SKILL, "examples", "plan-visual-explainer.props.json") });
  assert.equal(plan.status, 0, plan.stderr);
  assert.deepEqual(shots(plan.report), ["header", "tldr", "done-when", "phases", "architecture", "media", "decisions", "rabbit-holes"]);
  assert.deepEqual(sectionTitles(plan.report), ["1 Done when", "2 Phases", "3 Architecture: before / after", "4 Screenshots", "5 Decisions", "6 Rabbit holes &amp; no-gos"]);
  assert.match(plan.report, /<img src="data:image\/png;base64,[A-Za-z0-9+/=]+" alt="/, "the media file is inlined");
  assert.match(plan.report, /<figcaption>Before — a video per run<\/figcaption>/);
  assert.match(plan.report, /<figcaption>After — one shared renderer<\/figcaption>/);
  const ship = render(null, { propsFile: path.join(SKILL, "examples", "ship-ws-v6-m2.props.json") });
  assert.equal(ship.status, 0, ship.stderr);
  const props = JSON.parse(fs.readFileSync(path.join(SKILL, "examples", "ship-ws-v6-m2.props.json"), "utf8"));
  const cards = props.changes.map((c, i) => [`change-${i + 1}`, ...(c.sequence ? ["sequence"] : [])]).flat();
  assert.deepEqual(shots(ship.report), ["header", "tldr", "done-when", "changes", ...cards, "findings", "hand-test", "rollback"]);
  assert.deepEqual(sectionTitles(ship.report), ["1 Done when: planned vs delivered", "2 Changes, in reading order", "3 Findings", "4 Hand test", "5 Rollback"]);
  assert.match(ship.report, /<span class="add">\+33858<\/span> <span class="del">−749<\/span> across 186 files/);
  assert.match(ship.report, /<span class="pill ok">Ready to merge, 1 major finding<\/span>/);
  assert.match(ship.report, /<svg class="seq"/);
  assert.match(ship.report, /<span class="chip mode">mode: ship · PR review<\/span>/);
});

test("report (plan): TL;DR, Done when, phases, architecture panels, decision cards and rabbit holes show the props", () => {
  const props = planProps({
    meta: { title: "Widget effort plan", subtitle: "s", date: "2026-10-18", planId: "loop-widgets-2026-10-18-1200", facts: [{ label: "Route", value: "`/widgets`" }, { label: "Est.", value: "~2 days" }] },
    tldr: { goal: "Serve widgets.", ask: "approve `SQLite`.", risks: ["lock contention", "schema drift"] },
    doneWhen: [{ criterion: "GET returns 200", check: "`curl -f`" }],
    rabbitHoles: ["No writes in this milestone.", "Keep the API small."],
  });
  props.architecture.layers[0].components[0].status = "new";
  props.architecture.layers.push({ name: "Old", components: [{ name: "Legacy cron", role: "Copies files", status: "removed" }] });
  props.architecture.flows.push({ from: "Widget store", to: "Legacy cron", label: "export", status: "removed" });
  props.milestones[0].phases[0].summary = "tables and migrations";
  props.decisions[0].title = "SQLite store";
  const r = render(props);
  assert.equal(r.status, 0, r.stderr);
  const html = r.report;
  assert.match(html, /<span class="fact"><b>Route<\/b><code>\/widgets<\/code><\/span>/);
  assert.match(html, /<strong>What I need from you:<\/strong> approve <code>SQLite<\/code>\./);
  assert.match(html, /<strong>Top risks:<\/strong> ① lock contention; ② schema drift/);
  assert.match(html, /<th>Criterion<\/th><th>How we’ll check<\/th>/);
  assert.match(html, /<strong>Add the widget store<\/strong> — tables and migrations<\/span><span class="est">15-20 min<\/span>/);
  assert.match(html, /<strong>Review milestone 1<\/strong><span class="tag info">review · gate<\/span>/);
  assert.match(html, /<h3 class="mini">Milestone 2 — Write path<\/h3>/, "more than one milestone: a heading per group");
  const before = html.slice(html.indexOf("Before</figcaption>"), html.indexOf("After</figcaption>"));
  const after = html.slice(html.indexOf("After</figcaption>"));
  assert.ok(before.includes("Legacy cron") && !before.includes(">HTTP API<"), "before: no new component, the removed one");
  assert.ok(after.includes(">HTTP API<") && !after.slice(0, after.indexOf("</figure>")).includes("Legacy cron"), "after: the new component, no removed one");
  assert.match(html, /<span class="lg"><i class="sw st-new"><\/i>new<\/span>/);
  assert.match(html, /<div class="dc"><h3>SQLite store<\/h3><p class="k">DECISION<\/p><p>Use SQLite<\/p><p class="k">REJECTED<\/p><ul><li><s>Postgres<\/s><\/li><\/ul>/);
  assert.match(html, /<li><strong>No<\/strong> writes in this milestone\.<\/li><li>Keep the API small\.<\/li>/);
  const detail = html.slice(html.indexOf('<details class="more">'));
  for (const text of ["Lock contention", "WAL mode.", "loop-widgets-2026-10-18-1200", "acme/widgets", "Store tests pass", "A thin HTTP layer over a store."]) {
    assert.ok(detail.includes(text), `full detail shows ${text}`);
  }
});

test("report (plan): without tldr, doneWhen and rabbitHoles it falls back to the goal, top risks and phase doneWhen", () => {
  const fallback = planProps();
  fallback.architecture.layers[0].components[0].status = "new";
  const r = render(fallback);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.report, /<strong>Goal:<\/strong> Serve widgets over HTTP\./);
  assert.ok(!r.report.includes("What I need from you"));
  assert.match(r.report, /<strong>Top risks:<\/strong> ① Lock contention; ② Schema drift/, "highest impact first");
  assert.match(r.report, /<td data-label="Criterion">Add the widget store: Store tests pass<\/td><td data-label="How we’ll check">—<\/td>/);
  assert.ok(!shots(r.report).includes("rabbit-holes"));
  assert.match(r.report, /<figcaption>Before<\/figcaption>/, "default panel titles");
  const plain = planProps();
  for (const m of plain.milestones) for (const p of m.phases) delete p.doneWhen;
  const r2 = render(plain);
  assert.equal(r2.status, 0, r2.stderr);
  assert.ok(!shots(r2.report).includes("done-when"), "no Done when rows: no section");
  assert.match(r2.report, /<figcaption>Architecture<\/figcaption>/, "nothing changes: one panel");
  assert.deepEqual(sectionTitles(r2.report).map((t) => t.split(" ")[0]), ["1", "2", "3"], "numbers stay consecutive");
});

test("report (ship): changes, files, sequence, findings by severity, hand test and rollback", () => {
  const props = shipProps({
    stats: { files: 3, additions: 120, deletions: 4 },
    tldr: { changed: "A read API.", lookFirst: "`handler.js`", risk: "low" },
    changes: [
      { area: "Store", how: "Tables.", why: "Data lives somewhere.", files: [{ path: "src/store.js", additions: 80, deletions: 0, note: "new" }] },
      { area: "Read API", how: "A route.", why: "Clients read widgets.", sequence: sequence() },
    ],
  });
  props.reviews[0].findings = [
    { id: "F1", severity: "nit", title: "Typo", status: "open" },
    { id: "F2", severity: "major", title: "Slow query", status: "fixed", location: "src/store.js:4", detail: "Full scan.", fix: "Add an index." },
    { id: "F3", severity: "major", title: "No limit", status: "open" },
    { id: "F4", severity: "minor", title: "Log noise", status: "deferred" },
  ];
  const r = render(props);
  assert.equal(r.status, 0, r.stderr);
  const html = r.report;
  assert.match(html, /<strong>What changed:<\/strong> A read API\.<\/p><p><strong>Look at first:<\/strong> <code>handler\.js<\/code> <strong>Risk:<\/strong> low<\/p>/);
  assert.match(html, /<td class="st" data-label="Status"><span class="tag ok">met<\/span><\/td>/);
  assert.match(html, /<div class="chg" data-shot="change-1"><h3><span class="badge">1<\/span>Store<\/h3><p class="how">Tables\.<\/p><p class="why">Why: Data lives somewhere\.<\/p>/);
  assert.match(html, /<span class="p">src\/store\.js <span class="note">\(new\)<\/span><\/span><span class="d"><span class="add">\+80<\/span> <span class="del">−0<\/span><\/span>/);
  const card2 = html.slice(html.indexOf('data-shot="change-2"'));
  assert.match(card2, /^data-shot="change-2"[\s\S]*?<div data-shot="sequence"><div class="seq-box"><svg class="seq"/, "the sequence sits in its change card");
  assert.equal((html.match(/<g class="msg /g) ?? []).length, 4);
  assert.match(html, /<g class="msg ret">/);
  assert.match(html, /<g class="msg self"><path/);
  assert.match(html, /<g class="part st-new">/);
  const order = [...html.matchAll(/<span class="id">(F\d)<\/span>/g)].map((m) => m[1]);
  assert.deepEqual(order, ["F3", "F2", "F4", "F1"], "severity first, then open before fixed");
  assert.match(html, /<code class="loc">src\/store\.js:4<\/code>/);
  assert.match(html, /<p class="fix">Fix: Add an index\.<\/p>/);
  assert.match(html, /<span class="tag ok">fixed<\/span>/);
  assert.match(html, /<ol class="steps"><li><p>The server listens on port 3000\.<\/p><pre><code>npm start<\/code><\/pre><p class="ref">src\/server\.js:12<\/p><\/li><\/ol>/);
  assert.match(html, /<div class="callout"><p>Revert the merge commit\.<\/p><pre><code>git revert -m1 &lt;merge-sha&gt;<\/code><\/pre><\/div>/);
  const none = render(shipProps());
  assert.match(none.report, /<p class="empty">No findings\.<\/p>/);
  assert.match(none.report, /<strong>What changed:<\/strong> The widget service answers <code>GET \/widgets<\/code>\./, "fallback: the summary");
  assert.ok(!shots(none.report).includes("changes"), "no changes: no section");
});

test("report: the verdict pill counts open blockers, a REQUEST_CHANGES review, and the worst finding left", () => {
  const review = (verdictToken, findings) => ({ verdict: verdictToken, summary: "s", validation: [{ command: "t", result: "pass" }], findings });
  const f = (severity, status) => ({ severity, title: "t", status });
  const cases = [
    [[review("APPROVE", [])], { tone: "ok", text: "Ready to merge" }],
    [[review("APPROVE", [f("major", "fixed"), f("minor", "open")])], { tone: "ok", text: "Ready to merge, 1 minor finding" }],
    [[review("COMMENT", [f("nit", "open"), f("major", "deferred"), f("major", "open")])], { tone: "ok", text: "Ready to merge, 2 major findings" }],
    [[review("APPROVE", [f("blocker", "open"), f("blocker", "open"), f("blocker", "fixed")])], { tone: "bad", text: "Changes requested, 2 blocking" }],
    [[review("APPROVE", []), review("REQUEST_CHANGES", [f("minor", "open")])], { tone: "bad", text: "Changes requested" }],
    [[review("APPROVE", [f("blocker", "deferred")])], { tone: "ok", text: "Ready to merge, 1 blocker finding" }],
  ];
  for (const [reviews, want] of cases) assert.deepEqual(verdict({ reviews }), want);
  const r = render(shipProps({ reviews: [review("REQUEST_CHANGES", [f("blocker", "open")])] }));
  assert.match(r.report, /<span class="pill bad">Changes requested, 1 blocking<\/span>/);
});

test("report: sequence validation: one sequence at most, known and unique participants, self messages", () => {
  const r = render(shipProps({
    changes: [
      { area: "A", how: "h", why: "w", sequence: sequence({ participants: [{ name: "X" }, { name: "X" }], messages: [{ from: "X", to: "Y", label: "l" }, { from: "Z", to: "X", label: "l", kind: "self" }] }) },
      { area: "B", how: "h", why: "w", sequence: sequence() },
    ],
  }));
  assert.equal(r.status, 1);
  assert.deepEqual(r.stderr.trim().split("\n"), [
    "$.changes[1].sequence: only one change may have a sequence; $.changes[0] has one already",
    '$.changes[0].sequence.participants[1].name: "X" appears more than once',
    '$.changes[0].sequence.messages[0].to: no participant "Y"',
    '$.changes[0].sequence.messages[1].from: no participant "Z"',
    "$.changes[0].sequence.messages[1].to: a self message must have to equal to from",
  ]);
  const schema = render(shipProps({ changes: [{ area: "A", how: "h", why: "w", sequence: { participants: [{ name: "X", status: "removed" }], messages: [] } }] }));
  assert.equal(schema.status, 1);
  assert.match(schema.stderr, /\$\.changes\[0\]\.sequence\.participants: must have at least 2 item/);
  assert.match(schema.stderr, /\$\.changes\[0\]\.sequence\.participants\[0\]\.status: must be one of new, changed, existing/);
});

test("report: media files resolve against the props directory, must exist, and a change figure sits in its card", () => {
  const ok = render(shipProps({
    changes: [{ area: "Read API", how: "h", why: "w" }],
    media: [{ path: "shots/api.png", caption: "The `GET` response", change: "Read API" }, { path: "shots/list.png", caption: "The list" }],
  }), { files: { "shots/api.png": PNG, "shots/list.png": PNG } });
  assert.equal(ok.status, 0, ok.stderr);
  const card = ok.report.slice(ok.report.indexOf('data-shot="change-1"'), ok.report.indexOf('data-shot="media"'));
  assert.match(card, /<figure class="fig"><img src="data:image\/png;base64,iVBOR[^"]*" alt="The `GET` response"><figcaption>The <code>GET<\/code> response<\/figcaption><\/figure>/);
  assert.deepEqual(shots(ok.report).slice(3, 6), ["changes", "change-1", "media"], "the other figures follow the changes");
  const bad = render(shipProps({
    media: [
      { path: "missing.png", caption: "c" },
      { path: "a/../b.png", caption: "c" },
      { path: "/etc/x.png", caption: "c" },
      { path: "shot.svg", caption: "c" },
      { path: "ok.png", caption: "c", change: "Nope" },
    ],
  }), { files: { "ok.png": PNG } });
  assert.equal(bad.status, 1);
  const lines = bad.stderr.trim().split("\n");
  assert.deepEqual(lines.map((l) => l.split(": ")[0]), ["$.media[2].path", "$.media[3].path"], "the schema rejects absolute paths and other types first");
  const refs = render(shipProps({ media: [{ path: "missing.png", caption: "c" }, { path: "a/../b.png", caption: "c" }, { path: "ok.png", caption: "c", change: "Nope" }] }), { files: { "ok.png": PNG } });
  assert.equal(refs.status, 1);
  assert.match(refs.stderr, /^\$\.media\[0\]\.path: no file at .*missing\.png$/m);
  assert.match(refs.stderr, /^\$\.media\[1\]\.path: must not have a "\.\." segment$/m);
  assert.match(refs.stderr, /^\$\.media\[2\]\.change: no change with the area "Nope"$/m);
  const plan = render(planProps({ media: [{ path: "p.png", caption: "c", change: "x" }] }), { files: { "p.png": PNG } });
  assert.equal(plan.status, 1);
  assert.match(plan.stderr, /\$\.media\[0\]\.change: unknown field/, "plan media has no change");
});

test("report escaping: props text in the new fields cannot inject markup", () => {
  const evil = `<script>alert(1)</script>"><img src=x onerror=alert(2)>`;
  const props = shipProps({
    meta: { title: "Evil ship report", subtitle: evil, date: "2026-10-18", facts: [{ label: evil.slice(0, 40), value: evil }] },
    tldr: { changed: evil, lookFirst: `\`</code>${evil}\``, risk: evil },
    changes: [{
      area: evil, how: evil, why: evil, files: [{ path: evil, additions: 1, deletions: 2, note: evil }],
      sequence: { participants: [{ name: evil.slice(0, 40) }, { name: "B" }], messages: [{ from: evil.slice(0, 40), to: "B", label: evil.slice(0, 80) }] },
    }],
  });
  props.reviews[0].findings = [{ severity: "major", title: evil, location: evil, detail: evil, fix: evil, status: "open" }];
  props.reviews[0].split = evil;
  const plan = planProps({ tldr: { goal: evil, ask: evil, risks: [evil] }, doneWhen: [{ criterion: evil, check: evil }], rabbitHoles: [`No ${evil}`] });
  plan.decisions[0].title = evil;
  plan.architecture.beforeTitle = evil;
  plan.architecture.layers[0].components[0].status = "new";
  for (const r of [render(props), render(plan)]) {
    assert.equal(r.status, 0, r.stderr);
    assert.equal((r.report.match(/<script/g) ?? []).length, 0, "the report has no script at all");
    assert.ok(!/<img src=x/i.test(r.report), "no injected element");
    assert.ok(r.report.includes("&lt;script&gt;alert(1)&lt;/script&gt;"), "the hostile text is shown, escaped");
    assert.ok(!/\{\{[A-Z_]+\}\}/.test(r.report), "every template slot is filled");
  }
});

test("report: the walkthrough bar sits right after the TL;DR, and screenshots hide it", () => {
  const narration = path.join(fs.mkdtempSync(path.join(TMP, "narration-")), "narration.json");
  const text = Array.from({ length: 45 }, () => "word").join(" ");
  fs.writeFileSync(narration, JSON.stringify({ version: 3, mode: "ship", scenes: [{ scene: "intro", text }, { scene: "outro", text }] }));
  const r = render(shipProps(), { args: ["--narration", narration] });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.report, /<\/section>\n<p class="wt-bar"><button type="button" class="wt-start" data-walkthrough-start>Start the walkthrough<\/button><span class="wt-hint">2 steps/);
  assert.ok(r.report.indexOf('data-shot="tldr"') < r.report.indexOf('class="wt-bar"'));
  assert.match(r.report, /\.shot \.wt-bar \{ display: none; \}/, "screenshots hide the walkthrough bar");
  assert.ok(!render(shipProps()).report.includes('class="wt-bar"'));
});

for (const mode of ["plan", "ship"]) {
  test(`report contract (${mode}): self-contained, themed, phone-safe, motion-aware`, () => {
    const r = render(mode === "plan" ? planProps() : shipProps());
    assert.equal(r.status, 0, r.stderr);
    const html = r.report;
    assert.match(html, /<meta name="viewport" content="width=device-width, initial-scale=1">/);
    assert.match(html, /:root \{[^}]*--bg:[^}]*--surface:/, "color tokens on :root");
    assert.match(html, /@media \(prefers-color-scheme: dark\) \{\s*:root:not\(\[data-theme="light"\]\) \{[^}]*color-scheme: dark;[^}]*--bg:/, "dark tokens under the media query");
    assert.match(html, /:root\[data-theme="dark"\] \{[^}]*--bg:/, "dark tokens for an explicit dark theme");
    assert.match(html, /\nbody \{[^}]*background: var\(--bg\);/, "explicit body background");
    assert.match(html, /@media \(max-width: 640px\) \{[^@]*\.card \{[^}]*padding: 20px 16px 24px;/, "16px side gutter on a phone");
    assert.match(html, /\.tbl \{ overflow-x: auto;/, "tables scroll inside their box");
    assert.match(html, /\.seq-box \{[^}]*overflow-x: auto;/, "the sequence scrolls inside its box");
    assert.match(html, /\.arch-panel \{[^}]*overflow-x: auto;/, "the architecture scrolls inside its box");
    assert.match(html, /\.changes \{ display: grid; grid-template-columns: minmax\(0, 1fr\);/, "a wide card cannot widen the page");
    assert.match(html, /pre \{[^}]*overflow-x: auto;/);
    assert.match(html, /@media \(prefers-reduced-motion: no-preference\)/, "motion only when allowed");
    assert.ok(!/animation|transition/.test(html.slice(0, html.indexOf("@media (prefers-reduced-motion"))), "no motion outside the media query");
    assert.ok(!/<script[^>]+src=|<link\b|@import|url\(\s*["']?https?:/i.test(html), "no external scripts, styles or assets");
  });
}

test("report: a Done when table with more than 8 rows shows the rows that are not met first, the rest in a <details>", () => {
  const row = (i, status) => ({ criterion: `C${i}`, result: `R${i}`, status });
  const delivered = Array.from({ length: 12 }, (_, i) => row(i + 1, "met"));
  delivered[4].status = "partial";
  delivered[10].status = "missed";
  const r = render(shipProps({ delivered }));
  assert.equal(r.status, 0, r.stderr);
  const section = r.report.slice(r.report.indexOf('data-shot="done-when"'), r.report.indexOf("</section>", r.report.indexOf('data-shot="done-when"')));
  assert.match(section, /<p class="counts"><span class="tag bad">1 missed<\/span> <span class="tag warn">1 partial<\/span> <span class="tag ok">10 met<\/span><\/p>/);
  const [shown, rest] = section.split('<details class="more-rows">');
  assert.deepEqual([...shown.matchAll(/>(C\d+)</g)].map((m) => m[1]), ["C11", "C5", "C1", "C2", "C3", "C4", "C6", "C7"]);
  assert.match(rest, /^<summary>4 more met criteria<\/summary>/);
  assert.deepEqual([...rest.matchAll(/>(C\d+)</g)].map((m) => m[1]), ["C8", "C9", "C10", "C12"]);
  const eight = render(shipProps({ delivered: delivered.slice(0, 8) }));
  assert.ok(!eight.report.includes('<details class="more-rows">') && !eight.report.includes('<p class="counts">'), "8 rows: the full table");
  const many = render(shipProps({ delivered: Array.from({ length: 10 }, (_, i) => row(i + 1, "partial")) }));
  assert.equal((many.report.match(/<td class="st" data-label="Status">/g) ?? []).length, 10, "all rows that are not met stay visible");
  assert.ok(!many.report.includes('<details class="more-rows">'));
  const plan = render(planProps({ doneWhen: Array.from({ length: 10 }, (_, i) => ({ criterion: `P${i + 1}`, check: "c" })) }));
  assert.match(plan.report, /<details class="more-rows"><summary>2 more criteria<\/summary>/);
  assert.ok(!plan.report.includes('<p class="counts">'), "plan rows have no status");
});

test("report: a hand test with more than 8 steps is compact: title, command, and one muted line", () => {
  const steps = Array.from({ length: 9 }, (_, i) => ({ title: `Step ${i + 1}`, command: `cmd ${i + 1}`, ref: `src/a.js:${i + 1}`, why: `Why ${i + 1}.`, ...(i === 0 ? { expect: "`ok`" } : {}) }));
  const r = render(shipProps({ handTest: steps }));
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.report, /<ol class="steps compact"><li><p><strong>Step 1<\/strong><\/p><pre><code>cmd 1<\/code><\/pre><p class="aside">Why 1\. · Expect: <code>ok<\/code> · <span class="mono">src\/a\.js:1<\/span><\/p><\/li>/);
  assert.equal((r.report.match(/<p class="aside">/g) ?? []).length, 9);
  const short = render(shipProps({ handTest: steps.slice(0, 8) }));
  assert.ok(short.report.includes('<ol class="steps"><li><p><strong>Step 1.</strong> Why 1.</p>'), "8 steps: the full form");
});

test("report: each architecture arrowhead has the color of its line", () => {
  const props = planProps();
  props.architecture.flows[0].status = "new";
  props.architecture.flows.push({ from: "Widget store", to: "HTTP API", label: "old push", status: "removed" });
  const r = render(props);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.report, /<path class="flow fs-new" d="[^"]+" marker-end="url\(#ve-ah-after-new\)">/);
  assert.match(r.report, /<path class="flow fs-removed" d="[^"]+" marker-end="url\(#ve-ah-before-removed\)">/);
  assert.match(r.report, /<marker id="ve-ah-after-new"[^>]*><path class="ah fs-new"/);
  assert.match(r.report, /\.ah\.fs-new \{ fill: var\(--ok\); \}/);
});

const CHECKPOINT = path.join(import.meta.dirname, "fixtures", "visual-explainer-checkpoint.props.json");

test("checkpoint: the fixture renders a progress header, pending rows and no review, with the same data-shot ids", () => {
  const r = render(null, { propsFile: CHECKPOINT });
  assert.equal(r.status, 0, r.stderr);
  assert.ok(r.html, "the graph page renders with no review");
  const html = r.report;
  assert.match(html, /<span class="chip mode">mode: ship · checkpoint<\/span>/);
  assert.match(html, /<span class="fact"><b>Since<\/b>2026-10-20 22:30<\/span><span class="pill info">Checkpoint 1 · 5 of 12 phases done<\/span>/);
  assert.match(html, /<span class="progress" role="progressbar" aria-label="Phases done" aria-valuemin="0" aria-valuemax="12" aria-valuenow="5"><i style="width: 42%"><\/i><\/span>/);
  assert.ok(!html.includes('<span class="pill ok">') && !html.includes('<span class="pill bad">'), "no verdict pill");
  assert.match(html, /<p class="counts"><span class="tag warn">1 partial<\/span> <span class="tag">7 pending<\/span> <span class="tag ok">4 met<\/span><\/p>/);
  assert.match(html, /<p class="empty">No review yet\. The milestone review runs at the gate\.<\/p>/);
  assert.deepEqual(shots(html), ["header", "tldr", "done-when", "changes", "change-1", "change-2", "sequence", "findings", "hand-test", "rollback"]);
});

test("checkpoint: only a checkpoint may have no review; phasesDone is at most phasesTotal; since is a time", () => {
  const noReview = render(shipProps({ reviews: [] }));
  assert.equal(noReview.status, 1);
  assert.equal(noReview.stderr.trim(), "$.reviews: must have at least 1 item (only a checkpoint report, with meta.checkpoint, may have none)");
  const meta = (checkpoint) => ({ title: "Widget ship report", subtitle: "s", date: "2026-10-18", checkpoint });
  const ok = render(shipProps({ meta: meta({ number: 2, phasesDone: 3, phasesTotal: 9 }), reviews: [] }));
  assert.equal(ok.status, 0, ok.stderr);
  const bad = render(shipProps({ meta: meta({ number: 0, phasesDone: 1, phasesTotal: 4, since: "yesterday" }) }));
  assert.equal(bad.status, 1);
  assert.deepEqual(bad.stderr.trim().split("\n").map((l) => l.split(":")[0]), ["$.meta.checkpoint.number", "$.meta.checkpoint.since"]);
  const over = render(shipProps({ meta: meta({ number: 1, phasesDone: 5, phasesTotal: 4 }) }));
  assert.equal(over.status, 1);
  assert.equal(over.stderr.trim(), "$.meta.checkpoint.phasesDone: must be <= phasesTotal (4)");
});

test("checkpoint: pending rows follow missed and partial; more than 8 pending rows collapse with the met rows", () => {
  const row = (i, status) => ({ criterion: `C${i}`, result: `R${i}`, status });
  const delivered = [row(1, "met"), row(2, "partial"), ...Array.from({ length: 10 }, (_, i) => row(i + 3, "pending")), row(13, "missed"), row(14, "met")];
  const r = render(shipProps({ meta: { title: "Widget ship report", subtitle: "s", date: "2026-10-18", checkpoint: { number: 1, phasesDone: 2, phasesTotal: 14 } }, reviews: [], delivered }));
  assert.equal(r.status, 0, r.stderr);
  const section = r.report.slice(r.report.indexOf('data-shot="done-when"'), r.report.indexOf("</section>", r.report.indexOf('data-shot="done-when"')));
  const [shown, rest] = section.split('<details class="more-rows">');
  assert.deepEqual([...shown.matchAll(/>(C\d+)</g)].map((m) => m[1]), ["C13", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9", "C10"]);
  assert.match(rest, /^<summary>4 more criteria: 2 pending · 2 met<\/summary>/);
  assert.deepEqual([...rest.matchAll(/>(C\d+)</g)].map((m) => m[1]), ["C11", "C12", "C1", "C14"]);
  assert.match(section, /<span class="tag bad">1 missed<\/span> <span class="tag warn">1 partial<\/span> <span class="tag">10 pending<\/span> <span class="tag ok">2 met<\/span>/);
});

// The shoot test needs a Chrome binary: it skips without one.
const chrome = findChrome();
test("shoot: one PNG for the page and one for each data-shot section, light and dark", { skip: chrome ? false : "no Chrome binary found" }, () => {
  const r = render(shipProps({ changes: [{ area: "Read API", how: "h", why: "w", sequence: sequence() }] }));
  assert.equal(r.status, 0, r.stderr);
  for (const theme of ["light", "dark"]) {
    const out = path.join(r.dir, `shots-${theme}`);
    const res = spawnSync(process.execPath, [SHOOT, "--page", r.page, "--out", out, "--chrome", chrome, "--theme", theme, "--scale", "1", "--width", "390"], { encoding: "utf8", timeout: 90_000 });
    assert.equal(res.status, 0, res.stderr);
    const files = res.stdout.trim().split("\n");
    assert.deepEqual(files.map((f) => path.basename(f)), ["page.png", ...shots(r.report).map((id) => `${id}.png`)]);
    for (const file of files) {
      assert.ok(path.isAbsolute(file));
      const png = fs.readFileSync(file);
      assert.equal(png.subarray(1, 4).toString(), "PNG");
      if (path.basename(file) === "page.png") assert.equal(png.readUInt32BE(16), 390, "no horizontal page scroll at phone width");
    }
  }
});

test("shoot: an explicit --chrome that fails exits 3 at once", () => {
  const r = render(shipProps());
  assert.equal(r.status, 0, r.stderr);
  const broken = path.join(r.dir, "broken-chrome");
  fs.writeFileSync(broken, "#!/bin/sh\nexit 1\n", { mode: 0o755 });
  const res = spawnSync(process.execPath, [SHOOT, "--page", r.page, "--out", path.join(r.dir, "s"), "--chrome", broken], { encoding: "utf8" });
  assert.equal(res.status, 3);
  assert.match(res.stderr, /exited \(code 1\) before it opened the DevTools port/);
});

test("shoot: Chrome uses a mock keychain, so a sandbox HOME shows no keychain dialog", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ve-keychain-"));
  const argv = path.join(dir, "argv.txt");
  const fake = path.join(dir, "chrome");
  fs.writeFileSync(fake, `#!/bin/sh\nprintf '%s\\n' "$@" > '${argv}'\nexit 1\n`, { mode: 0o755 });
  const res = spawnSync(process.execPath, [SHOOT, "--page", CLI, "--out", path.join(dir, "s"), "--chrome", fake], { encoding: "utf8" });
  assert.equal(res.status, 3, res.stderr);
  const args = fs.readFileSync(argv, "utf8").split("\n");
  assert.ok(args.includes("--use-mock-keychain"), args.join(" "));
  assert.ok(args.includes("--password-store=basic"), args.join(" "));
});

test("shoot: usage errors exit 2, a missing browser exits 3", () => {
  assert.equal(spawnSync(process.execPath, [SHOOT], { encoding: "utf8" }).status, 2);
  assert.equal(spawnSync(process.execPath, [SHOOT, "--page", CLI, "--out", TMP, "--theme", "sepia"], { encoding: "utf8" }).status, 2);
  const res = spawnSync(process.execPath, [SHOOT, "--page", CLI, "--out", TMP, "--chrome", path.join(TMP, "no-chrome")], { encoding: "utf8" });
  assert.equal(res.status, 3);
  assert.match(res.stderr, /no Chrome found/);
});

// The same discovery order as shoot.mjs, without the flag.
function findChrome() {
  const candidate = process.env.VE_CHROME || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  return fs.existsSync(candidate) ? candidate : null;
}
