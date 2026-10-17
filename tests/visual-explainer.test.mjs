import { after, test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SKILL = path.join(import.meta.dirname, "..", ".claude", "skills", "visual-explainer");
const CLI = path.join(SKILL, "scripts", "render.mjs");
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

function render(props, { args = [] } = {}) {
  const dir = fs.mkdtempSync(path.join(TMP, "case-"));
  const propsFile = path.join(dir, "props.json");
  fs.writeFileSync(propsFile, typeof props === "string" ? props : JSON.stringify(props));
  const out = path.join(dir, "out");
  const target = args.includes("--check") ? [] : ["--out", out];
  const res = spawnSync(process.execPath, [CLI, "--props", propsFile, ...target, ...args], { encoding: "utf8" });
  const page = path.join(out, "index.html");
  return { ...res, dir, page, html: fs.existsSync(page) ? fs.readFileSync(page, "utf8") : null };
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

test("ship: renders every section in order with the props content", () => {
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
  const order = ["Outcome", "Delivered", "Deviations", "Review", "Hand test", "Rollback"]
    .map((title) => r.html.indexOf(`<h2>${title}</h2>`));
  assert.ok(order.every((i) => i > 0), `all sections present: ${order}`);
  assert.deepEqual([...order].sort((a, b) => a - b), order, "sections keep their order");
  for (const text of [
    "GET /widgets returns 200", "F4 moved to Phase 71", "One major finding stays open.",
    "Window lookup fails under load", "212 tests", "git revert -m1 &lt;merge-sha&gt;",
  ]) assert.ok(r.html.includes(text), `page shows ${text}`);
  assert.match(r.html, /<code>GET \/widgets<\/code>/, "backticks become inline code");
  assert.match(r.html, /href="https:\/\/github\.com\/acme\/widgets\/pull\/7#pullrequestreview-1"/);
  const dry = r.html.indexOf("./wsd migrate --dry-run");
  const install = r.html.indexOf("./install.sh ws");
  assert.ok(dry > 0 && install > dry, "hand-test steps keep their order");
  assert.ok(r.html.includes("wsd/src/import.rs:50") && r.html.includes("install.sh:412"), "each step shows its code reference");
  assert.ok(r.html.indexOf(">F4<") < r.html.indexOf(">F2<"), "the major finding comes before the minor one");
});

test("escaping: props text cannot inject HTML, script or template slots", () => {
  const evil = `<script>alert(1)</script>"><img src=x onerror=alert(2)>`;
  const props = shipProps({
    meta: { title: "Evil <b>ship</b> report", subtitle: `${evil} $& $' {{SCENES}}`, date: "2026-10-18" },
    summary: `Run \`</code><script>alert(3)</script>\` now`,
    deviations: [{ planned: evil, actual: "x' onmouseover='alert(4)", reason: "{{HERO}}" }],
    handTest: [{ command: `echo "${evil}"`, ref: "src/a.js:1", why: evil, expect: evil }],
  });
  props.reviews[0].findings = [{ id: "F1", severity: "major", title: evil, location: evil, detail: evil, fix: evil, status: "open" }];
  props.reviews[0].validation = [{ command: evil, result: "fail", note: evil }];
  const r = render(props);
  assert.equal(r.status, 0, r.stderr);
  assert.equal((r.html.match(/<script/g) ?? []).length, 2, "only the two template scripts exist");
  assert.ok(!/<img/i.test(r.html), "no injected element");
  assert.ok(!/<b>ship<\/b>/.test(r.html), "markup in a title stays text");
  assert.ok(!r.html.includes("' onmouseover='"), "quotes cannot break out of an attribute or text");
  assert.ok(r.html.includes("&lt;script&gt;alert(1)&lt;/script&gt;"), "the hostile text is shown, escaped");
  assert.ok(r.html.includes("&lt;/code&gt;&lt;script&gt;alert(3)&lt;/script&gt;"), "inline code content is escaped");
  assert.ok(r.html.includes("$&amp; $&#39; {{SCENES}}"), "replacement patterns and slot names stay literal");
  assert.ok(r.html.includes("{{HERO}}"), "a slot name inside props is never expanded");
});

test("escaping: link URLs must be http or https", () => {
  const props = shipProps({ meta: { title: "Link test page", subtitle: "s", date: "2026-10-18", links: [{ label: "x", url: "javascript:alert(1)" }] } });
  const r = render(props);
  assert.equal(r.status, 1);
  assert.match(r.stderr, /\$\.meta\.links\[0\]\.url: must match .*http or https URL/);
});

test("plan: renders goal, architecture, timeline, decisions and risks in order", () => {
  const r = render(planProps());
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.html, /<title>Widget effort plan<\/title>/);
  assert.match(r.html, /<body class="mode-plan">/);
  const order = ["Goal", "Architecture", "Timeline", "Decisions", "Risks"].map((t) => r.html.indexOf(`<h2>${t}</h2>`));
  assert.ok(order.every((i) => i > 0), `all sections present: ${order}`);
  assert.deepEqual([...order].sort((a, b) => a - b), order, "sections keep their order");
  for (const text of [
    "Serve widgets over HTTP.", "Widget store", "reads widgets", "Read path", "Widgets are readable.",
    "Add the widget store", "Store tests pass", "15-20 min", "acme/widgets", "Postgres",
    "Needs a server for a local tool.", "WAL mode.", "loop-widgets-2026-10-18-1200",
  ]) assert.ok(r.html.includes(text), `page shows ${text}`);
  const phases = ["Add the widget store", "Serve GET /widgets", "Review milestone 1", "Accept POST /widgets"].map((t) => r.html.indexOf(t));
  assert.deepEqual([...phases].sort((a, b) => a - b), phases, "phases keep plan order");
  assert.match(r.html, /merge gate/i, "the gate phase is marked");
  assert.ok(r.html.indexOf("Lock contention") < r.html.indexOf("Schema drift"), "high-impact risks come first");
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
    assert.ok(!/<script[^>]+src=|<link\b|@import|url\(\s*["']?https?:/i.test(r.html), "no external scripts, styles or assets");
    assert.ok(!/\{\{[A-Z]+\}\}/.test(r.html), "every template slot is filled");
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
  assert.match(res.stderr, /usage: render\.mjs --props <file> \(--out <dir> \| --check\)/);
});

test("the committed example props stay valid and render, one per mode at least", () => {
  const modes = new Set();
  for (const file of fs.readdirSync(path.join(SKILL, "examples"))) {
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
