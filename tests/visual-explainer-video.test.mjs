import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SKILL = path.join(import.meta.dirname, "..", ".claude", "skills", "visual-explainer");
const RENDER = path.join(SKILL, "scripts", "render.mjs");
const RENDER_VIDEO = path.join(SKILL, "scripts", "render-video.mjs");
const { clip, storyboard } = await import(path.join(SKILL, "video", "src", "storyboard.mjs"));
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
  const attempts = renderAttempts({ fps: 24, concurrency: 8, hardwareAcceleration: "disable", x264Preset: "veryfast", gl: "angle" }, 120, 14);
  assert.deepEqual(attempts.map((a) => [a.scale, a.crf, a.x264Preset, a.videoBitrate]), [[1, 23, "veryfast", undefined], [1, 30, "veryfast", undefined], [2 / 3, 30, "veryfast", undefined]]);
  assert.ok(attempts.every((a) => a.fps === 24 && a.concurrency === 8 && a.hardwareAcceleration === "disable" && a.gl === "angle"));
});

// The graph video moves a camera, so it needs more bytes per second than the slide video did.
// Measured on the graph video of the examples, x264 veryfast, 24 fps, 1920x1080, on an Apple M4:
// plan 165.75 s -> 7.90 MB at CRF 23 · ship 231.5 s -> 14.40 MB at CRF 23, 8.10 MB at CRF 30.
test("encoder: the first x264 quality comes from the video length, so a 4 minute graph video fits 14 MB on the first render", () => {
  const x264 = { fps: 24, concurrency: 8, hardwareAcceleration: "disable", x264Preset: "veryfast", gl: "angle" };
  const crfs = (seconds, maxMb = 14) => renderAttempts(x264, seconds, maxMb).map((a) => [a.label, a.scale, a.crf]);
  const backup = [["1920x1080 at a lower quality", 1, 30], ["1280x720", 2 / 3, 30]];
  assert.deepEqual(crfs(60), [["1920x1080", 1, 23], ...backup], "a short video keeps CRF 23");
  assert.deepEqual(crfs(165.75), [["1920x1080", 1, 23], ...backup], "plan example: 7.9 MB at CRF 23 fits");
  assert.deepEqual(crfs(231.5), [["1920x1080", 1, 26], ...backup], "ship example: 14.4 MB at CRF 23 is above the limit, 8.1 MB at CRF 30");
  assert.deepEqual(crfs(300), [["1920x1080", 1, 29], ...backup], "5 minutes");
  assert.deepEqual(crfs(300, 30), [["1920x1080", 1, 23], ...backup], "a larger limit keeps CRF 23");
  assert.deepEqual(crfs(600), [["1920x1080", 1, 30], ["1280x720", 2 / 3, 30]], "CRF 30 is the lowest first quality, then only 1280x720 is left");
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

// The narrated stops of the plan example: one clip each. The narration was a list of sections; it
// is now a list of stops, so the clip ids are the stop ids.
const PLAN_EXAMPLE_FILE = path.join(SKILL, "examples", "plan-visual-explainer.narration.json");
const PLAN_STOPS = checkNarration(PLAN_EXAMPLE_FILE, EXAMPLES.plan).stops.filter((s) => s.text);

test("render-video: when ElevenLabs fails in auto mode, the video uses say, one stderr line says so, and the result names the engine", () => {
  const world = fakeWorld();
  const r = world.render([], { ELEVENLABS_API_KEY: KEY, FAKE_FETCH: "401" });
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split("\n"), [path.join(world.out, "video.mp4"), "tts: say (fallback: HTTP 401 quota_exceeded)"]);
  assert.deepEqual(r.stderr.split("\n").filter((l) => l.includes("ElevenLabs failed")),
    ["render-video: ElevenLabs failed (HTTP 401 quota_exceeded): the video uses the macOS voice"]);
  assert.match(r.stderr, new RegExp(`render-video: tts: say voice Samantha model say: ${PLAN_STOPS.length} clips, 0 cached`));
  assert.equal(world.sayCalls().length, PLAN_STOPS.length, "say speaks every stop");
  assert.deepEqual(Object.keys(world.engineInput().voiceover).sort(), PLAN_STOPS.map((s) => s.id).sort());
  assert.ok(![r.stdout, r.stderr].some((text) => text.includes(KEY)), "no key in stdout or stderr");
  assert.ok(!treeContains(world.root, KEY), "no key in a file name or a file");
});

test("render-video: the composition gets the graph of the page and the resolved stops", () => {
  const world = fakeWorld();
  const r = world.render(["--tts", "say"], {});
  assert.equal(r.status, 0, r.stderr);
  const { props } = world.engineInput();
  const graph = buildGraph(EXAMPLES.plan);
  assert.deepEqual(Object.keys(props).sort(), ["graph", "kinds", "meta", "mode", "stops"]);
  assert.deepEqual(props.kinds.phase, { label: "Phases", color: "accent" }, "the kind labels and colors of graph.mjs");
  assert.deepEqual(props.graph.nodes.map((n) => [n.id, n.x, n.y]), graph.nodes.map((n) => [n.id, n.x, n.y]), "the positions of the page");
  assert.deepEqual(props.stops.map((s) => s.id), checkNarration(PLAN_EXAMPLE_FILE, EXAMPLES.plan).stops.map((s) => s.id));
  assert.ok(props.stops.every((s) => Array.isArray(s.nodes) && typeof s.overview === "boolean"));
});

test("render-video: an explicit --tts elevenlabs still fails with exit 3 and writes no video", () => {
  const world = fakeWorld();
  const r = world.render(["--tts", "elevenlabs"], { ELEVENLABS_API_KEY: KEY, FAKE_FETCH: "401" });
  assert.equal(r.status, 3, r.stderr);
  assert.match(r.stderr, new RegExp(`render-video\\.mjs: text to speech failed for section ${PLAN_STOPS[0].id}: ElevenLabs returned HTTP 401`));
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

// ---- storyboard: the camera moves over the graph from stop to stop ----
// The video had one scene per report section, with lists split across scenes. Now it draws the
// graph of the page and moves a camera between the stops of the narration: these tests replace the
// scene and list-split tests.

const { buildGraph } = await import(path.join(SKILL, "scripts", "graph.mjs"));
const { tourStops } = await import(NARRATION);
const { cameraAt, CARD, GRAPH_AREA, MAX_SCALE } = await import(path.join(SKILL, "video", "src", "storyboard.mjs"));

// The input props of a composition: what render-video.mjs passes.
function videoInput(props, narration = null) {
  const graph = buildGraph(props);
  return { mode: props.mode, meta: props.meta, graph, stops: tourStops(graph, narration, props) };
}
const planStops = (stops) => videoInput(planProps(), { version: 2, mode: "plan", stops });
const ids = (board) => board.stops.map((s) => s.id);
const stopOf = (board, id) => board.stops.find((s) => s.id === id);

// The screen box of a world box under a camera: the camera point sits at the frame center.
function onScreen(camera, box) {
  return { x: (box.x - camera.x) * camera.scale + 960, y: (box.y - camera.y) * camera.scale + 540, w: box.w * camera.scale, h: box.h * camera.scale };
}
function inside(inner, outer, slack = 1) {
  return inner.x >= outer.x - slack && inner.y >= outer.y - slack && inner.x + inner.w <= outer.x + outer.w + slack && inner.y + inner.h <= outer.y + outer.h + slack;
}

test("storyboard: 1920x1080 at 30 fps by default", () => {
  const board = storyboard(videoInput(planProps()));
  assert.deepEqual([board.width, board.height, board.fps], [1920, 1080, 30]);
});

test("storyboard: one stop for each narration stop, with a title stop and an end stop when the narration does not start and end on the overview", () => {
  const board = storyboard(planStops([stop(["goal"]), stop(["phase-2"])]));
  assert.deepEqual(ids(board), ["title", "stop-1", "stop-2", "end"]);
  assert.deepEqual(board.stops.map((s) => [s.card.type, s.overview]), [["title", true], ["node", false], ["node", false], ["end", true]]);
  const framed = storyboard(planStops([stop(["all"]), stop(["phase-2"]), stop(["all"])]));
  assert.deepEqual(ids(framed), ["stop-1", "stop-2", "stop-3"], "an overview at the start and at the end carries the title and the end card");
  assert.deepEqual(framed.stops.map((s) => s.card.type), ["title", "node", "end"]);
});

test("storyboard: a version 1 narration and the default tour visit every cluster", () => {
  const v1 = storyboard(videoInput(planProps(), { mode: "plan", sections: [{ id: "intro", text: "Hi." }] }));
  assert.deepEqual(ids(v1), ["intro", "goal", "architecture", "milestones", "decisions", "risks", "outro"]);
  assert.deepEqual(stopOf(v1, "milestones").nodes, ["milestone-1", "phase-1", "phase-2", "milestone-2", "phase-3"]);
  const ship = storyboard(videoInput(shipProps()));
  assert.ok(ids(ship).includes("group-hand-test") && ids(ship).includes("group-review"));
});

test("storyboard: the camera frames the focus nodes in the graph area, left of the card, up to the zoom limit", () => {
  const board = storyboard(planStops([stop(["all"]), stop(["phase-2"]), stop(["risk-1", "component-widget-store", "milestone-2"]), stop(["group-timeline"])]));
  const graph = buildGraph(planProps());
  const box = (idsOf) => {
    const nodes = graph.nodes.filter((n) => idsOf.includes(n.id));
    const x = Math.min(...nodes.map((n) => n.x));
    const y = Math.min(...nodes.map((n) => n.y));
    return { x, y, w: Math.max(...nodes.map((n) => n.x + n.w)) - x, h: Math.max(...nodes.map((n) => n.y + n.h)) - y };
  };
  assert.ok(GRAPH_AREA.x + GRAPH_AREA.w <= CARD.x, "the card is right of the graph area");
  for (const s of board.stops) {
    const target = s.overview ? { x: 0, y: 0, w: graph.width, h: graph.height } : box(s.nodes);
    const shown = onScreen(s.camera, target);
    assert.ok(inside(shown, GRAPH_AREA), `${s.id}: ${JSON.stringify(shown)} inside the graph area`);
    assert.ok(s.camera.scale <= MAX_SCALE, `${s.id}: zoom ${s.camera.scale}`);
    const fills = Math.max(shown.w / GRAPH_AREA.w, shown.h / GRAPH_AREA.h);
    assert.ok(s.camera.scale === MAX_SCALE || fills > 0.97, `${s.id}: the focus fills the area (${fills.toFixed(2)})`);
  }
  assert.equal(stopOf(board, "stop-2").camera.scale, MAX_SCALE, "one node zooms to the limit");
});

test("storyboard: the camera reaches each stop after its transition and holds it to the end of the stop", () => {
  const board = storyboard(planStops([stop(["all"]), stop(["phase-2"]), stop(["risk-1"]), stop(["group-architecture"]), stop(["all"])]));
  const same = (a, b) => Math.abs(a.x - b.x) < 1e-6 && Math.abs(a.y - b.y) < 1e-6 && Math.abs(a.scale - b.scale) < 1e-9;
  board.stops.forEach((s, i) => {
    assert.ok(s.transitionInFrames < s.durationInFrames, `${s.id}: the move ends inside the stop`);
    assert.ok(same(cameraAt(board, s.from + s.transitionInFrames), s.camera), `${s.id}: the camera arrives`);
    assert.ok(same(cameraAt(board, s.from + s.durationInFrames - 1), s.camera), `${s.id}: the camera holds`);
    if (i) assert.ok(same(cameraAt(board, s.from), board.stops[i - 1].camera), `${s.id}: the move starts at the previous stop`);
  });
  const s = board.stops[1];
  const midway = cameraAt(board, s.from + Math.floor(s.transitionInFrames / 2));
  assert.ok(midway.scale > board.stops[0].camera.scale && midway.scale < s.camera.scale, "the zoom moves between the two stops");
  assert.equal(board.stops[0].transitionInFrames, 0, "the first stop starts in place");
});

test("storyboard: the card shows the key facts of one node, or a list of the focus nodes", () => {
  const props = planProps();
  props.risks[0].mitigation = "WAL mode. ".repeat(40);
  const board = storyboard(videoInput(props, { version: 2, mode: "plan", stops: [stop(["risk-1"]), stop(["group-timeline"], "All phases.", { title: "The timeline" })] }));
  const one = stopOf(board, "stop-1").card;
  assert.equal(one.type, "node");
  assert.deepEqual([one.kind, one.title], ["risk", "Lock contention"]);
  assert.deepEqual(one.facts.map((f) => f.label), ["Impact", "Mitigation"]);
  assert.ok(one.facts[1].text.length <= 160 && one.facts[1].text.endsWith("…"), "a long fact is cut");
  const list = stopOf(board, "stop-2").card;
  assert.equal(list.type, "list");
  assert.equal(list.title, "The timeline");
  assert.deepEqual(list.items.map((i) => i.id), ["milestone-1", "phase-1", "phase-2", "milestone-2", "phase-3"]);
  assert.equal(list.more, 0);
  const big = storyboard(videoInput(planProps(), { version: 2, mode: "plan", stops: [stop(["all"]), stop(["goal", "milestone-1", "milestone-2", "phase-1", "phase-2", "phase-3", "risk-1", "decision-1"])] }));
  assert.deepEqual([stopOf(big, "stop-2").card.items.length, stopOf(big, "stop-2").card.more], [6, 2]);
  assert.equal(stopOf(big, "stop-1").card.type, "title");
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

function assertContiguous(board) {
  let at = 0;
  for (const s of board.stops) {
    assert.equal(s.from, at, `${s.id} starts where the previous stop ends`);
    assert.ok(Number.isInteger(s.durationInFrames) && s.durationInFrames > 0);
    at += s.durationInFrames;
  }
  assert.equal(at, board.durationInFrames, "the stops fill the whole video");
}

test("duration: without audio each stop plays its minimum visual time, 5 to 9 s; the added title stop 5 s and end stop 4 s", () => {
  for (const input of [videoInput(planProps()), videoInput(shipProps()), ...Object.values(EXAMPLES).map((p) => videoInput(p))]) {
    const board = storyboard(input);
    assertContiguous(board);
    assert.ok(board.stops.every((s) => s.durationInFrames >= 5 * 30 && s.durationInFrames <= 9 * 30 && s.audio === null), input.mode);
  }
  const added = storyboard(planStops([stop(["goal"])]));
  assert.deepEqual(added.stops.map((s) => s.durationInFrames / 30), [5, stopOf(added, "stop-1").durationInFrames / 30, 4]);
});

test("duration: a narrated stop lasts its audio plus 0.6 s, and the audio starts 0.3 s into the stop", () => {
  const board = storyboard(planStops([stop(["all"]), stop(["goal"])]), { "stop-2": { src: "tts/b.wav", seconds: 20 } });
  const s = stopOf(board, "stop-2");
  assert.equal(s.durationInFrames, 618, "20.6 s at 30 fps");
  assert.deepEqual(s.audio, { src: "tts/b.wav", from: s.from + 9, durationInFrames: 600 });
  assertContiguous(board);
});

test("duration: a short audio keeps the minimum visual time, and another frame rate keeps the times in seconds", () => {
  const silent = stopOf(storyboard(planStops([stop(["all"]), stop(["goal"])])), "stop-2").durationInFrames;
  const short = storyboard(planStops([stop(["all"]), stop(["goal"])]), { "stop-2": { src: "tts/b.wav", seconds: 1 } });
  assert.equal(stopOf(short, "stop-2").durationInFrames, silent);
  const at24 = storyboard(planStops([stop(["all"]), stop(["goal"])]), { "stop-2": { src: "tts/b.wav", seconds: 20 } }, { fps: 24 });
  assert.equal(at24.fps, 24);
  assert.equal(stopOf(at24, "stop-2").durationInFrames, 480 + 14, "20 s of audio plus 0.6 s at 24 fps");
  assert.equal(stopOf(at24, "stop-2").audio.from, stopOf(at24, "stop-2").from + 7, "0.3 s lead");
  assertContiguous(at24);
});

test("duration: the narration sets the length, with no upper limit", () => {
  const stops = Array.from({ length: 7 }, (_, i) => stop([i % 2 ? "goal" : "all"]));
  const voiceover = Object.fromEntries(stops.map((_, i) => [`stop-${i + 1}`, { src: `tts/${i}.wav`, seconds: 60 }]));
  const board = storyboard(planStops(stops), voiceover);
  assert.equal(board.durationInFrames, 7 * 1818, "7 stops of 60.6 s");
  assertContiguous(board);
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
