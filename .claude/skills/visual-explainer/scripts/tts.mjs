// Text to speech for the video narration. Zero dependencies: ElevenLabs over fetch, or the macOS
// `say` and `afconvert` commands. Each clip is a mono 16-bit WAV file, cached by its key.
//
// The ElevenLabs key comes from ELEVENLABS_API_KEY. It never goes into a log, an error message,
// a file name or a file.
import { execFile, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { promisify } from "node:util";

const run = promisify(execFile);

const ELEVENLABS_URL = "https://api.elevenlabs.io/v1/text-to-speech";
const ELEVENLABS_VOICE = "XrExE9yKIg1WjnnlVkGX"; // Matilda: a premade voice for informative and educational speech
const ELEVENLABS_MODEL = "eleven_flash_v2_5"; // half the credits per character of eleven_multilingual_v2
const SAMPLE_RATE = 24000;
const RETRY_STATUS = new Set([429, 500, 502, 503, 504]);
const RETRY_DELAY_MS = 2000;
const REQUEST_TIMEOUT_MS = 90_000; // one section of 1500 characters takes about 15 s
const ERROR_STATUS = /^[a-z_]{1,40}$/; // an ElevenLabs error status, for example quota_exceeded
const ERROR_CODE = /^[A-Z_]{1,40}$/; // a Node network error code, for example ENOTFOUND
const SAY_MODEL = "say";
// Known good standard voices, best first. A Premium or Enhanced English voice comes before all of them.
const SAY_VOICES = ["Samantha", "Ava", "Allison", "Susan", "Tom", "Alex", "Daniel", "Serena", "Karen", "Moira", "Tessa"];

// speak(sections, option, opts) -> { clips, sentChars, cached, settings, fallback }. It speaks the
// narration with the engine that `option` selects (auto, elevenlabs, say). In `auto` mode, when
// ElevenLabs fails for any clip, it speaks the whole narration again with say: one video has one
// voice. `fallback` is then the short reason, for example "HTTP 401", else null. It calls
// `onFallback(reason)` before the say pass. An explicit engine never falls back, and the end of
// `signal` (the run deadline) stops the run. VE_TTS_VOICE names the ElevenLabs voice in `auto`
// mode, so the say pass uses the best installed say voice.
// `opts`: { dir, env, signal, sayVoices, onFallback } and the test seams { engines, fetch, retryDelayMs, requestTimeoutMs }.
export async function speak(sections, option, opts) {
  const engine = chooseEngine(option, opts.env);
  const settings = ttsSettings(engine, opts.env, opts);
  try {
    return { ...(await synthesize(sections, settings, opts)), settings, fallback: null };
  } catch (err) {
    if (option !== "auto" || engine !== "elevenlabs" || opts.signal?.aborted) throw err;
    const reason = err.reason ?? "error";
    opts.onFallback?.(reason);
    const say = ttsSettings("say", { ...opts.env, VE_TTS_VOICE: "" }, opts);
    return { ...(await synthesize(sections, say, opts)), settings: say, fallback: reason };
  }
}

// chooseEngine(option, env) -> "elevenlabs" | "say". `auto` uses ElevenLabs when the key is set.
export function chooseEngine(option, env) {
  if (option !== "auto") return option;
  return env.ELEVENLABS_API_KEY ? "elevenlabs" : "say";
}

// ttsSettings(engine, env, { sayVoices }) -> { engine, voice, model }. VE_TTS_VOICE and
// VE_TTS_MODEL override the defaults. `sayVoices` is the output of `say -v ?` (read when absent).
export function ttsSettings(engine, env, { sayVoices } = {}) {
  if (engine === "elevenlabs") {
    if (!env.ELEVENLABS_API_KEY) throw new Error("ELEVENLABS_API_KEY is not set");
    return { engine, voice: env.VE_TTS_VOICE || ELEVENLABS_VOICE, model: env.VE_TTS_MODEL || ELEVENLABS_MODEL };
  }
  const voice = env.VE_TTS_VOICE || pickSayVoice(sayVoices ?? listSayVoices());
  return { engine, voice, model: SAY_MODEL };
}

// pickSayVoice(listing) -> the best English voice name in `say -v ?` output, or null.
export function pickSayVoice(listing) {
  const english = listing.split("\n")
    .map((line) => /^(.+?)\s+([a-z]{2,3}_[A-Z0-9]{2,3})\s+#/.exec(line))
    .filter((m) => m && m[2].startsWith("en_"))
    .map((m) => m[1].trim());
  const rank = (name) => {
    if (/\(Premium\)/.test(name)) return 0;
    if (/\(Enhanced\)/.test(name)) return 1;
    const known = SAY_VOICES.indexOf(name);
    return known === -1 ? Infinity : 2 + known;
  };
  const best = english.map((name) => [rank(name), name]).filter(([r]) => r !== Infinity).sort((a, b) => a[0] - b[0])[0];
  return best ? best[1] : null;
}

// clipKey({ engine, voice, model, text }) -> the SHA-256 hex digest that names a cached clip.
export function clipKey({ engine, voice, model, text }) {
  return createHash("sha256").update(JSON.stringify([engine, voice, model, text])).digest("hex");
}

// synthesize(sections, settings, { dir, env, signal, engines, fetch, retryDelayMs }) -> { clips, sentChars, cached }.
// One clip for each { id, text } section: { id, file, seconds }. A cached clip costs nothing.
// `sentChars` counts the characters sent to the engine. In tests, `engines` replaces real engines
// by name, and `fetch` and `retryDelayMs` go to the ElevenLabs engine. A failure is an Error with
// a redacted message and a short `reason`.
export async function synthesize(sections, settings, { dir, env, signal, engines: fakes, fetch, retryDelayMs, requestTimeoutMs }) {
  const engines = { ...ENGINES, ...fakes };
  fs.mkdirSync(dir, { recursive: true });
  const clips = [];
  let sentChars = 0;
  let cached = 0;
  for (const { id, text } of sections) {
    const file = path.join(dir, `${clipKey({ ...settings, text })}.wav`);
    if (fs.existsSync(file)) {
      cached += 1;
    } else {
      const tmp = `${file}.tmp-${process.pid}`;
      try {
        await engines[settings.engine]({ text, voice: settings.voice, model: settings.model, out: tmp, env, signal, fetch, retryDelayMs, requestTimeoutMs });
        wavSeconds(tmp);
        fs.renameSync(tmp, file);
      } catch (err) {
        fs.rmSync(tmp, { force: true });
        throw Object.assign(new Error(redact(`text to speech failed for section ${id}: ${err.message}`, env)), { reason: err.reason });
      }
      sentChars += text.length;
    }
    clips.push({ id, file, seconds: wavSeconds(file) });
  }
  return { clips, sentChars, cached };
}

// wavSeconds(file) -> the duration of a PCM WAV file. Throws for a file that is not WAV.
export function wavSeconds(file) {
  const buf = fs.readFileSync(file);
  if (buf.toString("ascii", 0, 4) !== "RIFF" || buf.toString("ascii", 8, 12) !== "WAVE") throw new Error("the clip is not a WAV file");
  let byteRate = 0;
  for (let at = 12; at + 8 <= buf.length;) {
    const id = buf.toString("ascii", at, at + 4);
    const size = buf.readUInt32LE(at + 4);
    if (id === "fmt ") byteRate = buf.readUInt32LE(at + 16);
    if (id === "data" && byteRate) return Math.min(size, buf.length - at - 8) / byteRate;
    at += 8 + size + (size % 2);
  }
  throw new Error("the WAV clip has no audio data");
}

const ENGINES = { elevenlabs: elevenlabsClip, say: sayClip };

// One retry for a busy or failing server. Each request has its own timeout. Each error has a short
// `reason`: "HTTP <status>" with the ElevenLabs error status when the body has one (for example
// "HTTP 401 quota_exceeded"), "timeout" or "network error <code>".
async function elevenlabsClip({ text, voice, model, out, env, signal, fetch = globalThis.fetch, retryDelayMs = RETRY_DELAY_MS, requestTimeoutMs = REQUEST_TIMEOUT_MS }) {
  const url = `${ELEVENLABS_URL}/${encodeURIComponent(voice)}?output_format=pcm_${SAMPLE_RATE}`;
  const request = () => ({
    method: "POST",
    headers: { "xi-api-key": env.ELEVENLABS_API_KEY, "content-type": "application/json" },
    body: JSON.stringify({ text, model_id: model }),
    signal: AbortSignal.any([AbortSignal.timeout(requestTimeoutMs), ...(signal ? [signal] : [])]),
  });
  try {
    let res = await fetch(url, request());
    if (RETRY_STATUS.has(res.status)) {
      await new Promise((resolve) => setTimeout(resolve, retryDelayMs));
      res = await fetch(url, request());
    }
    if (!res.ok) {
      const body = (await res.text()).slice(0, 300);
      const status = errorStatus(body);
      throw Object.assign(new Error(`ElevenLabs returned HTTP ${res.status}: ${body}`), { reason: `HTTP ${res.status}${status ? ` ${status}` : ""}` });
    }
    const pcm = Buffer.from(await res.arrayBuffer());
    fs.writeFileSync(out, Buffer.concat([wavHeader(pcm.length), pcm]));
  } catch (err) {
    if (err.reason) throw err;
    const code = err.cause?.code;
    const reason = err.name === "TimeoutError" ? "timeout" : `network error${ERROR_CODE.test(code) ? ` ${code}` : ""}`;
    throw Object.assign(new Error(`ElevenLabs request failed: ${err.message}`), { reason });
  }
}

// The ElevenLabs error status in a JSON error body ({ detail: { status } }), when it is a plain token.
function errorStatus(body) {
  try {
    const status = JSON.parse(body)?.detail?.status;
    return ERROR_STATUS.test(status) ? status : null;
  } catch {
    return null;
  }
}

// say writes AIFF. afconvert turns it into the same WAV format as the ElevenLabs clips.
async function sayClip({ text, voice, out, signal }) {
  const txt = `${out}.txt`;
  const aiff = `${out}.aiff`;
  try {
    fs.writeFileSync(txt, text);
    await run("say", [...(voice ? ["-v", voice] : []), "-o", aiff, "-f", txt], { signal });
    await run("afconvert", ["-f", "WAVE", "-d", `LEI16@${SAMPLE_RATE}`, "-c", "1", aiff, out], { signal });
  } finally {
    fs.rmSync(txt, { force: true });
    fs.rmSync(aiff, { force: true });
  }
}

// The installed voices, or "" where `say` does not exist: then say uses the system voice.
function listSayVoices() {
  try {
    return execFileSync("say", ["-v", "?"], { encoding: "utf8" });
  } catch {
    return "";
  }
}

// A 44-byte header for mono 16-bit PCM at SAMPLE_RATE.
function wavHeader(dataBytes) {
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + dataBytes, 4);
  h.write("WAVE", 8);
  h.write("fmt ", 12);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(SAMPLE_RATE, 24);
  h.writeUInt32LE(SAMPLE_RATE * 2, 28);
  h.writeUInt16LE(2, 32);
  h.writeUInt16LE(16, 34);
  h.write("data", 36);
  h.writeUInt32LE(dataBytes, 40);
  return h;
}

function redact(message, env) {
  const key = env?.ELEVENLABS_API_KEY;
  return key ? message.split(key).join("***") : message;
}
