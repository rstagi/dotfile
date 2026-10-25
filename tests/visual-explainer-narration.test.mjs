import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SKILL = path.join(import.meta.dirname, "..", ".claude", "skills", "visual-explainer");
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "visual-explainer-narration-"));
// Not a root after() hook: with --test-name-pattern, node runs it before a long filtered test ends.
process.on("exit", () => fs.rmSync(TMP, { recursive: true, force: true }));

const EXAMPLES = Object.fromEntries(fs.readdirSync(path.join(SKILL, "examples")).filter((f) => f.endsWith(".props.json")).map((file) => {
  const props = JSON.parse(fs.readFileSync(path.join(SKILL, "examples", file), "utf8"));
  return [props.mode, props];
}));
const EXAMPLE_NARRATION_FILE = Object.fromEntries(Object.keys(EXAMPLES).map((mode) => [mode, path.join(SKILL, "examples",
  fs.readdirSync(path.join(SKILL, "examples")).find((f) => f.startsWith(mode) && f.endsWith(".narration.json")))]));
// The example props file beside each narration file: props.media paths are relative to it.
const EXAMPLE_PROPS_FILE = Object.fromEntries(Object.entries(EXAMPLE_NARRATION_FILE).map(([mode, file]) => [mode, file.replace(".narration.json", ".props.json")]));

const planProps = (over = {}) => ({
  mode: "plan",
  meta: { title: "Widget effort plan", subtitle: "Two milestones, one lane", date: "2026-10-18" },
  goal: "Serve widgets over HTTP.",
  architecture: {
    summary: "A thin HTTP layer over a store.",
    layers: [
      { name: "Interface", components: [{ name: "HTTP API", role: "Routes `GET /widgets`", status: "new" }] },
      { name: "Storage", components: [{ name: "Widget store", role: "SQLite tables" }, { name: "Flat files", role: "Old store", status: "removed" }] },
    ],
    flows: [{ from: "HTTP API", to: "Widget store", label: "reads widgets" }],
  },
  milestones: [
    {
      number: 1, title: "Read path",
      phases: [
        { number: 1, title: "Add the widget store", kind: "work", estimate: "1 d", doneWhen: "The store passes its tests." },
        { number: 2, title: "Review milestone 1", kind: "pr-review", gate: true, dependsOn: [1] },
      ],
    },
    { number: 2, title: "Write path", phases: [{ number: 3, title: "Accept POST /widgets", kind: "work", summary: "one handler" }] },
  ],
  decisions: [{ decision: "Use SQLite for the widget store", why: "One file, no server.", rejected: [{ option: "Postgres", reason: "Needs a server." }] }],
  risks: [
    { risk: "Slow queries", impact: "low", mitigation: "Indexes." },
    { risk: "Lock contention", impact: "high", mitigation: "WAL mode." },
    { risk: "Disk full", impact: "medium", mitigation: "Alerts." },
  ],
  ...over,
});

const shipProps = (over = {}) => ({
  mode: "ship",
  meta: { title: "Widget ship report", subtitle: "Milestone 1 of the widget effort", date: "2026-10-18" },
  summary: "The widget service answers `GET /widgets`.",
  delivered: [{ criterion: "GET /widgets returns 200", result: "Handler and route added", status: "met" }],
  deviations: [],
  reviews: [{ verdict: "APPROVE", summary: "No open findings.", validation: [{ command: "npm test", result: "pass" }], findings: [] }],
  handTest: [{ command: "npm start", ref: "src/server.js:12", why: "The server listens on port 3000." }],
  rollback: [{ step: "Revert the merge commit.", command: "git revert -m1 <merge-sha>" }],
  ...over,
});

// A mid-milestone progress checkpoint: no review yet, some delivered rows still pending.
const checkpointProps = () => shipProps({
  meta: { title: "Widget ship report", subtitle: "Checkpoint", date: "2026-10-18", checkpoint: { number: 2, phasesDone: 3, phasesTotal: 5 } },
  reviews: [],
  delivered: [
    { criterion: "a", result: "ra", status: "pending" }, { criterion: "b", result: "rb", status: "met" },
    { criterion: "c", result: "rc", status: "partial" }, { criterion: "d", result: "rd", status: "pending" },
  ],
});

// Narration helpers: a text of `n` words, and a v3 narration of the given scenes.
const words = (n, word = "word") => Array.from({ length: n }, () => word).join(" ");
const scene = (id, n = 20) => ({ scene: id, text: words(n) });
const v3 = (mode, scenes) => ({ version: 3, mode, scenes });
const introOnly = (mode) => v3(mode, [scene("intro", 80)]);

const RENDER = path.join(SKILL, "scripts", "render.mjs");

function run(script, args, { props, narration } = {}) {
  const dir = fs.mkdtempSync(path.join(TMP, "case-"));
  const propsFile = path.join(dir, "props.json");
  const narrationFile = path.join(dir, "narration.json");
  if (props !== undefined) fs.writeFileSync(propsFile, JSON.stringify(props));
  if (narration !== undefined) fs.writeFileSync(narrationFile, typeof narration === "string" ? narration : JSON.stringify(narration));
  const argv = args.map((a) => a.replace("<props>", propsFile).replace("<narration>", narrationFile).replace("<out>", path.join(dir, "out")));
  const res = spawnSync(process.execPath, [script, ...argv], { encoding: "utf8" });
  const report = path.join(dir, "out", "index.html");
  return { ...res, dir, report: fs.existsSync(report) ? fs.readFileSync(report, "utf8") : null };
}
// The walkthrough steps that the report embeds as JSON.
function steps(report) {
  const m = report.match(/<script type="application\/json" id="ve-walkthrough">([\s\S]*?)<\/script>/);
  return m ? JSON.parse(m[1]) : null;
}

// ---- narration v3: one commentary text for each walkthrough step ----

const NARRATION = path.join(SKILL, "scripts", "narration.mjs");
const { checkNarration, countWords, hasSceneData, SCENES, tourStops } = await import(NARRATION);

function narrationFile(narration) {
  const file = path.join(fs.mkdtempSync(path.join(TMP, "narration-")), "narration.json");
  fs.writeFileSync(file, typeof narration === "string" ? narration : JSON.stringify(narration));
  return file;
}
const check = (narration, props) => checkNarration(narrationFile(narration), props);
const lines = (errors) => errors.map((e) => `${e.path}: ${e.message}`);

test("narration: the committed examples are valid for their props, concise, and in scene order", () => {
  for (const [mode, props] of Object.entries(EXAMPLES)) {
    const { errors, scenes } = checkNarration(EXAMPLE_NARRATION_FILE[mode], props);
    assert.deepEqual(errors, [], mode);
    const order = scenes.map((s) => SCENES[mode].indexOf(s.id));
    assert.deepEqual(order, [...order].sort((a, b) => a - b), `${mode}: canonical order`);
    const total = scenes.reduce((n, s) => n + countWords(s.text), 0);
    assert.ok(total >= 150 && total <= 380, `${mode}: ${total} words, about 60 to 150 s`);
  }
});

test("narration: a valid narration resolves to its scenes in walkthrough order", () => {
  const { errors, scenes } = check(v3("plan", [scene("intro"), scene("phases"), scene("decisions"), scene("outro")]), planProps());
  assert.deepEqual(errors, []);
  assert.deepEqual(scenes.map((s) => s.id), ["intro", "phases", "decisions", "outro"]);
  assert.equal(scenes[0].text, words(20));
});

test("narration: each scene id must belong to the mode, appear once, follow the scene order, and intro is required", () => {
  const { errors } = check(v3("ship", [scene("tldr"), scene("phases"), scene("rollback"), scene("hand-test"), scene("rollback")]), shipProps());
  assert.deepEqual(lines(errors), [
    'narration.scenes[1].scene: "phases" is not a ship scene (intro, tldr, done-when, changes, findings, hand-test, rollback, outro)',
    'narration.scenes[3].scene: "hand-test" must come before "rollback" (scene order: intro, tldr, done-when, changes, findings, hand-test, rollback, outro)',
    'narration.scenes[4].scene: "rollback" appears more than once',
    'narration.scenes: must have the "intro" scene',
  ]);
});

test("narration: a scene whose props data is empty is rejected with the field path of that data", () => {
  const ship = check(v3("ship", [scene("intro", 30), scene("changes", 30), scene("findings", 30)]), shipProps());
  assert.deepEqual(lines(ship.errors), [
    'narration.scenes[1].scene: the props have no data for "changes" ($.changes)',
    'narration.scenes[2].scene: the props have no data for "findings" ($.reviews[].findings)',
  ]);
  const plan = check(v3("plan", [scene("intro", 40), scene("rabbit-holes", 40)]), planProps());
  assert.deepEqual(lines(plan.errors), ['narration.scenes[1].scene: the props have no data for "rabbit-holes" ($.rabbitHoles)']);
  const withData = planProps({ rabbitHoles: ["No mobile layout."] });
  assert.deepEqual(check(v3("plan", [scene("intro", 40), scene("rabbit-holes", 40)]), withData).errors, []);
  assert.equal(hasSceneData(planProps(), "done-when"), true, "done-when falls back to the doneWhen of the first milestone's phases");
  const noDoneWhen = planProps();
  delete noDoneWhen.milestones[0].phases[0].doneWhen;
  assert.equal(hasSceneData(noDoneWhen, "done-when"), false);
  assert.equal(hasSceneData(shipProps(), "outro"), true, "intro, tldr and outro always have data");
});

test("narration: a progress checkpoint has no review yet, so a findings scene is rejected; the other ship scenes stay valid", () => {
  const props = checkpointProps();
  const bad = check(v3("ship", [scene("intro", 30), scene("done-when", 30), scene("findings", 30)]), props);
  assert.deepEqual(lines(bad.errors), ['narration.scenes[2].scene: the props have no data for "findings" ($.reviews[].findings): a progress checkpoint has no review yet, omit the scene']);
  assert.deepEqual(check(v3("ship", [scene("intro", 30), scene("done-when", 30), scene("hand-test", 30), scene("outro", 30)]), props).errors, []);
});

test("narration: 10 to 90 words in each scene and 80 to 450 words in total", () => {
  const short = check(v3("plan", [scene("intro", 9), scene("phases", 91), scene("outro", 40)]), planProps());
  assert.deepEqual(lines(short.errors), [
    "narration.scenes[0].text: has 9 words, it must have 10 to 90",
    "narration.scenes[1].text: has 91 words, it must have 10 to 90",
  ]);
  assert.deepEqual(lines(check(v3("plan", [scene("intro", 30), scene("outro", 40)]), planProps()).errors),
    ["narration.scenes: has 70 words in total, it must have 80 to 450"]);
  const long = planProps({ rabbitHoles: ["No mobile layout."], doneWhen: [{ criterion: "c", check: "k" }] });
  const all = SCENES.plan.map((id) => scene(id, 60));
  assert.deepEqual(lines(check(v3("plan", all), long).errors), ["narration.scenes: has 480 words in total, it must have 80 to 450"]);
});

test("narration: the schema needs version 3, a mode, and scenes with a known id and a text of at most 900 characters", () => {
  const { errors } = check({ version: 2, mode: "plan", scenes: [{ scene: "goal", text: "" }, { scene: "intro", text: "x".repeat(901), extra: 1 }] }, planProps());
  assert.deepEqual(errors.map((e) => e.path), ["narration.version", "narration.scenes[0].scene", "narration.scenes[0].text", "narration.scenes[1].text", "narration.scenes[1].extra"]);
  assert.deepEqual(lines(check({ mode: "plan" }, planProps()).errors), ["narration.version: required", "narration.scenes: required"]);
  assert.deepEqual(lines(check({ mode: "plan", sections: [{ id: "intro", text: "Hello." }] }, planProps()).errors),
    ["narration.version: required", "narration.scenes: required", "narration.sections: unknown field"], "a version 1 narration is not valid any more");
  assert.deepEqual(lines(check(v3("ship", [scene("intro", 80)]), planProps()).errors), ["narration.mode: must be plan, the mode of the props"]);
  assert.ok(!fs.existsSync(path.join(SKILL, "schema", "narration-v1.schema.json")), "the version 1 schema is gone");
});

test("narration: invalid JSON is one error, an unreadable file throws", () => {
  assert.match(checkNarration(narrationFile("{"), planProps()).errors[0].message, /^invalid JSON/);
  assert.throws(() => checkNarration(path.join(TMP, "missing.json"), planProps()), /cannot read/);
});

test("graph tour: tourStops(graph, null, props) is the default tour of the graph page", async () => {
  const { buildGraph } = await import(path.join(SKILL, "scripts", "graph.mjs"));
  const graph = buildGraph(planProps());
  const stops = tourStops(graph, null, planProps());
  assert.deepEqual(stops.map((s) => s.id), ["overview", "goal", ...graph.groups.map((g) => g.id), "explore"]);
  assert.ok(stops.every((s) => s.text === "" && s.caption && Array.isArray(s.nodes) && typeof s.overview === "boolean"));
  assert.deepEqual([stops[0].overview, stops.at(-1).overview], [true, true]);
});

// ---- text to speech ----

// ---- walkthrough: the report page focuses one section at a time, with the narration as commentary ----

test("walkthrough: render --narration embeds one step for each scene, with its section and its text", () => {
  const narration = v3("ship", [scene("intro", 30), scene("tldr", 30), scene("hand-test", 30), scene("outro", 30)]);
  const r = run(RENDER, ["--props", "<props>", "--narration", "<narration>", "--out", "<out>"], { props: shipProps(), narration });
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(steps(r.report), [
    { scene: "intro", target: "header", text: words(30) },
    { scene: "tldr", target: "tldr", text: words(30) },
    { scene: "hand-test", target: "hand-test", text: words(30) },
    { scene: "outro", target: null, text: words(30) },
  ]);
  assert.match(r.report, /<button type="button" class="wt-start" data-walkthrough-start>/);
  for (const target of ["header", "tldr", "hand-test"]) assert.match(r.report, new RegExp(`data-shot="${target}"`), target);
  assert.ok(!r.report.includes("<video"), "the report has no video player");
});

test("walkthrough: the embedded JSON cannot close its script element", () => {
  const narration = v3("ship", [{ scene: "intro", text: `${words(20)} </script><script>alert(1)</script>` }, scene("outro", 70)]);
  const r = run(RENDER, ["--props", "<props>", "--narration", "<narration>", "--out", "<out>"], { props: shipProps(), narration });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.report.match(/<\/script>/g).length, 2, "only the JSON block and the page script close a script element");
  assert.match(steps(r.report)[0].text, /<\/script><script>alert\(1\)<\/script>$/);
});

test("walkthrough: without --narration the report has no walkthrough", () => {
  const r = run(RENDER, ["--props", "<props>", "--out", "<out>"], { props: shipProps() });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(steps(r.report), null);
  assert.ok(!r.report.includes("data-walkthrough-start"));
});

test("walkthrough: render --check --narration checks both files; an invalid narration exits 1 with field paths", () => {
  const good = v3("plan", [scene("intro", 40), scene("phases", 40)]);
  assert.equal(run(RENDER, ["--props", "<props>", "--narration", "<narration>", "--check"], { props: planProps(), narration: good }).status, 0);
  const bad = run(RENDER, ["--props", "<props>", "--narration", "<narration>", "--check"], { props: planProps(), narration: v3("plan", [scene("phases", 40), scene("outro", 40)]) });
  assert.equal(bad.status, 1);
  assert.equal(bad.stderr, 'narration.scenes: must have the "intro" scene\n');
  const missing = run(RENDER, ["--props", "<props>", "--narration", "/nonexistent/narration.json", "--check"], { props: planProps() });
  assert.equal(missing.status, 2);
  assert.match(missing.stderr, /cannot read/);
});

test("walkthrough: --video is not a flag any more", () => {
  const r = run(RENDER, ["--props", "<props>", "--out", "<out>", "--video", "video.mp4"], { props: shipProps() });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /^usage: render\.mjs/);
});
