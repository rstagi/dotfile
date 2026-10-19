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

function render(props, { args = [], narration } = {}) {
  const dir = fs.mkdtempSync(path.join(TMP, "case-"));
  const propsFile = path.join(dir, "props.json");
  fs.writeFileSync(propsFile, typeof props === "string" ? props : JSON.stringify(props));
  if (narration !== undefined) fs.writeFileSync(path.join(dir, "narration.json"), typeof narration === "string" ? narration : JSON.stringify(narration));
  const out = path.join(dir, "out");
  const target = args.includes("--check") ? [] : ["--out", out];
  const res = spawnSync(process.execPath, [CLI, "--props", propsFile, ...target, ...args.map((a) => a.replace("<dir>", dir))], { encoding: "utf8" });
  const page = path.join(out, "index.html");
  return { ...res, dir, page, html: fs.existsSync(page) ? fs.readFileSync(page, "utf8") : null };
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

test("ship: valid props write one index.html and exit 0", () => {
  const r = render(shipProps());
  assert.equal(r.status, 0, r.stderr);
  assert.ok(r.html, "index.html exists");
  assert.match(r.html, /<title>Widget ship report<\/title>/);
  assert.match(r.stdout.trim(), new RegExp(`${r.page.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`));
});

test("ship: a hand-test step without a code reference is rejected with its field path", () => {
  const r = render(shipProps({ handTest: [{ command: "npm start", why: "The server listens." }] }));
  assert.equal(r.status, 1);
  assert.match(r.stderr, /\$\.handTest\[0\]\.ref: required/);
  assert.equal(r.html, null, "no page is written for invalid props");
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
  const r = render(props, { narration: { version: 2, mode: "ship", stops: [{ focus: ["all"], title: evil, text: evil }] } });
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
  assert.equal(data.stops[0].title, evil, "the data block keeps the text, with no markup in the page source");
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
    const r = render(props);
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
