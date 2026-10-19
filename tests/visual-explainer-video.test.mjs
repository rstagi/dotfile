import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SKILL = path.join(import.meta.dirname, "..", ".claude", "skills", "visual-explainer");
const RENDER = path.join(SKILL, "scripts", "render.mjs");
const RENDER_VIDEO = path.join(SKILL, "scripts", "render-video.mjs");
const { clip, paginate, storyboard } = await import(path.join(SKILL, "video", "src", "storyboard.mjs"));
const { cacheDir, cacheKey, parseArgs, pruneCaches, renderAttempts, videoSettings } = await import(RENDER_VIDEO);
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "visual-explainer-video-"));
// Not a root after() hook: with --test-name-pattern, node runs it before a long filtered test ends.
process.on("exit", () => fs.rmSync(TMP, { recursive: true, force: true }));

const EXAMPLES = Object.fromEntries(fs.readdirSync(path.join(SKILL, "examples")).filter((f) => f.endsWith(".props.json")).map((file) => {
  const props = JSON.parse(fs.readFileSync(path.join(SKILL, "examples", file), "utf8"));
  return [props.mode, props];
}));

const planProps = (over = {}) => ({
  mode: "plan",
  meta: { title: "Widget effort plan", subtitle: "Two milestones, one lane", date: "2026-10-18" },
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
      number: 1, title: "Read path",
      phases: [
        { number: 1, title: "Add the widget store", kind: "work" },
        { number: 2, title: "Review milestone 1", kind: "pr-review", gate: true, dependsOn: [1] },
      ],
    },
    { number: 2, title: "Write path", phases: [{ number: 3, title: "Accept POST /widgets", kind: "work" }] },
  ],
  decisions: [{ decision: "Use SQLite", why: "One file, no server.", rejected: [{ option: "Postgres", reason: "Needs a server." }] }],
  risks: [{ risk: "Lock contention", impact: "high", mitigation: "WAL mode." }],
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

const EXAMPLE_NARRATION = Object.fromEntries(Object.keys(EXAMPLES).map((mode) => [mode, JSON.parse(fs.readFileSync(path.join(SKILL, "examples",
  fs.readdirSync(path.join(SKILL, "examples")).find((f) => f.startsWith(mode) && f.endsWith(".narration.json"))), "utf8"))]));
const introOnly = (mode) => ({ mode, sections: [{ id: "intro", text: "Hello." }] });

const kinds = (board) => board.scenes.map((s) => s.kind);

function run(script, args, { props, narration, env = {}, timeout } = {}) {
  const dir = fs.mkdtempSync(path.join(TMP, "case-"));
  const propsFile = path.join(dir, "props.json");
  const narrationFile = path.join(dir, "narration.json");
  if (props !== undefined) fs.writeFileSync(propsFile, typeof props === "string" ? props : JSON.stringify(props));
  if (narration !== undefined) fs.writeFileSync(narrationFile, JSON.stringify(narration));
  const argv = args.map((a) => a.replace("<props>", propsFile).replace("<narration>", narrationFile).replace("<out>", path.join(dir, "out")));
  const res = spawnSync(process.execPath, [script, ...argv], { encoding: "utf8", env: { ...process.env, ...env }, timeout });
  const page = path.join(dir, "out", "index.html");
  return { ...res, dir, html: fs.existsSync(page) ? fs.readFileSync(page, "utf8") : null };
}

// ---- narration check ----

const NARRATION = path.join(SKILL, "scripts", "narration.mjs");
const { checkNarration } = await import(NARRATION);

function narrationFile(narration) {
  const file = path.join(fs.mkdtempSync(path.join(TMP, "narration-")), "narration.json");
  fs.writeFileSync(file, typeof narration === "string" ? narration : JSON.stringify(narration));
  return file;
}

const say = (id, text = `The ${id} section.`) => ({ id, text });

test("narration: the committed examples are valid for their props", () => {
  for (const [mode, props] of Object.entries(EXAMPLES)) {
    const file = path.join(SKILL, "examples", fs.readdirSync(path.join(SKILL, "examples")).find((f) => f.startsWith(mode) && f.endsWith(".narration.json")));
    assert.deepEqual(checkNarration(file, props).errors, [], mode);
  }
});

test("narration: a section id must exist for the mode, appear once, and have its props section", () => {
  const props = shipProps();
  const { errors } = checkNarration(narrationFile({
    mode: "ship",
    sections: [say("intro"), say("goal"), say("intro"), say("changes"), say("setbacks"), say("summary")],
  }), props);
  assert.deepEqual(errors.map((e) => `${e.path}: ${e.message}`), [
    'narration.sections[1].id: "goal" is not a ship section',
    'narration.sections[2].id: "intro" appears more than once',
    'narration.sections[3].id: the props have no changes section',
    'narration.sections[4].id: the props have no setbacks section',
  ]);
  const withSections = shipProps({ changes: [{ area: "a", how: "h", why: "w" }], setbacks: [{ kind: "retry", title: "t", what: "w", resolution: "r" }] });
  assert.deepEqual(checkNarration(narrationFile({ mode: "ship", sections: [say("changes"), say("setbacks")] }), withSections).errors, []);
});

test("narration: the schema rejects an unknown id, an empty or long text and a wrong mode", () => {
  const { errors } = checkNarration(narrationFile({
    mode: "plan",
    sections: [say("foo"), { id: "goal", text: "" }, { id: "risks", text: "x".repeat(1501) }, { id: "outro" }],
  }), planProps());
  assert.deepEqual(errors.map((e) => e.path), ["narration.sections[0].id", "narration.sections[1].text", "narration.sections[2].text", "narration.sections[3].text"]);
  const wrongMode = checkNarration(narrationFile({ mode: "ship", sections: [say("intro")] }), planProps());
  assert.deepEqual(wrongMode.errors.map((e) => `${e.path}: ${e.message}`), ["narration.mode: must be plan, the mode of the props"]);
});

test("narration: more than 9000 characters in total is rejected", () => {
  const sections = ["intro", "goal", "architecture", "milestones", "decisions", "risks", "outro"].map((id) => say(id, "x".repeat(1300)));
  const { errors } = checkNarration(narrationFile({ mode: "plan", sections }), planProps());
  assert.deepEqual(errors.map((e) => `${e.path}: ${e.message}`), ["narration.sections: has 9100 characters in total, the limit is 9000"]);
});

test("narration: invalid JSON is one error, an unreadable file throws", () => {
  assert.match(checkNarration(narrationFile("{"), planProps()).errors[0].message, /^invalid JSON/);
  assert.throws(() => checkNarration(path.join(TMP, "missing.json"), planProps()), /cannot read/);
});

// ---- narration v2: stops on the graph ----

const stop = (focus, text = `About ${focus.join(" and ")}.`, extra = {}) => ({ focus, text, ...extra });
const v2 = (mode, stops) => ({ version: 2, mode, stops });

test("narration v2: each stop resolves to its nodes; all is the overview and a group id names its nodes", () => {
  const props = planProps();
  const { errors, stops } = checkNarration(narrationFile(v2("plan", [
    stop(["all"], "The whole plan."),
    stop(["risk-1", "phase-2"], "A risky phase.", { title: "Risky phase" }),
    stop(["group-timeline"]),
    stop(["component-http-api"]),
  ])), props);
  assert.deepEqual(errors, []);
  assert.deepEqual(stops.map((s) => s.id), ["stop-1", "stop-2", "stop-3", "stop-4"]);
  assert.equal(stops[0].overview, true);
  assert.equal(stops[0].nodes.length, 12, "the overview lights every node");
  assert.deepEqual(stops[1].nodes, ["phase-2", "risk-1"], "in graph order");
  assert.equal(stops[1].title, "Risky phase");
  assert.deepEqual(stops[2].nodes, ["milestone-1", "phase-1", "phase-2", "milestone-2", "phase-3"]);
  assert.equal(stops[2].title, "Timeline", "a group stop is titled by its group");
  assert.equal(stops[3].title, "HTTP API", "a one-node stop is titled by its node");
  assert.deepEqual(stops.map((s) => s.text), ["The whole plan.", "A risky phase.", "About group-timeline.", "About component-http-api."]);
});

test("narration v2: an unknown or repeated focus id, a wrong mode and the length limit fail with field paths", () => {
  const { errors } = checkNarration(narrationFile(v2("plan", [
    stop(["goal", "phase-9"]), stop(["risk-1", "risk-1"]), stop(["group-nope"]),
  ])), planProps());
  assert.deepEqual(errors.map((e) => `${e.path}: ${e.message}`), [
    'narration.stops[0].focus[1]: "phase-9" is not a node id or a group id (render.mjs --ids lists them)',
    'narration.stops[1].focus[1]: "risk-1" appears more than once in this stop',
    'narration.stops[2].focus[0]: "group-nope" is not a node id or a group id (render.mjs --ids lists them)',
  ]);
  const wrongMode = checkNarration(narrationFile(v2("ship", [stop(["summary"])])), planProps());
  assert.deepEqual(wrongMode.errors.map((e) => `${e.path}: ${e.message}`), ["narration.mode: must be plan, the mode of the props"]);
  const long = checkNarration(narrationFile(v2("plan", Array.from({ length: 7 }, () => stop(["goal"], "x".repeat(1300))))), planProps());
  assert.deepEqual(long.errors.map((e) => `${e.path}: ${e.message}`), ["narration.stops: has 9100 characters in total, the limit is 9000"]);
});

test("narration v2: the schema needs version 2 and stops with a focus list and a text", () => {
  const { errors } = checkNarration(narrationFile({ version: 3, mode: "plan", stops: [{ focus: [], text: "" }, { text: "t", extra: 1 }] }), planProps());
  assert.deepEqual(errors.map((e) => e.path), [
    "narration.version", "narration.stops[0].focus", "narration.stops[0].text", "narration.stops[1].focus", "narration.stops[1].extra",
  ]);
  const missing = checkNarration(narrationFile({ mode: "plan" }), planProps());
  assert.deepEqual(missing.errors.map((e) => `${e.path}: ${e.message}`), ["narration.version: required", "narration.stops: required"]);
});

// ---- narration v1: each section maps to its cluster of nodes ----

test("narration v1 (plan): each section becomes one stop on its cluster, in video order, with or without text", () => {
  const { errors, stops } = checkNarration(narrationFile({ mode: "plan", sections: [say("risks"), say("intro"), say("milestones")] }), planProps());
  assert.deepEqual(errors, []);
  assert.deepEqual(stops.map((s) => s.id), ["intro", "goal", "architecture", "milestones", "decisions", "risks", "outro"]);
  const byId = Object.fromEntries(stops.map((s) => [s.id, s]));
  assert.deepEqual([byId.intro.overview, byId.outro.overview], [true, true]);
  assert.deepEqual(byId.goal.nodes, ["goal"]);
  assert.deepEqual(byId.architecture.nodes, ["layer-1", "component-http-api", "layer-2", "component-widget-store"]);
  assert.deepEqual(byId.milestones.nodes, ["milestone-1", "phase-1", "phase-2", "milestone-2", "phase-3"]);
  assert.deepEqual(byId.risks.nodes, ["risk-1"]);
  assert.deepEqual([byId.risks.text, byId.intro.text], ["The risks section.", "The intro section."]);
  assert.equal(byId.goal.text, "", "a section without narration has no voice");
  assert.ok(byId.goal.caption.length > 0, "but the page tour still has a caption");
});

test("narration v1 (ship): an empty cluster maps to the outcome node, and optional clusters follow the props", () => {
  const props = shipProps({ changes: [{ area: "Read API", how: "h", why: "w" }], process: { attempts: 2 } });
  const { errors, stops } = checkNarration(narrationFile({ mode: "ship", sections: [say("deviations"), say("setbacks"), say("changes")] }), props);
  assert.deepEqual(errors, []);
  assert.deepEqual(stops.map((s) => s.id), ["intro", "summary", "changes", "delivered", "deviations", "setbacks", "reviews", "handTest", "rollback", "outro"]);
  const byId = Object.fromEntries(stops.map((s) => [s.id, s]));
  assert.deepEqual(byId.deviations.nodes, ["summary"], "no deviations: the outcome node");
  assert.deepEqual(byId.setbacks.nodes, ["process"]);
  assert.deepEqual(byId.changes.nodes, ["change-1"]);
  assert.deepEqual(byId.reviews.nodes, ["review-1"]);
});

// ---- the page tour comes from the narration ----

test("page tour: render.mjs takes narration.json beside the props, or --narration, as the tour captions", () => {
  const narration = v2("plan", [stop(["all"], "Welcome."), stop(["phase-2"], "The read phase.")]);
  const beside = run(RENDER, ["--props", "<props>", "--out", "<out>"], { props: planProps(), narration });
  assert.equal(beside.status, 0, beside.stderr);
  assert.equal(beside.stderr, "");
  const data = (html) => JSON.parse(/<script type="application\/json" id="ve-data">([^<]*)<\/script>/.exec(html)[1]);
  assert.deepEqual(data(beside.html).stops.map((s) => [s.text, s.nodes.length, s.overview]), [["Welcome.", 12, true], ["The read phase.", 1, false]]);
  const explicit = run(RENDER, ["--props", "<props>", "--out", "<out>", "--narration", "<narration>"], { props: planProps(), narration });
  assert.equal(explicit.status, 0, explicit.stderr);
  assert.equal(data(explicit.html).stops[1].text, "The read phase.");
});

test("page tour: a bad narration.json beside the props gives one warning and the default tour; a bad --narration exits 1", () => {
  const narration = v2("plan", [stop(["phase-99"], "Gone.")]);
  const beside = run(RENDER, ["--props", "<props>", "--out", "<out>"], { props: planProps(), narration });
  assert.equal(beside.status, 0, beside.stderr);
  assert.deepEqual(beside.stderr.trim().split("\n"), ['render.mjs: the tour ignores narration.json: narration.stops[0].focus[0]: "phase-99" is not a node id or a group id (render.mjs --ids lists them)']);
  assert.ok(beside.html, "the page is written");
  const explicit = run(RENDER, ["--props", "<props>", "--out", "<out>", "--narration", "<narration>"], { props: planProps(), narration });
  assert.equal(explicit.status, 1);
  assert.match(explicit.stderr, /^narration\.stops\[0\]\.focus\[0\]: "phase-99" is not a node id/);
  assert.equal(explicit.html, null);
});

// ---- text to speech ----

const { chooseEngine, clipKey, pickSayVoice, speak, synthesize, ttsSettings, wavSeconds } = await import(path.join(SKILL, "scripts", "tts.mjs"));
const KEY = "sk_test_0123456789abcdef";

// A fake engine: one second of silence for each 10 characters, as 24 kHz mono 16-bit WAV.
function fakeEngines(calls) {
  const engine = (name) => async ({ text, voice, model, out }) => {
    calls.push({ name, text, voice, model });
    const samples = Math.round((text.length / 10) * 24000);
    fs.writeFileSync(out, Buffer.concat([wavHeaderForTest(samples * 2), Buffer.alloc(samples * 2)]));
  };
  return { elevenlabs: engine("elevenlabs"), say: engine("say") };
}

function wavHeaderForTest(dataBytes, rate = 24000) {
  const h = Buffer.alloc(44);
  h.write("RIFF", 0); h.writeUInt32LE(36 + dataBytes, 4); h.write("WAVE", 8); h.write("fmt ", 12);
  h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22); h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate * 2, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34); h.write("data", 36); h.writeUInt32LE(dataBytes, 40);
  return h;
}

test("tts: auto uses ElevenLabs when the key is set, else say; an explicit engine wins", () => {
  assert.equal(chooseEngine("auto", { ELEVENLABS_API_KEY: KEY }), "elevenlabs");
  assert.equal(chooseEngine("auto", {}), "say");
  assert.equal(chooseEngine("auto", { ELEVENLABS_API_KEY: "" }), "say");
  assert.equal(chooseEngine("say", { ELEVENLABS_API_KEY: KEY }), "say");
  assert.equal(chooseEngine("elevenlabs", {}), "elevenlabs");
});

test("tts: settings use a stock voice and model, and VE_TTS_VOICE and VE_TTS_MODEL override them", () => {
  assert.deepEqual(ttsSettings("elevenlabs", { ELEVENLABS_API_KEY: KEY }), { engine: "elevenlabs", voice: "XrExE9yKIg1WjnnlVkGX", model: "eleven_flash_v2_5" });
  assert.deepEqual(ttsSettings("elevenlabs", { ELEVENLABS_API_KEY: KEY, VE_TTS_VOICE: "v1", VE_TTS_MODEL: "m1" }), { engine: "elevenlabs", voice: "v1", model: "m1" });
  assert.throws(() => ttsSettings("elevenlabs", {}), /ELEVENLABS_API_KEY is not set/);
  const listing = "Albert              en_US    # Hello!\nSamantha            en_US    # Hello!\n";
  assert.deepEqual(ttsSettings("say", {}, { sayVoices: listing }), { engine: "say", voice: "Samantha", model: "say" });
  assert.deepEqual(ttsSettings("say", { VE_TTS_VOICE: "Daniel" }, { sayVoices: listing }), { engine: "say", voice: "Daniel", model: "say" });
});

test("tts: say picks a premium or enhanced English voice first, then a known good voice", () => {
  const base = [
    "Albert              en_US    # Hello! My name is Albert.",
    "Amélie              fr_CA    # Bonjour !",
    "Daniel              en_GB    # Hello! My name is Daniel.",
    "Eddy (English (US)) en_US    # Hello! My name is Eddy.",
    "Samantha            en_US    # Hello! My name is Samantha.",
  ];
  assert.equal(pickSayVoice(base.join("\n")), "Samantha");
  assert.equal(pickSayVoice([...base, "Zoe (Premium)       en_US    # Hello!"].join("\n")), "Zoe (Premium)");
  assert.equal(pickSayVoice([...base, "Ava (Enhanced)      en_US    # Hello!"].join("\n")), "Ava (Enhanced)");
  assert.equal(pickSayVoice([base[0], base[2]].join("\n")), "Daniel");
  assert.equal(pickSayVoice("Amélie              fr_CA    # Bonjour !"), null, "no English voice: the system default");
});

test("tts: the clip key covers the engine, the voice, the model and the text, and never the API key", () => {
  const base = { engine: "elevenlabs", voice: "v", model: "m", text: "Hello." };
  const key = clipKey(base);
  assert.match(key, /^[0-9a-f]{64}$/);
  for (const change of [{ engine: "say" }, { voice: "w" }, { model: "n" }, { text: "Hello!" }]) {
    assert.notEqual(clipKey({ ...base, ...change }), key, JSON.stringify(change));
  }
  assert.equal(clipKey({ ...base, apiKey: KEY }), key);
});

test("tts: synthesize makes one clip for each section, caches it, and counts the characters it sent", async () => {
  const dir = fs.mkdtempSync(path.join(TMP, "tts-"));
  const calls = [];
  const sections = [{ id: "intro", text: "x".repeat(50) }, { id: "goal", text: "y".repeat(30) }];
  const settings = { engine: "elevenlabs", voice: "v", model: "m" };
  const first = await synthesize(sections, settings, { dir, engines: fakeEngines(calls), env: { ELEVENLABS_API_KEY: KEY } });
  assert.deepEqual(first.clips.map((c) => [c.id, c.seconds]), [["intro", 5], ["goal", 3]]);
  assert.deepEqual([first.sentChars, first.cached, calls.length], [80, 0, 2]);
  assert.ok(first.clips.every((c) => c.file === path.join(dir, `${clipKey({ ...settings, text: sections.find((s) => s.id === c.id).text })}.wav`)));

  const second = await synthesize(sections, settings, { dir, engines: fakeEngines(calls), env: { ELEVENLABS_API_KEY: KEY } });
  assert.deepEqual([second.sentChars, second.cached, calls.length], [0, 2, 2], "a re-render sends nothing");
  await synthesize(sections, { ...settings, voice: "w" }, { dir, engines: fakeEngines(calls), env: {} });
  assert.equal(calls.length, 4, "a new voice makes new clips");
  assert.ok(fs.readdirSync(dir).every((name) => /^[0-9a-f]{64}\.wav$/.test(name)), "only finished clips, named by their key");
});

test("tts: a failure names the section, hides the API key and leaves no partial clip", async () => {
  const dir = fs.mkdtempSync(path.join(TMP, "tts-"));
  const engines = {
    elevenlabs: async ({ out }) => {
      fs.writeFileSync(out, "partial");
      throw new Error(`HTTP 401 for key ${KEY}`);
    },
  };
  await assert.rejects(
    synthesize([{ id: "goal", text: "Hello." }], { engine: "elevenlabs", voice: "v", model: "m" }, { dir, engines, env: { ELEVENLABS_API_KEY: KEY } }),
    (err) => {
      assert.match(err.message, /^text to speech failed for section goal: HTTP 401 for key \*\*\*$/);
      assert.ok(!err.message.includes(KEY));
      return true;
    },
  );
  assert.deepEqual(fs.readdirSync(dir), []);
});

// ---- text to speech: the macOS voice fallback ----

const SAY_LISTING = "Samantha            en_US    # Hello!\n";
const QUOTA_BODY = JSON.stringify({ detail: { status: "quota_exceeded", message: `The key ${KEY} has 355 credits, 667 are needed.` } });

// A fake fetch: each call takes the next response. A response is { status, body } or "network".
function fakeFetch(responses, calls) {
  return async (url, init) => {
    calls.push({ url, headers: init.headers });
    const next = responses[Math.min(calls.length - 1, responses.length - 1)];
    if (next === "network") throw new TypeError("fetch failed", { cause: Object.assign(new Error(`getaddrinfo ENOTFOUND api.elevenlabs.io ${KEY}`), { code: "ENOTFOUND" }) });
    return new Response(next.body ?? "", { status: next.status });
  };
}

const NARRATED = [{ id: "intro", text: "x".repeat(50) }, { id: "goal", text: "y".repeat(30) }, { id: "outro", text: "z".repeat(20) }];

function filesContain(dir, needle) {
  return fs.readdirSync(dir).some((name) => name.includes(needle) || fs.readFileSync(path.join(dir, name)).includes(needle));
}

for (const [label, responses, reason, requests] of [
  ["401 with a quota error", [{ status: 401, body: QUOTA_BODY }], "HTTP 401 quota_exceeded", 1],
  ["402", [{ status: 402, body: "payment required" }], "HTTP 402", 1],
  ["429 after its one retry", [{ status: 429, body: "busy" }], "HTTP 429", 2],
  ["a network error", ["network"], "network error ENOTFOUND", 1],
]) {
  test(`tts fallback: auto renders the whole narration with say when ElevenLabs fails with ${label}`, async () => {
    const dir = fs.mkdtempSync(path.join(TMP, "tts-"));
    const calls = [];
    const sayCalls = [];
    const notices = [];
    const result = await speak(NARRATED, "auto", {
      dir, env: { ELEVENLABS_API_KEY: KEY }, fetch: fakeFetch(responses, calls), retryDelayMs: 0,
      engines: { say: fakeEngines(sayCalls).say }, sayVoices: SAY_LISTING, onFallback: (r) => notices.push(r),
    });
    assert.equal(calls.length, requests, "the first clip fails, so no later clip goes to ElevenLabs");
    assert.deepEqual(result.settings, { engine: "say", voice: "Samantha", model: "say" });
    assert.equal(result.fallback, reason);
    assert.deepEqual(notices, [reason]);
    assert.deepEqual(sayCalls.map((c) => c.text), NARRATED.map((s) => s.text), "say speaks every section: one voice");
    assert.deepEqual(result.clips.map((c) => [c.id, c.file]), NARRATED.map((s) => [s.id, path.join(dir, `${clipKey({ ...result.settings, text: s.text })}.wav`)]));
    assert.ok(!JSON.stringify(result).includes(KEY), "no key in the result");
    assert.ok(!filesContain(dir, KEY), "no key in a clip name or a clip");
  });
}

test("tts fallback: a failure after good ElevenLabs clips still gives a say voice for every section", async () => {
  const dir = fs.mkdtempSync(path.join(TMP, "tts-"));
  const calls = [];
  const sayCalls = [];
  const result = await speak(NARRATED, "auto", {
    dir, env: { ELEVENLABS_API_KEY: KEY }, retryDelayMs: 0, engines: { say: fakeEngines(sayCalls).say }, sayVoices: SAY_LISTING,
    fetch: fakeFetch([{ status: 200, body: Buffer.alloc(4800) }, { status: 401, body: QUOTA_BODY }], calls),
  });
  assert.equal(calls.length, 2);
  assert.equal(result.fallback, "HTTP 401 quota_exceeded");
  assert.ok(result.clips.every((c) => c.file.endsWith(`${clipKey({ engine: "say", voice: "Samantha", model: "say", text: NARRATED.find((s) => s.id === c.id).text })}.wav`)));
  assert.equal(sayCalls.length, 3, "the intro clip of ElevenLabs is not used");
});

test("tts fallback: an explicit --tts elevenlabs fails, with no say pass and no key in the error", async () => {
  const dir = fs.mkdtempSync(path.join(TMP, "tts-"));
  const sayCalls = [];
  await assert.rejects(speak(NARRATED, "elevenlabs", {
    dir, env: { ELEVENLABS_API_KEY: KEY }, retryDelayMs: 0, engines: { say: fakeEngines(sayCalls).say }, sayVoices: SAY_LISTING,
    fetch: fakeFetch([{ status: 401, body: QUOTA_BODY }], []),
  }), (err) => {
    assert.match(err.message, /^text to speech failed for section intro: ElevenLabs returned HTTP 401/);
    assert.ok(!err.message.includes(KEY));
    return true;
  });
  assert.equal(sayCalls.length, 0);
});

// A fetch that answers only when its signal aborts. Like an open socket, it keeps node running.
const hangingFetch = (calls) => (url, init) => {
  calls.push(url);
  const alive = setInterval(() => {}, 1000);
  return new Promise((_, reject) => init.signal.addEventListener("abort", () => {
    clearInterval(alive);
    reject(init.signal.reason);
  }));
};

test("tts fallback: an ElevenLabs request above its timeout falls back to say", async () => {
  const dir = fs.mkdtempSync(path.join(TMP, "tts-"));
  const sayCalls = [];
  const result = await speak(NARRATED, "auto", {
    dir, env: { ELEVENLABS_API_KEY: KEY }, fetch: hangingFetch([]), requestTimeoutMs: 20,
    engines: { say: fakeEngines(sayCalls).say }, sayVoices: SAY_LISTING, signal: AbortSignal.timeout(60_000),
  });
  assert.equal(result.fallback, "timeout");
  assert.equal(sayCalls.length, 3);
});

test("tts fallback: when the run deadline stops ElevenLabs, there is no say pass", async () => {
  const dir = fs.mkdtempSync(path.join(TMP, "tts-"));
  const sayCalls = [];
  await assert.rejects(speak(NARRATED, "auto", {
    dir, env: { ELEVENLABS_API_KEY: KEY }, fetch: hangingFetch([]), engines: { say: fakeEngines(sayCalls).say }, sayVoices: SAY_LISTING,
    signal: AbortSignal.timeout(20),
  }), /text to speech failed for section intro/);
  assert.equal(sayCalls.length, 0);
});

test("tts fallback: VE_TTS_VOICE names the ElevenLabs voice, so the say pass uses the best say voice", async () => {
  const sayCalls = [];
  const result = await speak(NARRATED, "auto", {
    dir: fs.mkdtempSync(path.join(TMP, "tts-")), env: { ELEVENLABS_API_KEY: KEY, VE_TTS_VOICE: "XrExE9yKIg1WjnnlVkGX" }, retryDelayMs: 0,
    fetch: fakeFetch([{ status: 401 }], []), engines: { say: fakeEngines(sayCalls).say }, sayVoices: SAY_LISTING,
  });
  assert.deepEqual(result.settings, { engine: "say", voice: "Samantha", model: "say" });
  assert.ok(sayCalls.every((c) => c.voice === "Samantha"));
});

test("tts: wavSeconds reads the duration from the WAV header", () => {
  const file = path.join(fs.mkdtempSync(path.join(TMP, "wav-")), "a.wav");
  fs.writeFileSync(file, Buffer.concat([wavHeaderForTest(48000 * 3, 24000), Buffer.alloc(48000 * 3)]));
  assert.equal(wavSeconds(file), 3);
});

// ---- render-video.mjs: arguments ----

test("render-video: parseArgs reads the flags and applies the defaults", () => {
  const base = { props: "p.json", narration: "n.json", check: false, timeoutMin: 15, maxMb: 14, tts: "auto" };
  assert.deepEqual(parseArgs(["--props", "p.json", "--narration", "n.json", "--out", "o"]), { ...base, out: "o" });
  assert.deepEqual(parseArgs(["--check", "--props", "p.json", "--narration", "n.json"]), { ...base, check: true });
  assert.deepEqual(parseArgs(["--props", "p.json", "--narration", "n.json", "--out", "o", "--timeout-min", "2.5", "--max-mb", "9", "--tts", "say"]),
    { ...base, out: "o", timeoutMin: 2.5, maxMb: 9, tts: "say" });
});

test("render-video: parseArgs rejects a call that is not exactly one valid form", () => {
  const ok = ["--props", "p.json", "--narration", "n.json", "--out", "o"];
  for (const argv of [
    [], ["--props", "p.json", "--narration", "n.json"], ["--out", "o"], ["--props", "p.json", "--out", "o"], ["--narration", "n.json", "--check"],
    [...ok.slice(0, 5)], [...ok, "--extra"], [...ok, "--timeout-min", "0"], [...ok, "--timeout-min", "ten"],
    [...ok, "--max-mb", "-1"], [...ok, "--max-mb", ""], [...ok, "--tts", "openai"], [...ok, "--tts"],
  ]) assert.equal(parseArgs(argv), null, argv.join(" "));
});

test("render-video: a usage error exits 2 with the usage line", () => {
  const r = run(RENDER_VIDEO, ["--props", "<props>", "--timeout-min", "x"], { props: shipProps() });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /^usage: render-video\.mjs --props <file> --narration <file> --out <dir> \[--check\] \[--timeout-min <n>\] \[--max-mb <n>\] \[--tts auto\|elevenlabs\|say\]/);
});

test("render-video: invalid props exit 1 with one field-path line each, and no cache or output is made", () => {
  const cache = fs.mkdtempSync(path.join(TMP, "cache-"));
  const r = run(RENDER_VIDEO, ["--props", "<props>", "--narration", "<narration>", "--out", "<out>"], {
    props: shipProps({ rollback: [], extra: 1 }), narration: introOnly("ship"), env: { XDG_CACHE_HOME: cache },
  });
  assert.equal(r.status, 1, r.stderr);
  assert.deepEqual(r.stderr.trim().split("\n"), ["$.rollback: must have at least 1 item(s)", "$.extra: unknown field"]);
  assert.deepEqual(fs.readdirSync(cache), [], "no cache directory");
  assert.ok(!fs.existsSync(path.join(r.dir, "out")), "no output directory");
});

test("render-video: an invalid narration exits 1 with one field-path line each", () => {
  const r = run(RENDER_VIDEO, ["--props", "<props>", "--narration", "<narration>", "--out", "<out>"], {
    props: shipProps(), narration: { mode: "ship", sections: [{ id: "goal", text: "Hi." }, { id: "intro", text: "" }] },
  });
  assert.equal(r.status, 1, r.stderr);
  assert.deepEqual(r.stderr.trim().split("\n"), ["narration.sections[1].text: must not be empty"]);
  const second = run(RENDER_VIDEO, ["--props", "<props>", "--narration", "<narration>", "--check"], {
    props: shipProps(), narration: { mode: "ship", sections: [{ id: "goal", text: "Hi." }] },
  });
  assert.deepEqual(second.stderr.trim().split("\n"), ['narration.sections[0].id: "goal" is not a ship section']);
  assert.ok(!fs.existsSync(path.join(r.dir, "out")), "no output directory");
});

test("render-video: --check validates the props and the narration of either mode and makes nothing", () => {
  const cache = fs.mkdtempSync(path.join(TMP, "cache-"));
  for (const [mode, props] of Object.entries(EXAMPLES)) {
    const r = run(RENDER_VIDEO, ["--props", "<props>", "--narration", "<narration>", "--check"], {
      props, narration: EXAMPLE_NARRATION[mode], env: { XDG_CACHE_HOME: cache },
    });
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.stdout, "");
  }
  assert.deepEqual(fs.readdirSync(cache), []);
  const bad = run(RENDER_VIDEO, ["--props", "<props>", "--narration", "<narration>", "--check"], { props: { mode: "video" }, narration: introOnly("plan") });
  assert.equal(bad.status, 1);
  assert.match(bad.stderr, /^\$\.mode: must be one of plan, ship$/m);
});

test("render-video: an unreadable props or narration file exits 2", () => {
  const r = run(RENDER_VIDEO, ["--props", path.join(TMP, "missing.json"), "--narration", "<narration>", "--check"], { narration: introOnly("plan") });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /cannot read .*missing\.json/);
  const n = run(RENDER_VIDEO, ["--props", "<props>", "--narration", path.join(TMP, "missing-narration.json"), "--check"], { props: planProps() });
  assert.equal(n.status, 2);
  assert.match(n.stderr, /cannot read .*missing-narration\.json/);
});

// ---- render-video.mjs: encoder settings ----

test("encoder: the defaults are 24 fps, x264 veryfast, ANGLE and all cores but 2", () => {
  assert.deepEqual(videoSettings({}),
    { fps: 24, concurrency: Math.max(1, os.availableParallelism() - 2), hardwareAcceleration: "disable", x264Preset: "veryfast", gl: "angle" });
});

test("encoder: VE_VIDEO_* variables override each default, and a bad value is an error", () => {
  assert.deepEqual(videoSettings({ VE_VIDEO_FPS: "30", VE_VIDEO_CONCURRENCY: "8", VE_VIDEO_HWACCEL: "if-possible", VE_VIDEO_X264_PRESET: "medium", VE_VIDEO_GL: "swangle" }),
    { fps: 30, concurrency: 8, hardwareAcceleration: "if-possible", x264Preset: "medium", gl: "swangle" });
  assert.equal(videoSettings({ VE_VIDEO_GL: "default" }).gl, null, "default lets Chrome choose");
  for (const [name, value] of [["VE_VIDEO_FPS", "0"], ["VE_VIDEO_FPS", "29.97"], ["VE_VIDEO_CONCURRENCY", "-2"], ["VE_VIDEO_HWACCEL", "yes"], ["VE_VIDEO_X264_PRESET", "turbo"], ["VE_VIDEO_GL", "metal"]]) {
    assert.throws(() => videoSettings({ [name]: value }), new RegExp(`^Error: ${name} must be`), `${name}=${value}`);
  }
});

test("encoder: render-video rejects a bad VE_VIDEO_* value with exit 2 before any render work", () => {
  const cache = fs.mkdtempSync(path.join(TMP, "cache-"));
  const r = run(RENDER_VIDEO, ["--props", "<props>", "--narration", "<narration>", "--out", "<out>"], {
    props: planProps(), narration: introOnly("plan"), env: { XDG_CACHE_HOME: cache, VE_VIDEO_FPS: "fast" },
  });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /VE_VIDEO_FPS must be an integer from 1 to 60, got fast/);
  assert.deepEqual(fs.readdirSync(cache), [], "no cache work");
});

test("encoder: x264 attempts lower the quality, then the size", () => {
  const attempts = renderAttempts({ fps: 24, concurrency: 8, hardwareAcceleration: "disable", x264Preset: "veryfast", gl: "angle" }, 300, 14);
  assert.deepEqual(attempts.map((a) => [a.scale, a.crf, a.x264Preset, a.videoBitrate]), [[1, 23, "veryfast", undefined], [1, 30, "veryfast", undefined], [2 / 3, 30, "veryfast", undefined]]);
  assert.ok(attempts.every((a) => a.fps === 24 && a.concurrency === 8 && a.hardwareAcceleration === "disable" && a.gl === "angle"));
});

// Measured on the ship example, x264 veryfast, 24 fps, 1920x1080, on an Apple M4:
// 278.6 s -> 10.64 MB at CRF 23 · 547.5 s -> 15.54 MB at CRF 23, 12.11 MB at CRF 27, 10.35 MB at CRF 30 ·
// 599.4 s -> 12.39 MB at CRF 28.
test("encoder: the first x264 quality comes from the video length, so a 10 minute video fits 14 MB on the first render", () => {
  const x264 = { fps: 24, concurrency: 8, hardwareAcceleration: "disable", x264Preset: "veryfast", gl: "angle" };
  const crfs = (seconds, maxMb = 14) => renderAttempts(x264, seconds, maxMb).map((a) => [a.label, a.scale, a.crf]);
  const backup = [["1920x1080 at a lower quality", 1, 30], ["1280x720", 2 / 3, 30]];
  assert.deepEqual(crfs(60), [["1920x1080", 1, 23], ...backup], "a short video keeps CRF 23");
  assert.deepEqual(crfs(278.6), [["1920x1080", 1, 23], ...backup], "10.6 MB at CRF 23 fits");
  assert.deepEqual(crfs(547.5), [["1920x1080", 1, 27], ...backup], "15.5 MB at CRF 23 is above the limit, 12.1 MB at CRF 27 fits");
  assert.deepEqual(crfs(600), [["1920x1080", 1, 28], ...backup], "10 minutes: 12.4 MB at CRF 28 fits");
  assert.deepEqual(crfs(600, 30), [["1920x1080", 1, 23], ...backup], "a larger limit keeps CRF 23");
  assert.deepEqual(crfs(1500), [["1920x1080", 1, 30], ["1280x720", 2 / 3, 30]], "CRF 30 is the lowest first quality, then only 1280x720 is left");
  let last = 0;
  for (let seconds = 30; seconds <= 1500; seconds += 30) {
    const crf = renderAttempts(x264, seconds, 14)[0].crf;
    assert.ok(crf >= last, `${seconds} s: a longer video never gets a better first quality`);
    last = crf;
  }
});

test("encoder: a hardware encoder gets a bitrate that fits the size limit for the duration", () => {
  const settings = { fps: 24, concurrency: 8, hardwareAcceleration: "if-possible", x264Preset: "veryfast", gl: "angle" };
  const attempts = renderAttempts(settings, 300, 14);
  assert.deepEqual(attempts.map((a) => [a.scale, a.videoBitrate, a.crf]), [[1, "272k", undefined], [2 / 3, "190k", undefined]],
    "14 MB x 8 x 0.9 / 300 s = 336 kbps, minus 64 kbps of audio");
  assert.equal(renderAttempts(settings, 30, 14)[0].videoBitrate, "2500k", "a short video gets at most 2500 kbps");
  assert.equal(renderAttempts(settings, 3000, 14)[0].videoBitrate, "150k", "a long video gets at least 150 kbps");
});

// ---- render-video.mjs: cache ----

function copyVideoProject() {
  const dir = fs.mkdtempSync(path.join(TMP, "video-"));
  fs.cpSync(path.join(SKILL, "video"), dir, { recursive: true, filter: (src) => !src.includes(`${path.sep}node_modules`) });
  return dir;
}

test("cache key: 12 hex characters that depend on the content, not on the location", () => {
  const key = cacheKey(path.join(SKILL, "video"));
  assert.match(key, /^[0-9a-f]{12}$/);
  assert.equal(cacheKey(copyVideoProject()), key, "a copy of the project has the same key");
});

test("cache key: a change to the lock file or to a source file changes the key", () => {
  const base = cacheKey(path.join(SKILL, "video"));
  const lock = copyVideoProject();
  fs.appendFileSync(path.join(lock, "package-lock.json"), "\n");
  assert.notEqual(cacheKey(lock), base, "lock file");
  const source = copyVideoProject();
  fs.appendFileSync(path.join(source, "src", "theme.mjs"), "\n");
  assert.notEqual(cacheKey(source), base, "source file");
  const added = copyVideoProject();
  fs.writeFileSync(path.join(added, "src", "extra.mjs"), "");
  assert.notEqual(cacheKey(added), base, "new source file");
});

test("cache key: installed packages and dot files do not change the key", () => {
  const dir = copyVideoProject();
  fs.mkdirSync(path.join(dir, "node_modules", "remotion"), { recursive: true });
  fs.writeFileSync(path.join(dir, "node_modules", "remotion", "index.js"), "x");
  fs.writeFileSync(path.join(dir, ".DS_Store"), "x");
  assert.equal(cacheKey(dir), cacheKey(path.join(SKILL, "video")));
});

test("cache dir: under XDG_CACHE_HOME, else under HOME/.cache, never under the skill", () => {
  assert.equal(cacheDir({ XDG_CACHE_HOME: "/x/cache", HOME: "/home/u" }, "0123456789ab"), "/x/cache/visual-explainer/video-0123456789ab");
  assert.equal(cacheDir({ HOME: "/home/u" }, "0123456789ab"), "/home/u/.cache/visual-explainer/video-0123456789ab");
  assert.equal(cacheDir({ XDG_CACHE_HOME: "", HOME: "/home/u" }, "0123456789ab"), "/home/u/.cache/visual-explainer/video-0123456789ab", "an empty XDG_CACHE_HOME counts as unset");
});

test("cache prune: removes other caches unused for 7 days and stale temporary copies, nothing else", () => {
  const parent = fs.mkdtempSync(path.join(TMP, "prune-"));
  const now = Date.now();
  const day = 24 * 3600 * 1000;
  const make = (name, ageDays, ready = true) => {
    const dir = path.join(parent, name);
    fs.mkdirSync(dir);
    const marker = path.join(dir, ready ? ".ready" : "package.json");
    fs.writeFileSync(marker, "");
    const at = new Date(now - ageDays * day);
    fs.utimesSync(marker, at, at);
    fs.utimesSync(dir, at, at);
  };
  make("video-aaaaaaaaaaaa", 10);
  make("video-bbbbbbbbbbbb", 2);
  make("video-cccccccccccc", 30);
  make("video-cccccccccccc.tmp-1-ab12cd34", 2, false);
  make("video-dddddddddddd.tmp-2-ab12cd34", 0, false);
  make("notes", 30, false);
  pruneCaches(path.join(parent, "video-cccccccccccc"), now);
  assert.deepEqual(fs.readdirSync(parent).sort(), ["notes", "video-bbbbbbbbbbbb", "video-cccccccccccc", "video-dddddddddddd.tmp-2-ab12cd34"]);
});

test("render-video: a timeout exits 3, removes the partial cache and writes no video", () => {
  const cache = fs.mkdtempSync(path.join(TMP, "cache-"));
  const r = run(RENDER_VIDEO, ["--props", "<props>", "--narration", "<narration>", "--out", "<out>", "--timeout-min", "0.002"], {
    props: shipProps(), narration: introOnly("ship"), env: { XDG_CACHE_HOME: cache }, timeout: 60_000,
  });
  assert.equal(r.status, 3, r.stderr);
  assert.match(r.stderr, /timed out after 0\.002 min/);
  assert.deepEqual(fs.readdirSync(path.join(cache, "visual-explainer")), [], "no cache directory stays");
  assert.deepEqual(fs.readdirSync(path.join(r.dir, "out")), [], "no video and no temporary file");
});

// ---- render-video.mjs: the text to speech engine, end to end with fakes ----

// A fake world for one render-video.mjs process: a fetch preload (FAKE_FETCH=ok|401), `say` and
// `afconvert` on PATH, and a ready cache whose engine.mjs records its input and writes a tiny MP4.
function fakeWorld() {
  const root = fs.mkdtempSync(path.join(TMP, "world-"));
  const bin = path.join(root, "bin");
  fs.mkdirSync(bin);
  const wav = path.join(root, "silence.wav");
  fs.writeFileSync(wav, Buffer.concat([wavHeaderForTest(48000), Buffer.alloc(48000)]));
  const script = (name, body) => fs.writeFileSync(path.join(bin, name), `#!/usr/bin/env node\nconst fs = require("fs");\nconst a = process.argv.slice(2);\n${body}\n`, { mode: 0o755 });
  script("say", `if (a[0] === "-v" && a[1] === "?") { process.stdout.write(${JSON.stringify(SAY_LISTING)}); process.exit(0); }
fs.appendFileSync(process.env.FAKE_SAY_LOG, a.join(" ") + "\\n");
fs.writeFileSync(a[a.indexOf("-o") + 1], "aiff");`);
  script("afconvert", "fs.copyFileSync(process.env.FAKE_WAV, a.at(-1));");
  const preload = path.join(root, "fake-fetch.mjs");
  fs.writeFileSync(preload, `globalThis.fetch = async (url, init) => process.env.FAKE_FETCH === "ok"
  ? new Response(Buffer.alloc(48000), { status: 200 })
  : new Response(JSON.stringify({ detail: { status: "quota_exceeded", message: "key " + init.headers["xi-api-key"] } }), { status: 401 });\n`);
  const xdg = path.join(root, "xdg");
  const cache = cacheDir({ XDG_CACHE_HOME: xdg }, cacheKey(path.join(SKILL, "video")));
  fs.mkdirSync(cache, { recursive: true });
  fs.writeFileSync(path.join(cache, ".ready"), "");
  fs.writeFileSync(path.join(cache, "engine.mjs"), `import fs from "node:fs";
const a = process.argv.slice(2);
fs.copyFileSync(a[a.indexOf("--input") + 1], process.env.FAKE_ENGINE_INPUT);
fs.writeFileSync(a[a.indexOf("--out") + 1], "mp4");\n`);
  const out = path.join(root, "out");
  const env = {
    ...process.env, PATH: `${bin}:${process.env.PATH}`, XDG_CACHE_HOME: xdg, VE_TTS_VOICE: "",
    FAKE_WAV: wav, FAKE_SAY_LOG: path.join(root, "say.log"), FAKE_ENGINE_INPUT: path.join(root, "engine-input.json"),
  };
  const render = (args, extraEnv) => spawnSync(process.execPath, ["--import", preload, RENDER_VIDEO,
    "--props", path.join(SKILL, "examples", "plan-visual-explainer.props.json"), "--narration", path.join(SKILL, "examples", "plan-visual-explainer.narration.json"),
    "--out", out, ...args], { encoding: "utf8", env: { ...env, ...extraEnv }, timeout: 60_000 });
  const sayCalls = () => (fs.existsSync(env.FAKE_SAY_LOG) ? fs.readFileSync(env.FAKE_SAY_LOG, "utf8").trim().split("\n") : []);
  return { root, out, render, sayCalls, engineInput: () => JSON.parse(fs.readFileSync(env.FAKE_ENGINE_INPUT, "utf8")) };
}

function treeContains(dir, needle) {
  return fs.readdirSync(dir, { recursive: true }).some((rel) => {
    const file = path.join(dir, rel);
    return rel.includes(needle) || (fs.statSync(file).isFile() && fs.readFileSync(file).includes(needle));
  });
}

const PLAN_SECTIONS = EXAMPLE_NARRATION.plan.sections.length;

test("render-video: when ElevenLabs fails in auto mode, the video uses say, one stderr line says so, and the result names the engine", () => {
  const world = fakeWorld();
  const r = world.render([], { ELEVENLABS_API_KEY: KEY, FAKE_FETCH: "401" });
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split("\n"), [path.join(world.out, "video.mp4"), "tts: say (fallback: HTTP 401 quota_exceeded)"]);
  assert.deepEqual(r.stderr.split("\n").filter((l) => l.includes("ElevenLabs failed")),
    ["render-video: ElevenLabs failed (HTTP 401 quota_exceeded): the video uses the macOS voice"]);
  assert.match(r.stderr, new RegExp(`render-video: tts: say voice Samantha model say: ${PLAN_SECTIONS} clips, 0 cached`));
  assert.equal(world.sayCalls().length, PLAN_SECTIONS, "say speaks every section");
  assert.deepEqual(Object.keys(world.engineInput().voiceover).sort(), EXAMPLE_NARRATION.plan.sections.map((s) => s.id).sort());
  assert.ok(![r.stdout, r.stderr].some((text) => text.includes(KEY)), "no key in stdout or stderr");
  assert.ok(!treeContains(world.root, KEY), "no key in a file name or a file");
});

test("render-video: an explicit --tts elevenlabs still fails with exit 3 and writes no video", () => {
  const world = fakeWorld();
  const r = world.render(["--tts", "elevenlabs"], { ELEVENLABS_API_KEY: KEY, FAKE_FETCH: "401" });
  assert.equal(r.status, 3, r.stderr);
  assert.match(r.stderr, /render-video\.mjs: text to speech failed for section intro: ElevenLabs returned HTTP 401/);
  assert.ok(!r.stderr.includes("macOS voice"));
  assert.equal(r.stdout, "");
  assert.deepEqual(fs.readdirSync(world.out), [], "no video");
  assert.deepEqual(world.sayCalls(), []);
  assert.ok(!r.stderr.includes(KEY) && !treeContains(world.root, KEY));
});

test("render-video: the result names the engine when no fallback happens", () => {
  const ok = fakeWorld().render([], { ELEVENLABS_API_KEY: KEY, FAKE_FETCH: "ok" });
  assert.equal(ok.status, 0, ok.stderr);
  assert.equal(ok.stdout.trim().split("\n")[1], "tts: elevenlabs");
  assert.ok(!ok.stderr.includes("ElevenLabs failed"));
  const say = fakeWorld().render(["--tts", "say"], { ELEVENLABS_API_KEY: KEY });
  assert.equal(say.status, 0, say.stderr);
  assert.equal(say.stdout.trim().split("\n")[1], "tts: say");
});

// ---- page embed: render.mjs --video ----

// The video was a block at the top of the report. The graph page keeps it in a drawer that the bar opens.
test("page: --video adds one video in a closed drawer, opened from the bar, after the graph and the outline", () => {
  const r = run(RENDER, ["--props", "<props>", "--out", "<out>", "--video", "video.mp4"], { props: EXAMPLES.ship });
  assert.equal(r.status, 0, r.stderr);
  const videos = r.html.match(/<video\b[^>]*>/g) ?? [];
  assert.equal(videos.length, 1, "exactly one video element");
  assert.match(videos[0], /\bcontrols\b/);
  assert.match(videos[0], /\bpreload="metadata"/);
  assert.match(videos[0], /\bplaysinline\b/);
  assert.match(videos[0], /\bsrc="video\.mp4"/);
  assert.match(r.html, /<div class="drawer" id="video-panel" role="dialog" aria-label="Video" hidden>/, "a closed drawer");
  assert.match(r.html, /<button type="button" class="btn" data-video aria-controls="video-panel" aria-expanded="false">Video<\/button>/, "a bar button opens it");
  const graph = r.html.indexOf('<svg class="graph"');
  const outline = r.html.indexOf('id="outline"');
  const video = r.html.indexOf("<video");
  assert.ok(graph > 0 && graph < outline && outline < video, "the video is not at the top");
});

test("page: without --video the page has no video markup", () => {
  for (const props of Object.values(EXAMPLES)) {
    const r = run(RENDER, ["--props", "<props>", "--out", "<out>"], { props });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(!/<video|id="video-panel"|data-video aria-controls/.test(r.html), "no video markup");
  }
});

test("page: --video rejects a path that is absolute, has a '..' segment, a scheme or another extension", () => {
  for (const bad of ["/tmp/video.mp4", "../video.mp4", "a/../../video.mp4", "javascript:alert(1).mp4", "https://x.test/v.mp4",
    "video.webm", "video.mp4.html", "%2e%2e/video.mp4", "a b.mp4", "a\\b.mp4", "a//b.mp4", "x/.."]) {
    const r = run(RENDER, ["--props", "<props>", "--out", "<out>", "--video", bad], { props: EXAMPLES.ship });
    assert.equal(r.status, 2, `${bad}: ${r.stderr}`);
    assert.match(r.stderr, /--video must be a relative path that ends in \.mp4/, bad);
    assert.equal(r.html, null, `${bad}: no page is written`);
  }
  for (const good of ["video.mp4", "media/video-1.mp4", "./video.mp4"]) {
    const r = run(RENDER, ["--props", "<props>", "--out", "<out>", "--video", good], { props: EXAMPLES.plan });
    assert.equal(r.status, 0, `${good}: ${r.stderr}`);
    assert.ok(r.html.includes(`src="${good}"`), good);
  }
});

test("page: --video needs --out", () => {
  const r = run(RENDER, ["--props", "<props>", "--check", "--video", "video.mp4"], { props: EXAMPLES.ship });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /^usage: render\.mjs/);
});

// ---- text cut rule ----

test("clip: short text stays whole, with newlines and runs of spaces folded to one space", () => {
  assert.equal(clip("Serve widgets over HTTP.", 40), "Serve widgets over HTTP.");
  assert.equal(clip("Line one.\nLine   two.", 40), "Line one. Line two.");
});

test("clip: long text is cut at a word boundary and ends in an ellipsis within the limit", () => {
  const text = "The daemon owns sessions, logs, messages, dependency edges, triggers and a local MCP server.";
  assert.equal(clip(text, 40), "The daemon owns sessions, logs,…");
  assert.ok(clip(text, 40).length <= 40);
  assert.equal(clip("Supercalifragilisticexpialidocious", 10), "Supercali…", "one long word is cut inside the word");
});

test("clip: a cut inside inline code closes the code span", () => {
  assert.equal(clip("Run `ws migrate --from-zsh --dry-run` first", 24), "Run `ws migrate…`");
});

// ---- list split rule ----

test("paginate: a list that fits one scene stays in one scene", () => {
  assert.deepEqual(paginate(["a", "b", "c"], 5, 2), { pages: [["a", "b", "c"]], more: 0 });
  assert.deepEqual(paginate([], 5, 2), { pages: [], more: 0 });
});

test("paginate: a longer list splits into balanced scenes in list order", () => {
  assert.deepEqual(paginate([1, 2, 3, 4, 5, 6], 5, 2).pages, [[1, 2, 3], [4, 5, 6]]);
  assert.deepEqual(paginate([1, 2, 3, 4, 5, 6, 7], 5, 3).pages, [[1, 2, 3, 4], [5, 6, 7]]);
});

test("paginate: items past the scene cap are cut and counted", () => {
  const list = Array.from({ length: 14 }, (_, i) => i + 1);
  assert.deepEqual(paginate(list, 5, 2), { pages: [[1, 2, 3, 4, 5], [6, 7, 8, 9, 10]], more: 4 });
});

// ---- storyboard: scenes ----

test("storyboard: 1920x1080 at 30 fps", () => {
  const board = storyboard(planProps());
  assert.deepEqual([board.width, board.height, board.fps], [1920, 1080, 30]);
});

test("storyboard (plan): the scenes mirror the page order, one timeline scene per milestone", () => {
  assert.deepEqual(kinds(storyboard(planProps())),
    ["title", "goal", "architecture", "flows", "timeline", "timeline", "decisions", "risks", "end"]);
  const noFlows = planProps();
  delete noFlows.architecture.flows;
  assert.ok(!kinds(storyboard(noFlows)).includes("flows"), "no flows scene without flows");
});

test("storyboard (ship): the scenes mirror the page order, and an empty deviation list still gets its scene", () => {
  assert.deepEqual(kinds(storyboard(shipProps())),
    ["title", "summary", "delivered", "deviations", "review", "hand-test", "rollback", "end"]);
  const withChanges = shipProps({ changes: [{ area: "Storage", how: "SQLite", why: "One file" }] });
  assert.deepEqual(kinds(storyboard(withChanges)).slice(0, 3), ["title", "summary", "changes"]);
});

test("storyboard: a long list splits across scenes, and the last scene counts the items that only the page shows", () => {
  const delivered = many(14, (i) => ({ criterion: `Criterion ${i}`, result: "Done", status: i === 12 ? "missed" : "met" }));
  const scenes = storyboard(shipProps({ delivered })).scenes.filter((s) => s.kind === "delivered");
  assert.deepEqual(scenes.map((s) => [s.delivered.length, s.more, s.part.index, s.part.count]), [[4, 0, 1, 2], [4, 6, 2, 2]]);
  assert.equal(scenes[0].delivered[0].criterion, "Criterion 12", "a missed row comes first, so a cut drops met rows");
});

test("storyboard: the timeline never mixes milestones and counts the phases past its cap", () => {
  const milestones = many(3, (m) => ({ number: m, title: `M${m}`, phases: many(7, (i) => ({ number: m * 10 + i, title: `Phase ${i}`, kind: "work" })) }));
  const scenes = storyboard(planProps({ milestones })).scenes.filter((s) => s.kind === "timeline");
  assert.deepEqual(scenes.map((s) => [s.milestone.number, s.phases.length]), [[1, 4], [1, 3], [2, 4], [2, 3]]);
  assert.deepEqual(scenes.map((s) => s.more), [0, 0, 0, 7], "milestone 3 is only on the page");
});

test("storyboard: sub-lists keep their first items and count the rest", () => {
  const plan = planProps();
  plan.architecture.layers[0].components = many(6, (i) => ({ name: `C${i}`, role: "r" }));
  plan.decisions[0].rejected.push({ option: "MySQL", reason: "Needs a server." });
  const board = storyboard(plan);
  const arch = board.scenes.find((s) => s.kind === "architecture");
  assert.deepEqual([arch.layers[0].components.length, arch.layers[0].more], [4, 2]);
  assert.equal(board.scenes.find((s) => s.kind === "decisions").decisions[0].moreRejected, 1);

  const ship = shipProps({ handTest: [{ command: "a\nb\n\nc\nd", ref: "src/a.js:1", why: "w" }] });
  ship.reviews[0].findings = [
    { id: "F1", severity: "nit", title: "n", status: "open" },
    { id: "F2", severity: "major", title: "m", status: "fixed" },
    { id: "F3", severity: "major", title: "m", status: "open" },
    { id: "F4", severity: "blocker", title: "b", status: "deferred" },
  ];
  const shipBoard = storyboard(ship);
  const reviewScene = shipBoard.scenes.find((s) => s.kind === "review");
  assert.deepEqual(reviewScene.findings.map((f) => f.id), ["F4", "F3", "F2"], "severity first, then open before fixed");
  assert.equal(reviewScene.moreFindings, 1);
  assert.deepEqual(shipBoard.scenes.find((s) => s.kind === "hand-test").steps[0].command, { lines: ["a", "b"], more: 2 });
});

test("theme: the video colors are the dark color tokens of the page", async () => {
  const { COLORS } = await import(path.join(SKILL, "video", "src", "theme.mjs"));
  const page = fs.readFileSync(path.join(SKILL, "template", "page.html"), "utf8");
  const block = /:root\[data-theme="dark"\] \{([^}]*)\}/.exec(page)[1];
  const tokens = Object.fromEntries([...block.matchAll(/--([a-z0-9-]+):\s*(#[0-9a-f]{6});/g)].map((m) => [m[1], m[2]]));
  assert.ok(Object.keys(tokens).length >= 10, "the page has dark color tokens");
  assert.deepEqual(COLORS, tokens);
});

// ---- storyboard: duration ----

const many = (n, make) => Array.from({ length: n }, (_, i) => make(i + 1));
const longText = (n) => many(n, (i) => `word${i}`).join(" ");

function longShip() {
  return shipProps({
    summary: longText(400),
    changes: many(30, (i) => ({ area: `Area ${i}`, how: longText(80), why: longText(60) })),
    delivered: many(80, (i) => ({ phase: i, criterion: longText(60), result: longText(60), status: i % 7 ? "met" : "partial" })),
    deviations: many(40, () => ({ planned: longText(60), actual: longText(60), reason: longText(60) })),
    process: { phases: 80, attempts: 120, retries: 30, escalations: 4, questions: 3, hil: 2, mergeConflicts: 1, reviewRounds: 4, fixRounds: 3, wallTime: "2 days" },
    setbacks: many(20, (i) => ({ phase: i, kind: "retry", title: longText(20), what: longText(60), cost: "1 attempt", resolution: longText(60), lesson: longText(40) })),
    reviews: many(10, () => ({ verdict: "COMMENT", summary: longText(80), validation: [{ command: "npm test", result: "pass" }],
      findings: many(60, (i) => ({ id: `F${i}`, severity: "minor", title: longText(30), status: "open" })) })),
    handTest: many(40, (i) => ({ title: `Step ${i}`, command: "a\nb\nc\nd", ref: "src/a.js:1", why: longText(60), expect: longText(30) })),
    rollback: many(20, () => ({ step: longText(60), command: "git revert -m1 <sha>" })),
  });
}

function assertContiguous(board) {
  let at = 0;
  for (const scene of board.scenes) {
    assert.equal(scene.from, at, `${scene.kind} starts where the previous scene ends`);
    assert.ok(Number.isInteger(scene.durationInFrames) && scene.durationInFrames > 0);
    at += scene.durationInFrames;
  }
  assert.equal(at, board.durationInFrames, "the scenes fill the whole video");
}

const sceneOf = (board, kind) => board.scenes.find((s) => s.kind === kind);
const sectionOf = (board, id) => board.sections.find((s) => s.id === id);

test("duration: without narration each scene plays its minimum visual time, 5 to 10 s, the title 5 s and the end 4 s", () => {
  for (const props of [planProps(), shipProps(), longShip(), ...Object.values(EXAMPLES)]) {
    const board = storyboard(props);
    assertContiguous(board);
    assert.equal(sceneOf(board, "title").durationInFrames, 5 * 30);
    assert.equal(sceneOf(board, "end").durationInFrames, 4 * 30);
    const content = board.scenes.filter((s) => s.kind !== "title" && s.kind !== "end");
    assert.ok(content.every((s) => s.durationInFrames >= 5 * 30 && s.durationInFrames <= 10 * 30), props.mode);
    assert.ok(board.sections.every((s) => s.audio === null), "no audio");
  }
  const short = sceneOf(storyboard(planProps({ goal: "Serve widgets." })), "goal");
  const long = sceneOf(storyboard(planProps({ goal: longText(40) })), "goal");
  assert.ok(long.durationInFrames > short.durationInFrames, "a longer goal stays on screen longer");
});

test("duration: a narrated section lasts its audio plus 0.6 s, and the audio starts 0.3 s into the section", () => {
  const board = storyboard(planProps(), { goal: { src: "tts/goal.wav", seconds: 20 } });
  const goal = sceneOf(board, "goal");
  assert.equal(goal.durationInFrames, 618, "20.6 s at 30 fps");
  assert.deepEqual(sectionOf(board, "goal").audio, { src: "tts/goal.wav", from: goal.from + 9, durationInFrames: 600 });
  assertContiguous(board);
});

test("duration: a short narration keeps the minimum visual time of its section", () => {
  const silent = sceneOf(storyboard(planProps()), "goal").durationInFrames;
  const board = storyboard(planProps(), { goal: { src: "tts/goal.wav", seconds: 1 } });
  assert.equal(sceneOf(board, "goal").durationInFrames, silent);
});

test("duration: a section with several scenes splits its time between them by their minimum visual time", () => {
  const silent = storyboard(planProps());
  const board = storyboard(planProps(), { architecture: { src: "tts/a.wav", seconds: 40 } });
  const [arch, flows] = ["architecture", "flows"].map((kind) => sceneOf(board, kind));
  assert.equal(arch.durationInFrames + flows.durationInFrames, 1218, "40.6 s at 30 fps");
  const ratio = sceneOf(silent, "architecture").durationInFrames / sceneOf(silent, "flows").durationInFrames;
  assert.ok(Math.abs(arch.durationInFrames / flows.durationInFrames - ratio) < 0.02, "the split keeps the ratio of the minimum times");
  assert.deepEqual(sectionOf(board, "architecture"), {
    id: "architecture", from: arch.from, durationInFrames: 1218, audio: { src: "tts/a.wav", from: arch.from + 9, durationInFrames: 1200 },
  });
  assertContiguous(board);
});

test("duration: another frame rate keeps the same times in seconds", () => {
  const board = storyboard(planProps(), { goal: { src: "tts/goal.wav", seconds: 20 } }, { fps: 24 });
  assert.equal(board.fps, 24);
  assert.equal(sceneOf(board, "goal").durationInFrames, 480 + 14, "20 s of audio plus 0.6 s at 24 fps");
  assert.equal(sectionOf(board, "goal").audio.from, sceneOf(board, "goal").from + 7, "0.3 s lead");
  assert.equal(sceneOf(board, "title").durationInFrames, 5 * 24);
  assertContiguous(board);
});

test("duration: the narration sets the length, with no upper limit", () => {
  const voiceover = Object.fromEntries(["intro", "goal", "architecture", "milestones", "decisions", "risks", "outro"].map((id) => [id, { src: `tts/${id}.wav`, seconds: 60 }]));
  const board = storyboard(planProps(), voiceover);
  assert.equal(board.durationInFrames, 7 * 1818, "7 sections of 60.6 s");
  assertContiguous(board);
});

test("storyboard (ship): process counts and setbacks make one 'Setbacks and hiccups' scene after the deviations", () => {
  const props = shipProps({
    process: { phases: 6, attempts: 9, retries: 2, hil: 1, wallTime: "3 h 10 min" },
    setbacks: [{ phase: 4, kind: "verify-fail", title: "Flaky socket test", what: "Verify failed twice.", cost: "2 attempts", resolution: "A shared fixture.", lesson: "Sandbox sockets." }],
  });
  const board = storyboard(props);
  assert.deepEqual(kinds(board), ["title", "summary", "delivered", "deviations", "setbacks", "review", "hand-test", "rollback", "end"]);
  const scene = board.scenes[4];
  assert.equal(scene.section.title, "Setbacks and hiccups");
  assert.deepEqual(scene.process, [["phases", "6"], ["attempts", "9"], ["retries", "2"], ["HIL", "1"], ["wall time", "3 h 10 min"]], "the labels of the page");
  assert.deepEqual(scene.setbacks.map((s) => [s.phase, s.kind, s.title, s.resolution]), [[4, "verify-fail", "Flaky socket test", "A shared fixture."]]);
  assert.deepEqual(kinds(storyboard(shipProps({ process: { retries: 0 } }))).slice(3, 5), ["deviations", "setbacks"], "process alone");
  const empty = storyboard(shipProps({ setbacks: [] }));
  assert.deepEqual(kinds(empty).slice(3, 5), ["deviations", "setbacks"], "like the page: an empty list still shows the scene");
  assert.deepEqual([empty.scenes[4].process, empty.scenes[4].setbacks], [[], []]);
  assert.deepEqual(kinds(storyboard(shipProps())).slice(3, 5), ["deviations", "review"], "no field, no scene");
});

// ---- real render (VE_VIDEO_E2E=1) ----

const E2E = process.env.VE_VIDEO_E2E === "1";
const FFPROBE = spawnSync("ffprobe", ["-version"]).status === 0;

for (const mode of ["plan", "ship"]) {
  test(`e2e (${mode}): renders the narrated example to an MP4 under the size limit, then embeds it in the page`, {
    skip: !E2E && "set VE_VIDEO_E2E=1 to render real videos with real text to speech", timeout: 20 * 60_000,
  }, () => {
    const r = run(RENDER_VIDEO, ["--props", "<props>", "--narration", "<narration>", "--out", "<out>"], {
      props: EXAMPLES[mode], narration: EXAMPLE_NARRATION[mode], timeout: 18 * 60_000,
    });
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stderr, /render-video: tts: (elevenlabs|say) voice .+: \d+ clips, \d+ cached, \d+ characters sent/);
    if (process.env.ELEVENLABS_API_KEY) assert.ok(!r.stderr.includes(process.env.ELEVENLABS_API_KEY), "the key is never in the log");
    const out = path.join(r.dir, "out");
    const video = path.join(out, "video.mp4");
    const [line, engine, ...rest] = r.stdout.trim().split("\n");
    assert.equal(line, video, "stdout starts with the absolute MP4 path");
    assert.match(engine, /^tts: (elevenlabs|say|say \(fallback: .+\))$/, "then the engine line");
    assert.deepEqual(rest, []);
    assert.deepEqual(fs.readdirSync(out), ["video.mp4"], "no temporary file stays");
    const size = fs.statSync(video).size;
    assert.ok(size > 0 && size <= 14_000_000, `${size} bytes`);
    assert.ok(!["node_modules", "bundle", ".ready"].some((name) => fs.existsSync(path.join(SKILL, "video", name))), "the skill directory stays clean");
    if (FFPROBE) {
      const probe = JSON.parse(spawnSync("ffprobe", ["-v", "error", "-show_streams", "-show_format", "-of", "json", video], { encoding: "utf8" }).stdout);
      const streams = Object.fromEntries(probe.streams.map((s) => [s.codec_type, s]));
      assert.deepEqual(Object.keys(streams).sort(), ["audio", "video"]);
      assert.equal(streams.video.codec_name, "h264");
      assert.ok([[1920, 1080], [1280, 720]].some(([w, h]) => streams.video.width === w && streams.video.height === h));
      assert.equal(streams.audio.codec_name, "aac");
      assert.ok(Number(probe.format.duration) >= 60, `${probe.format.duration} s: the narration sets the length`);
    }
    const page = run(RENDER, ["--props", "<props>", "--out", out, "--video", "video.mp4"], { props: EXAMPLES[mode] });
    assert.equal(page.status, 0, page.stderr);
    assert.match(fs.readFileSync(path.join(out, "index.html"), "utf8"), /<video controls preload="metadata" playsinline src="video\.mp4"/);
  });
}
