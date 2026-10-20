import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SKILL = path.join(import.meta.dirname, "..", ".claude", "skills", "visual-explainer");
const RENDER_VIDEO = path.join(SKILL, "scripts", "render-video.mjs");
const { checkpointOf, clip, modeLabel, sceneData, storyboard, terminalLines, verdictOf, TERMINAL } = await import(path.join(SKILL, "video", "src", "storyboard.mjs"));
const { cacheDir, cacheKey, findChrome, parseArgs, pngSize, previewArgs, pruneCaches, renderAttempts, videoSettings } = await import(RENDER_VIDEO);
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "visual-explainer-video-"));
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

function run(script, args, { props, narration, env = {}, timeout } = {}) {
  const dir = fs.mkdtempSync(path.join(TMP, "case-"));
  const propsFile = path.join(dir, "props.json");
  const narrationFile = path.join(dir, "narration.json");
  if (props !== undefined) fs.writeFileSync(propsFile, typeof props === "string" ? props : JSON.stringify(props));
  if (narration !== undefined) fs.writeFileSync(narrationFile, JSON.stringify(narration));
  const argv = args.map((a) => a.replace("<props>", propsFile).replace("<narration>", narrationFile).replace("<out>", path.join(dir, "out")));
  const res = spawnSync(process.execPath, [script, ...argv], { encoding: "utf8", env: { ...process.env, ...env }, timeout });
  return { ...res, dir };
}

// ---- narration v3: one text for each scene ----

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

test("narration: a valid narration resolves to its scenes in video order", () => {
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
    props: shipProps(), narration: v3("ship", [scene("intro", 80), { scene: "outro", text: "" }]),
  });
  assert.equal(r.status, 1, r.stderr);
  assert.deepEqual(r.stderr.trim().split("\n"), ["narration.scenes[1].text: must not be empty"]);
  const second = run(RENDER_VIDEO, ["--props", "<props>", "--narration", "<narration>", "--check"], {
    props: shipProps(), narration: v3("ship", [scene("intro", 40), scene("findings", 40)]),
  });
  assert.equal(second.status, 1);
  assert.deepEqual(second.stderr.trim().split("\n"), ['narration.scenes[1].scene: the props have no data for "findings" ($.reviews[].findings)']);
  assert.ok(!fs.existsSync(path.join(r.dir, "out")), "no output directory");
});

test("render-video: --check validates the props and the narration of either mode and makes nothing", () => {
  const cache = fs.mkdtempSync(path.join(TMP, "cache-"));
  for (const mode of Object.keys(EXAMPLES)) {
    const r = spawnSync(process.execPath, [RENDER_VIDEO, "--props", EXAMPLE_PROPS_FILE[mode], "--narration", EXAMPLE_NARRATION_FILE[mode], "--check"],
      { encoding: "utf8", env: { ...process.env, XDG_CACHE_HOME: cache } });
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

test("encoder: the defaults are 30 fps, x264 veryfast, ANGLE and all cores but 2", () => {
  assert.deepEqual(videoSettings({}),
    { fps: 30, concurrency: Math.max(1, os.availableParallelism() - 2), hardwareAcceleration: "disable", x264Preset: "veryfast", gl: "angle" });
});

test("encoder: VE_VIDEO_* variables override each default, and a bad value is an error", () => {
  assert.deepEqual(videoSettings({ VE_VIDEO_FPS: "24", VE_VIDEO_CONCURRENCY: "8", VE_VIDEO_HWACCEL: "if-possible", VE_VIDEO_X264_PRESET: "medium", VE_VIDEO_GL: "swangle" }),
    { fps: 24, concurrency: 8, hardwareAcceleration: "if-possible", x264Preset: "medium", gl: "swangle" });
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
  const attempts = renderAttempts({ fps: 30, concurrency: 8, hardwareAcceleration: "disable", x264Preset: "veryfast", gl: "angle" }, 120, 14);
  assert.deepEqual(attempts.map((a) => [a.scale, a.crf, a.x264Preset, a.videoBitrate]), [[1, 23, "veryfast", undefined], [1, 30, "veryfast", undefined], [2 / 3, 30, "veryfast", undefined]]);
  assert.ok(attempts.every((a) => a.fps === 30 && a.concurrency === 8 && a.hardwareAcceleration === "disable" && a.gl === "angle"));
});

// The scene video is short: the size model only matters for a long narration. The model stays
// conservative (render-video.mjs X264 has the measured numbers).
test("encoder: the first x264 quality comes from the video length and the size limit", () => {
  const x264 = { fps: 30, concurrency: 8, hardwareAcceleration: "disable", x264Preset: "veryfast", gl: "angle" };
  const crfs = (seconds, maxMb = 14) => renderAttempts(x264, seconds, maxMb).map((a) => [a.label, a.scale, a.crf]);
  const backup = [["1920x1080 at a lower quality", 1, 30], ["1280x720", 2 / 3, 30]];
  assert.deepEqual(crfs(60), [["1920x1080", 1, 23], ...backup], "a short video keeps CRF 23");
  assert.deepEqual(crfs(180), [["1920x1080", 1, 23], ...backup], "the longest concise video keeps CRF 23");
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
  const settings = { fps: 30, concurrency: 8, hardwareAcceleration: "if-possible", x264Preset: "veryfast", gl: "angle" };
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

test("cache key: a change to the lock file, a source file or a vendored snapcn file changes the key", () => {
  const base = cacheKey(path.join(SKILL, "video"));
  const lock = copyVideoProject();
  fs.appendFileSync(path.join(lock, "package-lock.json"), "\n");
  assert.notEqual(cacheKey(lock), base, "lock file");
  const source = copyVideoProject();
  fs.appendFileSync(path.join(source, "src", "theme.mjs"), "\n");
  assert.notEqual(cacheKey(source), base, "source file");
  const vendored = copyVideoProject();
  fs.appendFileSync(path.join(vendored, "src", "components", "snap-cn", "word-captions.jsx"), "\n");
  assert.notEqual(cacheKey(vendored), base, "vendored snapcn file");
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

// ---- render-video.mjs: screenshots and the preview ----

test("screenshots: findChrome finds the browser of the cache, pngSize reads a PNG header", () => {
  const dir = fs.mkdtempSync(path.join(TMP, "chrome-"));
  assert.equal(findChrome(dir), null, "no browser yet");
  const bin = path.join(dir, "node_modules", ".remotion", "chrome-headless-shell", "mac-arm64", "chrome-headless-shell-mac-arm64");
  fs.mkdirSync(bin, { recursive: true });
  fs.writeFileSync(path.join(bin, "chrome-headless-shell"), "");
  assert.equal(findChrome(dir), path.join(bin, "chrome-headless-shell"));
  const png = Buffer.alloc(24);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(png);
  png.writeUInt32BE(2200, 16);
  png.writeUInt32BE(1400, 20);
  assert.deepEqual(pngSize(png), { width: 2200, height: 1400 });
  assert.equal(pngSize(Buffer.from("GIF89a and more bytes here")), null);
});

test("preview: one input of 1.5 s from the middle of each scene, then fps, width and a palette", () => {
  const board = { fps: 30, scenes: [{ from: 0, durationInFrames: 150 }, { from: 150, durationInFrames: 600 }, { from: 750, durationInFrames: 30 }] };
  const args = previewArgs("v.mp4", board, { fps: 10, width: 720 }, "out.gif");
  const inputs = [];
  for (let i = 0; i < args.length; i += 1) if (args[i] === "-ss") inputs.push([args[i + 1], args[i + 3], args[i + 5]]);
  assert.deepEqual(inputs, [["1.75", "1.5", "v.mp4"], ["14.25", "1.5", "v.mp4"], ["24.75", "1.5", "v.mp4"]]);
  const filter = args[args.indexOf("-filter_complex") + 1];
  assert.match(filter, /^\[0:v\]\[1:v\]\[2:v\]concat=n=3:v=1:a=0,fps=10,scale=720:-1:flags=lanczos,split\[a\]\[b\];\[a\]palettegen/);
  assert.equal(args.at(-1), "out.gif");
});

// ---- render-video.mjs: the text to speech engine, end to end with fakes ----

// A fake world for one render-video.mjs process: a fetch preload (FAKE_FETCH=ok|401), `say` and
// `afconvert` on PATH, a browser path that does not exist (the screenshots fail and only warn),
// and a ready cache whose engine.mjs records its input and writes a tiny MP4.
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
    ...process.env, PATH: `${bin}:${process.env.PATH}`, XDG_CACHE_HOME: xdg, VE_TTS_VOICE: "", VE_CHROME: path.join(root, "no-browser"),
    FAKE_WAV: wav, FAKE_SAY_LOG: path.join(root, "say.log"), FAKE_ENGINE_INPUT: path.join(root, "engine-input.json"),
  };
  const render = (args, extraEnv) => spawnSync(process.execPath, ["--import", preload, RENDER_VIDEO,
    "--props", EXAMPLE_PROPS_FILE.plan, "--narration", EXAMPLE_NARRATION_FILE.plan,
    "--out", out, ...args], { encoding: "utf8", env: { ...env, ...extraEnv }, timeout: 120_000 });
  const sayCalls = () => (fs.existsSync(env.FAKE_SAY_LOG) ? fs.readFileSync(env.FAKE_SAY_LOG, "utf8").trim().split("\n") : []);
  return { root, out, render, sayCalls, engineInput: () => JSON.parse(fs.readFileSync(env.FAKE_ENGINE_INPUT, "utf8")) };
}

function treeContains(dir, needle) {
  return fs.readdirSync(dir, { recursive: true }).some((rel) => {
    const file = path.join(dir, rel);
    return rel.includes(needle) || (fs.statSync(file).isFile() && fs.readFileSync(file).includes(needle));
  });
}

// The scenes of the plan example: one clip each, with the scene id as the clip id.
const PLAN_SCENES = checkNarration(EXAMPLE_NARRATION_FILE.plan, EXAMPLES.plan).scenes;

test("render-video: when ElevenLabs fails in auto mode, the video uses say, one stderr line says so, and the result names the engine", () => {
  const world = fakeWorld();
  const r = world.render([], { ELEVENLABS_API_KEY: KEY, FAKE_FETCH: "401" });
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split("\n"), [path.join(world.out, "video.mp4"), "tts: say (fallback: HTTP 401 quota_exceeded)"]);
  assert.deepEqual(r.stderr.split("\n").filter((l) => l.includes("ElevenLabs failed")),
    ["render-video: ElevenLabs failed (HTTP 401 quota_exceeded): the video uses the macOS voice"]);
  assert.match(r.stderr, new RegExp(`render-video: tts: say voice Samantha model say: ${PLAN_SCENES.length} clips, 0 cached`));
  assert.equal(world.sayCalls().length, PLAN_SCENES.length, "say speaks every scene");
  assert.deepEqual(Object.keys(world.engineInput().voiceover).sort(), PLAN_SCENES.map((s) => s.id).sort());
  assert.ok(![r.stdout, r.stderr].some((text) => text.includes(KEY)), "no key in stdout or stderr");
  assert.ok(!treeContains(world.root, KEY), "no key in a file name or a file");
});

test("render-video: the composition gets the props, the narration scenes and the screenshots; failed screenshots and a failed GIF only warn", () => {
  const world = fakeWorld();
  const r = world.render(["--tts", "say"], {});
  assert.equal(r.status, 0, r.stderr);
  const { props, video } = world.engineInput();
  assert.deepEqual(Object.keys(props).sort(), ["mode", "props", "scenes", "shots"]);
  assert.deepEqual(props.props, EXAMPLES.plan);
  assert.deepEqual(props.scenes, PLAN_SCENES);
  assert.deepEqual(props.shots, {}, "the browser does not exist, so no screenshot");
  assert.match(r.stderr, /render-video: screenshots skipped, the scenes draw their native view: screenshots failed/);
  assert.match(r.stderr, /render-video: preview\.gif skipped: /, "the fake MP4 is not a video");
  assert.equal(video.fps, 30);
  assert.deepEqual(fs.readdirSync(world.out), ["video.mp4"], "no temporary file stays");
});

test("render-video: an explicit --tts elevenlabs still fails with exit 3 and writes no video", () => {
  const world = fakeWorld();
  const r = world.render(["--tts", "elevenlabs"], { ELEVENLABS_API_KEY: KEY, FAKE_FETCH: "401" });
  assert.equal(r.status, 3, r.stderr);
  assert.match(r.stderr, new RegExp(`render-video\\.mjs: text to speech failed for section ${PLAN_SCENES[0].id}: ElevenLabs returned HTTP 401`));
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

// ---- storyboard: scenes, timing and captions ----

// The input props of a composition: what render-video.mjs passes.
const input = (props, ids, shots = {}) => ({ mode: props.mode, props, scenes: ids.map((id) => ({ id, text: `The ${id} scene, in eleven words of plain spoken narration text.` })), shots });
const PLAN_ALL = SCENES.plan;
const SHIP_ALL = SCENES.ship;
const richPlan = () => planProps({ rabbitHoles: ["No mobile layout.", "Do not tune the cache."] });
const finding = (severity, status, title = `${severity} ${status}`) => ({ severity, status, title });
const richShip = () => shipProps({
  stats: { files: 9, additions: 812, deletions: 14 },
  changes: Array.from({ length: 6 }, (_, i) => ({ area: `Area ${i + 1}`, how: "h", why: "w", files: Array.from({ length: i + 1 }, (_, j) => ({ path: `f${j}.ts`, additions: j, deletions: 0 })) })),
  delivered: [
    { criterion: "a", result: "ra", status: "met" }, { criterion: "b", result: "rb", status: "partial" },
    { criterion: "c", result: "rc", status: "met" }, { criterion: "d", result: "rd", status: "missed" },
  ],
  reviews: [
    { verdict: "COMMENT", summary: "s", validation: [], findings: [finding("minor", "open"), finding("nit", "fixed"), finding("major", "fixed"), finding("minor", "deferred")] },
    { verdict: "APPROVE", summary: "s", validation: [], findings: [finding("style", "open"), finding("minor", "fixed")] },
  ],
});
const dataOf = (board, id) => board.scenes.find((s) => s.id === id).data;

function assertContiguous(board) {
  let at = 0;
  for (const s of board.scenes) {
    assert.equal(s.from, at, `${s.id} starts where the previous scene ends`);
    assert.ok(Number.isInteger(s.durationInFrames) && s.durationInFrames > 0);
    at += s.durationInFrames;
  }
  assert.equal(at, board.durationInFrames, "the scenes fill the whole video");
}

test("storyboard: 1920x1080 at 30 fps, one scene for each narration scene, numbered like the report sections", () => {
  const board = storyboard(input(richPlan(), PLAN_ALL));
  assert.deepEqual([board.width, board.height, board.fps], [1920, 1080, 30]);
  assert.deepEqual(board.scenes.map((s) => [s.id, s.number]), [
    ["intro", null], ["tldr", null], ["done-when", 1], ["phases", 2], ["architecture", 3], ["decisions", 4], ["rabbit-holes", 5], ["outro", null],
  ]);
  assert.deepEqual(board.scenes.map((s) => s.title), ["", "", "Done when", "Phases", "Architecture: before / after", "Decisions", "Rabbit holes & no-gos", ""]);
  assert.deepEqual(board.scenes.map((s) => s.label), ["Intro", "TL;DR", "Done when", "Phases", "Architecture: before / after", "Decisions", "Rabbit holes & no-gos", "Wrap-up"]);
  const ship = storyboard(input(richShip(), ["intro", "changes", "rollback"]));
  assert.deepEqual(ship.scenes.map((s) => [s.id, s.number, s.title]), [["intro", null, ""], ["changes", 1, "Changes, in reading order"], ["rollback", 2, "Rollback"]],
    "an omitted scene is not in the video, and the numbers follow the scenes that are");
});

test("duration: without audio each scene plays its minimum visual time, 4 to 20 s", () => {
  for (const board of [storyboard(input(richPlan(), PLAN_ALL)), storyboard(input(richShip(), SHIP_ALL)),
    ...Object.entries(EXAMPLES).map(([mode, props]) => storyboard(input(props, checkNarration(EXAMPLE_NARRATION_FILE[mode], props).scenes.map((s) => s.id))))]) {
    assertContiguous(board);
    assert.ok(board.scenes.every((s) => s.durationInFrames >= 4 * 30 && s.durationInFrames <= 20 * 30 && s.audio === null));
  }
  const outro = storyboard(input(richPlan(), ["outro"])).scenes[0];
  assert.equal(outro.durationInFrames, 198, "the outro plays at least the 198 frames of status-cycle");
  assert.deepEqual(outro.data.statuses, ["planned in 3 phases", "ready for review"]);
  assert.ok(24 + outro.data.statusHold * outro.data.statuses.length <= outro.durationInFrames, "the last status holds to the end: the chip act never starts");
});

test("duration: a narrated scene lasts its audio plus 0.6 s, and the audio starts 0.3 s into the scene", () => {
  const board = storyboard(input(richPlan(), ["intro", "phases"]), { phases: { src: "tts/b.wav", seconds: 20 } });
  const s = board.scenes[1];
  assert.equal(s.durationInFrames, 618, "20.6 s at 30 fps");
  assert.deepEqual(s.audio, { src: "tts/b.wav", from: s.from + 9, durationInFrames: 600 });
  assertContiguous(board);
  const silent = storyboard(input(richPlan(), ["intro", "phases"])).scenes[1].durationInFrames;
  const short = storyboard(input(richPlan(), ["intro", "phases"]), { phases: { src: "tts/b.wav", seconds: 1 } });
  assert.equal(short.scenes[1].durationInFrames, silent, "a short audio keeps the minimum visual time");
  const at24 = storyboard(input(richPlan(), ["intro", "phases"]), { phases: { src: "tts/b.wav", seconds: 20 } }, { fps: 24 });
  assert.deepEqual([at24.fps, at24.scenes[1].durationInFrames, at24.scenes[1].audio.from - at24.scenes[1].from], [24, 480 + 14, 7]);
  assertContiguous(at24);
});

test("captions: each word is timed in proportion to its length across the voice clip of its scene", () => {
  const props = richPlan();
  const text = "One shared skill renders every explainer from a props file.";
  const board = storyboard({ mode: "plan", props, scenes: [{ id: "intro", text }], shots: {} }, { intro: { src: "tts/a.wav", seconds: 6 } });
  const { captions, audio, from } = board.scenes[0];
  assert.deepEqual(captions.map((w) => w.text), text.split(" "));
  assert.equal(captions[0].startFrame, audio.from - from, "the first word starts with the voice");
  assert.equal(captions.at(-1).endFrame, audio.from - from + audio.durationInFrames, "the last word ends with the voice");
  captions.forEach((w, i) => {
    assert.ok(w.endFrame > w.startFrame);
    if (i) assert.equal(w.startFrame, captions[i - 1].endFrame, "no gap and no overlap");
  });
  const long = captions.find((w) => w.text === "explainer");
  const short = captions.find((w) => w.text === "a");
  assert.ok(long.endFrame - long.startFrame > short.endFrame - short.startFrame, "a long word stays longer");
});

test("beats: the tldr beats, the decision pages, the featured changes and the hand-test steps start in order inside their scene", () => {
  const plan = storyboard(input(planProps({ decisions: Array.from({ length: 5 }, (_, i) => ({ decision: `D${i}`, why: "w", rejected: [{ option: "o", reason: "r" }] })) }), ["tldr", "decisions"]));
  const ship = storyboard(input(richShip(), ["changes", "hand-test"]));
  for (const [board, id] of [[plan, "tldr"], [plan, "decisions"], [ship, "changes"], [ship, "hand-test"]]) {
    const s = board.scenes.find((x) => x.id === id);
    const starts = s.data.starts;
    assert.ok(starts.length >= 1 && starts.every((f, i) => f >= 0 && f < s.durationInFrames && (!i || f > starts[i - 1])), `${id}: ${starts}`);
  }
  assert.equal(dataOf(plan, "decisions").pages.length, 2, "5 decisions: pages of 3");
});

// ---- storyboard: plan scene data ----

test("plan data: the TL;DR falls back to the goal and the first 2 risks by impact; tldr wins when present", () => {
  const fallback = sceneData({ mode: "plan", props: planProps() }, "tldr");
  assert.deepEqual(fallback.beats, [{ label: "Goal", text: "Serve widgets over HTTP." }, { label: "Top risks", text: "① Lock contention  ② Disk full" }]);
  const given = sceneData({ mode: "plan", props: planProps({ tldr: { goal: "G", ask: "Approve X.", risks: ["R1"] } }) }, "tldr");
  assert.deepEqual(given.beats.map((b) => b.label), ["Goal", "What I need from you", "Top risks"]);
});

test("plan data: done-when falls back to the phases of the first milestone; phases group by milestone with a review chip", () => {
  assert.deepEqual(sceneData({ mode: "plan", props: planProps() }, "done-when"),
    { columns: ["Criterion", "How we'll check"], rows: [{ cells: ["Add the widget store — The store passes its tests.", "—"] }], more: 0 });
  const many = sceneData({ mode: "plan", props: planProps({ doneWhen: Array.from({ length: 9 }, (_, i) => ({ criterion: `c${i}`, check: "k" })) }) }, "done-when");
  assert.deepEqual([many.rows.length, many.more], [5, 4]);
  const phases = sceneData({ mode: "plan", props: planProps() }, "phases");
  assert.deepEqual(phases.groups.map((g) => g.title), ["Milestone 1 — Read path", "Milestone 2 — Write path"]);
  assert.deepEqual(phases.groups[0].rows.map((r) => [r.title, r.summary, r.estimate, r.review]), [
    ["Add the widget store", "The store passes its tests.", "1 d", false], ["Review milestone 1", "", "", true],
  ]);
  assert.equal(phases.groups[1].rows[0].summary, "one handler", "the summary comes before doneWhen");
});

test("plan data: architecture uses the screenshots when present, else the layers without removed components", () => {
  const shot = { src: "shots/a.png", width: 2200, height: 1400 };
  const withShot = sceneData({ mode: "plan", props: planProps(), shots: { architecture: shot } }, "architecture");
  assert.deepEqual([withShot.shot, withShot.media], [shot, null]);
  const native = sceneData({ mode: "plan", props: planProps() }, "architecture");
  assert.deepEqual(native.layers.map((l) => l.components.map((c) => `${c.name}:${c.status}`)), [["HTTP API:new"], ["Widget store:existing"]]);
});

test("plan data: decision cards have a title, the rejected options and the why; rabbit holes split a leading No", () => {
  const [card] = sceneData({ mode: "plan", props: planProps() }, "decisions").pages[0];
  assert.deepEqual(card, { title: "Use SQLite for the widget store", decision: "Use SQLite for the widget store", why: "One file, no server.", rejected: ["Postgres"] });
  const titled = sceneData({ mode: "plan", props: planProps({ decisions: [{ title: "SQLite", decision: "d", why: "w", rejected: [{ option: "o", reason: "r" }] }] }) }, "decisions");
  assert.equal(titled.pages[0][0].title, "SQLite");
  assert.deepEqual(sceneData({ mode: "plan", props: richPlan() }, "rabbit-holes").items, [{ lead: "No", text: "mobile layout." }, { lead: "", text: "Do not tune the cache." }]);
});

// ---- storyboard: ship scene data ----

test("verdict: changes requested for an open blocker or a REQUEST_CHANGES review, else ready with the worst finding that is not fixed", () => {
  const reviews = (verdict, findings) => ({ reviews: [{ verdict, summary: "s", validation: [], findings }] });
  assert.deepEqual(verdictOf(shipProps()), { tone: "ok", label: "Ready to merge" });
  assert.deepEqual(verdictOf(shipProps(reviews("APPROVE", [finding("blocker", "open"), finding("blocker", "open")]))), { tone: "bad", label: "Changes requested, 2 blocking" });
  assert.deepEqual(verdictOf(shipProps(reviews("REQUEST_CHANGES", [finding("minor", "open")]))), { tone: "bad", label: "Changes requested" });
  assert.deepEqual(verdictOf(shipProps(reviews("COMMENT", [finding("blocker", "fixed"), finding("minor", "open"), finding("nit", "open")]))), { tone: "ok", label: "Ready to merge, 1 minor finding" });
  assert.deepEqual(verdictOf(shipProps(reviews("COMMENT", [finding("major", "deferred"), finding("minor", "open")]))), { tone: "ok", label: "Ready to merge, 1 major finding" });
  assert.deepEqual(verdictOf(richShip()), { tone: "ok", label: "Ready to merge, 2 minor findings" }, "every review counts");
  assert.deepEqual(verdictOf(EXAMPLES.ship), { tone: "ok", label: "Ready to merge, 1 major finding" });
});

test("ship data: the intro has the stats and the verdict; the TL;DR falls back to the summary", () => {
  const intro = sceneData({ mode: "ship", props: richShip() }, "intro");
  assert.deepEqual([intro.chip, intro.stats, intro.verdict.label], ["mode: ship · PR review", { files: 9, additions: 812, deletions: 14 }, "Ready to merge, 2 minor findings"]);
  assert.deepEqual(intro.facts, [{ label: "Date", value: "2026-10-18" }], "without meta.facts: the date");
  assert.deepEqual(sceneData({ mode: "ship", props: shipProps() }, "tldr").beats, [{ label: "What changed", text: "The widget service answers `GET /widgets`." }]);
  const given = sceneData({ mode: "ship", props: shipProps({ tldr: { changed: "C", lookFirst: "L", risk: "R" } }) }, "tldr");
  assert.deepEqual(given.beats.map((b) => b.label), ["What changed", "Look at first", "Risk"]);
});

test("ship data: done-when counts each status and shows the rows that are not met first", () => {
  const data = sceneData({ mode: "ship", props: richShip() }, "done-when");
  assert.deepEqual(data.counts, [{ status: "met", n: 2 }, { status: "partial", n: 1 }, { status: "missed", n: 1 }]);
  assert.deepEqual(data.rows.map((r) => r.cells[0]), ["b", "d", "a", "c"]);
});

test("ship data: changes list every area and feature the first ones, with the sequence change and its screenshot", () => {
  const props = richShip();
  props.changes[5].sequence = { participants: [{ name: "A" }, { name: "B" }], messages: [{ from: "A", to: "B", label: "call" }] };
  const shot = { src: "shots/s.png", width: 1800, height: 900 };
  const data = sceneData({ mode: "ship", props, shots: { sequence: shot } }, "changes");
  assert.deepEqual(data.list.map((c) => c.n), [1, 2, 3, 4, 5, 6]);
  assert.deepEqual(data.featured.map((c) => [c.n, c.sequence]), [[1, false], [2, false], [3, false], [6, true]]);
  assert.deepEqual(data.beats, [{ type: "card", n: 1 }, { type: "card", n: 2 }, { type: "card", n: 3 }, { type: "card", n: 6 }, { type: "shot", n: 6 }],
    "the sequence diagram gets its own beat after its change");
  assert.deepEqual([data.featured[3].files.length, data.featured[3].filesMore], [5, 1]);
  assert.equal(data.sequenceShot, shot);
  const noShot = sceneData({ mode: "ship", props }, "changes");
  assert.equal(noShot.featured[3].sequence, false, "without the screenshot the card shows the file rows");
  assert.equal(noShot.beats.length, 4, "and there is no screenshot beat");
});

test("ship data: findings of every review sort by severity, then status, with a count of each severity", () => {
  const data = sceneData({ mode: "ship", props: richShip() }, "findings");
  assert.deepEqual(data.items.map((f) => `${f.severity}/${f.status}`), ["major/fixed", "minor/open", "minor/deferred", "minor/fixed", "nit/fixed"]);
  assert.equal(data.more, 1);
  assert.deepEqual(data.counts, [{ severity: "major", n: 1 }, { severity: "minor", n: 3 }, { severity: "nit", n: 1 }, { severity: "style", n: 1 }]);
});

test("hand test: the terminal rows wrap long commands, end each step with its expect line, and stop at the row budget", () => {
  const long = `cd ~/dotfile/wsd && cargo build --release && ./target/release/ws migrate --from-zsh --dry-run --verbose`;
  const { lines, shown } = terminalLines([
    { n: 1, title: "Build", command: long, expect: "A plan with 3 sessions." },
    { n: 2, title: "Send", command: "ws send '#N' \"hi\"\n# in the tab of #N:\nws inbox" },
    { n: 3, title: "Wait", command: "x ".repeat(200) },
  ]);
  assert.ok(lines.every((l) => l.text.length <= TERMINAL.cols), "every row fits the terminal width");
  assert.equal(lines[0].text, "# 1. Build");
  assert.ok(lines[1].text.startsWith("$ cd ~/dotfile/wsd") && lines[1].text.endsWith(" \\"), "a long command wraps with a continuation");
  assert.ok(lines[2].text.startsWith("    "));
  assert.deepEqual(lines.filter((l) => l.step === 0).at(-1), { text: "→ A plan with 3 sessions.", type: "success", step: 0 });
  assert.deepEqual(lines.filter((l) => l.step === 1).map((l) => l.text), ["# 2. Send", "$ ws send '#N' \"hi\"", "> # in the tab of #N:", "> ws inbox"]);
  assert.equal(shown, 2, "the third step does not fit");
  assert.ok(lines.length <= TERMINAL.rows);
  const data = sceneData({ mode: "ship", props: EXAMPLES.ship }, "hand-test");
  assert.equal(data.steps[0].title, "Check out the PR head");
  assert.ok(data.terminalSteps >= 1 && data.lines.length <= TERMINAL.rows);
  const titleless = sceneData({ mode: "ship", props: shipProps() }, "hand-test");
  assert.equal(titleless.steps[0].title, "The server listens on port 3000", "no title: the first sentence of why");
});

test("hand test: the terminal delays make the first row of each step start at its beat", () => {
  const board = storyboard(input(EXAMPLES.ship, ["hand-test"]), { "hand-test": { src: "tts/h.wav", seconds: 20 } });
  const { lines, starts } = board.scenes[0].data;
  let at = TERMINAL.leadFrames;
  lines.forEach((line, i) => {
    at += line.delay;
    if (i === 0 || lines[i - 1].step !== line.step) assert.ok(at >= starts[line.step], `step ${line.step} starts at ${at}, its beat is ${starts[line.step]}`);
    at += Math.ceil(line.text.length / (TERMINAL.charsPerFrame * TERMINAL.chunkSize));
  });
});

test("examples: every scene of both examples has data, and the scenes fit the limits of one frame", () => {
  for (const [mode, props] of Object.entries(EXAMPLES)) {
    for (const id of SCENES[mode].filter((s) => hasSceneData(props, s))) {
      const data = sceneData({ mode, props }, id);
      assert.ok(data && typeof data === "object", `${mode} ${id}`);
      if (data.rows) assert.ok(data.rows.length <= 5);
      if (data.items) assert.ok(data.items.length <= 7);
    }
  }
});

test("checkpoint: the intro shows the phase progress instead of a verdict, the header and the outro say so, pending rows come last", () => {
  const props = checkpointProps();
  assert.deepEqual(checkpointOf(props), { number: 2, done: 3, total: 5, since: "" });
  assert.equal(checkpointOf(shipProps()), null);
  const intro = sceneData({ mode: "ship", props }, "intro");
  assert.deepEqual([intro.chip, intro.verdict, intro.checkpoint], ["mode: ship · progress checkpoint", null, { number: 2, done: 3, total: 5, since: "" }]);
  assert.equal(sceneData({ mode: "ship", props: shipProps() }, "intro").checkpoint, null);
  assert.deepEqual([modeLabel({ mode: "ship", props }), modeLabel({ mode: "ship", props: shipProps() }), modeLabel({ mode: "plan", props: planProps() })],
    ["Progress checkpoint", "PR review", "Plan"]);
  assert.deepEqual(sceneData({ mode: "ship", props }, "outro").statuses, ["in progress", "3 of 5 done"]);
  const done = sceneData({ mode: "ship", props }, "done-when");
  assert.deepEqual(done.counts, [{ status: "met", n: 1 }, { status: "partial", n: 1 }, { status: "pending", n: 2 }]);
  assert.deepEqual(done.rows.map((r) => r.cells[0]), ["c", "b", "a", "d"], "attention, then met, then pending");
});

test("theme: the video colors are the dark color tokens of the report page", async () => {
  const { COLORS } = await import(path.join(SKILL, "video", "src", "theme.mjs"));
  const page = fs.readFileSync(path.join(SKILL, "template", "report.html"), "utf8");
  const block = /:root\[data-theme="dark"\] \{([^}]*)\}/.exec(page)[1];
  const tokens = Object.fromEntries([...block.matchAll(/--([a-z0-9-]+):\s*(#[0-9a-f]{6});/g)].map((m) => [m[1], m[2]]));
  assert.ok(Object.keys(tokens).length >= 10, "the report page has dark color tokens");
  assert.deepEqual(COLORS, tokens);
});

test("theme: every tone names a color token", async () => {
  const { COLORS, TONES } = await import(path.join(SKILL, "video", "src", "theme.mjs"));
  for (const [name, [ink, bg]] of Object.entries(TONES)) assert.ok(COLORS[ink] && COLORS[bg], name);
  assert.ok(Object.values(COLORS).every((c) => /^#[0-9a-f]{6}$/.test(c)));
});

test("snapcn: the vendored components carry the MIT notice, and no source loads a Google font at render time", () => {
  const video = path.join(SKILL, "video");
  const notice = fs.readFileSync(path.join(video, "src", "components", "snap-cn", "NOTICE"), "utf8");
  assert.match(notice, /MIT License/);
  assert.match(notice, /snapcn/);
  const components = JSON.parse(fs.readFileSync(path.join(video, "components.json"), "utf8"));
  assert.equal(components.aliases.components, "@/components");
  const sources = fs.readdirSync(path.join(video, "src"), { recursive: true }).filter((f) => /\.(m?js|jsx)$/.test(f));
  for (const rel of sources) {
    const text = fs.readFileSync(path.join(video, "src", rel), "utf8");
    assert.ok(!/from "@remotion\/google-fonts/.test(text), `${rel} imports @remotion/google-fonts`);
  }
  const pkg = JSON.parse(fs.readFileSync(path.join(video, "package.json"), "utf8"));
  assert.ok(!pkg.dependencies["@remotion/google-fonts"]);
  assert.match(fs.readFileSync(path.join(video, "engine.mjs"), "utf8"), /"@": SRC_DIR/, "the @ alias is set in the bundler");
});

// ---- real render (VE_VIDEO_E2E=1) ----

const E2E = process.env.VE_VIDEO_E2E === "1";
const FFPROBE = spawnSync("ffprobe", ["-version"]).status === 0;

for (const mode of ["plan", "ship"]) {
  test(`e2e (${mode}): renders the narrated example to a concise MP4 under the size limit, with a preview GIF`, {
    skip: !E2E && "set VE_VIDEO_E2E=1 to render real videos with real text to speech", timeout: 20 * 60_000,
  }, () => {
    const out = fs.mkdtempSync(path.join(TMP, `e2e-${mode}-`));
    const r = spawnSync(process.execPath, [RENDER_VIDEO, "--props", EXAMPLE_PROPS_FILE[mode],
      "--narration", EXAMPLE_NARRATION_FILE[mode], "--out", out], { encoding: "utf8", timeout: 18 * 60_000 });
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stderr, /render-video: tts: (elevenlabs|say) voice .+: \d+ clips, \d+ cached, \d+ characters sent/);
    assert.match(r.stderr, /render-video: screenshots: /);
    if (process.env.ELEVENLABS_API_KEY) assert.ok(!r.stderr.includes(process.env.ELEVENLABS_API_KEY), "the key is never in the log");
    const video = path.join(out, "video.mp4");
    const [line, engine, ...rest] = r.stdout.trim().split("\n");
    assert.equal(line, video, "stdout starts with the absolute MP4 path");
    assert.match(engine, /^tts: (elevenlabs|say|say \(fallback: .+\))$/, "then the engine line");
    assert.deepEqual(rest, []);
    assert.deepEqual(fs.readdirSync(out).sort(), ["preview.gif", "video.mp4"], "no temporary file stays");
    const size = fs.statSync(video).size;
    assert.ok(size > 0 && size <= 14_000_000, `${size} bytes`);
    assert.ok(fs.statSync(path.join(out, "preview.gif")).size <= 4_000_000);
    assert.ok(!["node_modules", "bundle", ".ready"].some((name) => fs.existsSync(path.join(SKILL, "video", name))), "the skill directory stays clean");
    if (FFPROBE) {
      const probe = JSON.parse(spawnSync("ffprobe", ["-v", "error", "-show_streams", "-show_format", "-of", "json", video], { encoding: "utf8" }).stdout);
      const streams = Object.fromEntries(probe.streams.map((s) => [s.codec_type, s]));
      assert.deepEqual(Object.keys(streams).sort(), ["audio", "video"]);
      assert.equal(streams.video.codec_name, "h264");
      assert.ok([[1920, 1080], [1280, 720]].some(([w, h]) => streams.video.width === w && streams.video.height === h));
      assert.equal(streams.audio.codec_name, "aac");
      const seconds = Number(probe.format.duration);
      assert.ok(seconds >= 45 && seconds <= 180, `${seconds} s: concise`);
    }
  });
}
