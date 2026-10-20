#!/usr/bin/env node
// visual-explainer video: props JSON + narration JSON -> <out>/video.mp4, a short narrated video
// that Remotion renders from the video/ project, and <out>/preview.gif. The video has one scene for
// each narration scene, and each scene shows its section of the report page with snapcn components.
// The diagrams and the media come from dark screenshots of a report page that this script renders
// in a temporary directory (render.mjs, then shoot.mjs). Without screenshots, the scenes draw a
// simpler native view: a failed screenshot step only warns.
//
//   node render-video.mjs --props <file> --narration <file> --out <dir> [--timeout-min <n>] [--max-mb <n>] [--tts <engine>]
//   node render-video.mjs --props <file> --narration <file> --check     validate only
//
// The skill directory stays read-only: it can be a symlinked checkout or a Loop integration worktree.
// The script copies video/ to ${XDG_CACHE_HOME:-$HOME/.cache}/visual-explainer/video-<key>, runs
// `npm ci` and the browser download there once for each key, and renders there. Text to speech
// clips go to .../visual-explainer/tts/<sha256>.wav, so a re-render sends no text again.
// --timeout-min bounds the whole run. The first x264 quality comes from the video length and
// --max-mb (firstCrf). A video above --max-mb renders again at a lower quality, then at 1280x720. --tts: auto (ElevenLabs when ELEVENLABS_API_KEY is set, else say), elevenlabs, say.
// In auto mode, a failed ElevenLabs call makes the whole narration speak with say (tts.mjs speak).
// VE_VIDEO_FPS, VE_VIDEO_CONCURRENCY, VE_VIDEO_HWACCEL, VE_VIDEO_X264_PRESET and VE_VIDEO_GL override
// the encoder defaults (videoSettings).
//
// After the MP4, ffmpeg writes preview.gif: about 1.5 s from the middle of each scene, at most 4 MB.
// A failed GIF only warns.
//
// Stdout after a render: the absolute MP4 path, then "tts: elevenlabs", "tts: say" or
// "tts: say (fallback: <reason>)".
// Exit: 0 valid (MP4 written unless --check) · 1 invalid props or narration, one "<field path>: <error>"
// line each on stderr · 2 usage or I/O error · 3 the text to speech or the render failed or timed out.
import { spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { storyboard } from "../video/src/storyboard.mjs";
import { checkNarration } from "./narration.mjs";
import { checkProps } from "./props.mjs";
import { speak } from "./tts.mjs";
import { formatErrors } from "./validate.mjs";

const VIDEO_DIR = path.join(import.meta.dirname, "..", "video");
const RENDER_PAGE = path.join(import.meta.dirname, "render.mjs");
const SHOOT = path.join(import.meta.dirname, "shoot.mjs");
// The screenshots of the report page that the scenes show (data-shot ids).
const SHOT_IDS = ["architecture", "sequence", "media"];
const USAGE = "usage: render-video.mjs --props <file> --narration <file> --out <dir> [--check] [--timeout-min <n>] [--max-mb <n>] [--tts auto|elevenlabs|say]";
const DEFAULTS = { timeoutMin: 15, maxMb: 14, tts: "auto" };
const NUMBER_FLAGS = { "--timeout-min": "timeoutMin", "--max-mb": "maxMb" };
const PATH_FLAGS = ["--props", "--narration", "--out"];
const TTS_ENGINES = ["auto", "elevenlabs", "say"];
const SECRET_ENV = ["ELEVENLABS_API_KEY", "ANTHROPIC_API_KEY"];
const BYTES_PER_MB = 1_000_000;
const READY = ".ready";
const CACHE = /^video-[0-9a-f]{12}$/;
const TEMP_CACHE = /^video-[0-9a-f]{12}\.tmp-/;
const UNUSED_MAX_AGE_MS = 7 * 24 * 3600 * 1000;
const TEMP_MAX_AGE_MS = 24 * 3600 * 1000;
const TAIL_LINES = 20;
const KILL_GRACE_MS = 3000;
// Encoder defaults: the fastest measured combination that keeps the text sharp (see SKILL.md, The video).
// Frame capture in the browser takes most of the time, so fewer frames and GPU compositing help most.
// VE_VIDEO_FPS, VE_VIDEO_CONCURRENCY, VE_VIDEO_HWACCEL, VE_VIDEO_X264_PRESET and VE_VIDEO_GL override them.
const VIDEO_DEFAULTS = { fps: 30, hardwareAcceleration: "disable", x264Preset: "veryfast", gl: "angle" };
const SPARE_CORES = 2;
const HWACCEL = ["disable", "if-possible", "required"];
const GL = ["default", "angle", "egl", "swiftshader", "swangle", "vulkan", "angle-egl"];
const X264_PRESETS = ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"];
const AUDIO_KBPS = 64;
const SIZE_MARGIN = 0.9; // a hardware encoder can overshoot its target bitrate, and the x264 model is an estimate
// x264 size model, measured on the old graph video (13 stops, 1920x1080, veryfast, 24 fps): at
// CRF 23 the video stream had about 0.5 MB plus 54 KB for each second, and 7 CRF steps halved it.
// The model uses 8 steps, so it predicts a larger file. The scene video moves less: at 30 fps and
// CRF 23 the plan example (107 s) has 5.7 MB and the ship example (105 s) 5.0 MB, audio included,
// so the model stays conservative and a concise video keeps CRF 23.
const X264 = { bestCrf: 23, lowCrf: 30, halvingCrf: 8, fixedBytes: 500_000, bytesPerSecond: 54_000 };
const MIN_VIDEO_KBPS = 150;
const MAX_VIDEO_KBPS = 2500;
// preview.gif: PREVIEW_SECONDS from the middle of each scene. Each attempt has fewer frames or
// pixels than the one before it, until the file fits PREVIEW_MAX_BYTES.
const PREVIEW_SECONDS = 1.5;
const PREVIEW_MAX_BYTES = 4 * BYTES_PER_MB;
const PREVIEW_ATTEMPTS = [{ fps: 10, width: 720 }, { fps: 8, width: 640 }, { fps: 6, width: 560 }, { fps: 5, width: 480 }];
const PREVIEW_TIMEOUT_MS = 120_000;

class IoError extends Error {}
class StepError extends Error {}

// The last child process group and the temporary paths to remove when the run stops early.
const live = { group: null, temps: new Set() };

if (isMain()) await main(process.argv.slice(2));

async function main(argv) {
  const args = parseArgs(argv);
  if (!args) fail(2, USAGE);
  const { props, scenes } = checkInputs(args);
  let settings;
  try {
    settings = videoSettings(process.env);
  } catch (err) {
    fail(2, `render-video.mjs: ${err.message}`);
  }
  if (args.check) return;

  for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => stop(3, `render-video.mjs: stopped by ${signal}`));
  const deadline = { at: Date.now() + args.timeoutMin * 60_000, label: `${args.timeoutMin} min` };
  try {
    const out = makeDir(path.resolve(args.out));
    const dir = cacheDir(process.env, cacheKey(VIDEO_DIR));
    await prepareCache(dir, deadline);
    markUsed(dir);
    const shots = await captureShots(dir, args.props, deadline);
    const { voiceover, engine } = await narrate(dir, scenes, args.tts, deadline);
    const input = { mode: props.mode, props, scenes, shots };
    const board = storyboard(input, voiceover, { fps: settings.fps });
    const attempts = renderAttempts(settings, board.durationInFrames / settings.fps, args.maxMb);
    const video = await renderVideo(dir, { props: input, voiceover }, attempts, out, args.maxMb, deadline);
    await writePreview(video, board, out, deadline);
    process.stdout.write(`${video}\ntts: ${engine}\n`);
  } catch (err) {
    stop(err instanceof IoError ? 2 : 3, `render-video.mjs: ${err.message}`);
  }
}

// parseArgs(argv) -> { props, narration, out?, check, timeoutMin, maxMb, tts }, or null when the
// arguments do not form exactly one valid call. --out is required unless --check is set.
export function parseArgs(argv) {
  const args = { check: false, ...DEFAULTS };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    if (flag === "--check") args.check = true;
    else if (PATH_FLAGS.includes(flag) && value) args[flag.slice(2)] = argv[++i];
    else if (NUMBER_FLAGS[flag] && value && Number(value) > 0) args[NUMBER_FLAGS[flag]] = Number(argv[++i]);
    else if (flag === "--tts" && TTS_ENGINES.includes(value)) args.tts = argv[++i];
    else return null;
  }
  return args.props && args.narration && (args.check || args.out) ? args : null;
}

// cacheKey(videoDir) -> the first 12 hex characters of a SHA-256 over the relative path and the
// content of each project file: the lock file and the sources. node_modules and dot files do not count.
export function cacheKey(videoDir) {
  const hash = createHash("sha256");
  for (const rel of projectFiles(videoDir)) {
    hash.update(`${rel}\0`).update(fs.readFileSync(path.join(videoDir, rel))).update("\0");
  }
  return hash.digest("hex").slice(0, 12);
}

// cacheDir(env, key) -> ${XDG_CACHE_HOME:-$HOME/.cache}/visual-explainer/video-<key>.
export function cacheDir(env, key) {
  return path.join(cacheRoot(env), `video-${key}`);
}

function cacheRoot(env) {
  const base = env.XDG_CACHE_HOME || path.join(env.HOME || os.homedir(), ".cache");
  return path.join(path.resolve(base), "visual-explainer");
}

// Check the props, then the narration against them. Exit 1 or 2 on a failure.
function checkInputs(args) {
  try {
    const { props, errors } = checkProps(args.props);
    if (errors.length) fail(1, formatErrors(errors));
    const checked = checkNarration(args.narration, props);
    if (checked.errors.length) fail(1, formatErrors(checked.errors));
    return { props, scenes: checked.scenes };
  } catch (err) {
    fail(2, `render-video.mjs: ${err.message}`);
  }
}

// pruneCaches(current, now) removes the sibling caches of `current` with no use for 7 days, and
// temporary copies older than 1 day (a run that was killed). A run marks its cache as used.
export function pruneCaches(current, now = Date.now()) {
  const parent = path.dirname(current);
  for (const name of fs.readdirSync(parent)) {
    const dir = path.join(parent, name);
    if (dir === current) continue;
    const temp = TEMP_CACHE.test(name);
    if (!temp && !CACHE.test(name)) continue;
    const stamp = temp ? dir : path.join(dir, READY);
    const age = now - (fs.statSync(stamp, { throwIfNoEntry: false })?.mtimeMs ?? now);
    if (age > (temp ? TEMP_MAX_AGE_MS : UNUSED_MAX_AGE_MS)) fs.rmSync(dir, { recursive: true, force: true });
  }
}

// videoSettings(env) -> { fps, concurrency, hardwareAcceleration, x264Preset, gl }: the defaults, with
// the VE_VIDEO_* overrides. The default concurrency leaves 2 cores free. VE_VIDEO_GL=default lets
// Chrome choose. Throws an IoError for a value that is not valid.
export function videoSettings(env) {
  const settings = { ...VIDEO_DEFAULTS, concurrency: Math.max(1, os.availableParallelism() - SPARE_CORES) };
  if (env.VE_VIDEO_FPS) settings.fps = integerIn(env.VE_VIDEO_FPS, 1, 60, "VE_VIDEO_FPS");
  if (env.VE_VIDEO_CONCURRENCY) settings.concurrency = integerIn(env.VE_VIDEO_CONCURRENCY, 1, 64, "VE_VIDEO_CONCURRENCY");
  if (env.VE_VIDEO_HWACCEL) settings.hardwareAcceleration = oneOf(env.VE_VIDEO_HWACCEL, HWACCEL, "VE_VIDEO_HWACCEL");
  if (env.VE_VIDEO_X264_PRESET) settings.x264Preset = oneOf(env.VE_VIDEO_X264_PRESET, X264_PRESETS, "VE_VIDEO_X264_PRESET");
  if (env.VE_VIDEO_GL) settings.gl = oneOf(env.VE_VIDEO_GL, GL, "VE_VIDEO_GL") === "default" ? null : env.VE_VIDEO_GL;
  return settings;
}

// renderAttempts(settings, seconds, maxMb) -> the encoder settings of each attempt, best first.
// A file above maxMb moves to the next attempt. x264 starts at the CRF that firstCrf predicts for
// this duration, then lowers the quality to CRF 30, then the size. A hardware encoder gets a
// bitrate that fits maxMb for this duration, then a lower one at 1280x720.
export function renderAttempts(settings, seconds, maxMb) {
  const base = { fps: settings.fps, concurrency: settings.concurrency, hardwareAcceleration: settings.hardwareAcceleration, gl: settings.gl };
  if (settings.hardwareAcceleration === "disable") {
    const x264 = { ...base, x264Preset: settings.x264Preset };
    const first = firstCrf(seconds, maxMb);
    return [
      { ...x264, label: "1920x1080", scale: 1, crf: first },
      ...(first < X264.lowCrf ? [{ ...x264, label: "1920x1080 at a lower quality", scale: 1, crf: X264.lowCrf }] : []),
      { ...x264, label: "1280x720", scale: 2 / 3, crf: X264.lowCrf },
    ];
  }
  const kbps = Math.floor((maxMb * BYTES_PER_MB * 8 * SIZE_MARGIN) / seconds / 1000) - AUDIO_KBPS;
  const target = Math.min(MAX_VIDEO_KBPS, Math.max(MIN_VIDEO_KBPS, kbps));
  return [
    { ...base, label: `1920x1080 at ${target} kbps`, scale: 1, videoBitrate: `${target}k` },
    { ...base, label: `1280x720 at ${Math.floor(target * 0.7)} kbps`, scale: 2 / 3, videoBitrate: `${Math.floor(target * 0.7)}k` },
  ];
}

// firstCrf(seconds, maxMb) -> the best x264 CRF, from 23 to 30, whose predicted file fits
// SIZE_MARGIN of maxMb. The prediction is the X264 size model plus the audio.
function firstCrf(seconds, maxMb) {
  const budget = maxMb * BYTES_PER_MB * SIZE_MARGIN - (seconds * AUDIO_KBPS * 1000) / 8;
  const atBest = X264.fixedBytes + X264.bytesPerSecond * seconds;
  if (budget >= atBest) return X264.bestCrf;
  if (budget <= 0) return X264.lowCrf;
  return Math.min(X264.lowCrf, X264.bestCrf + Math.ceil(X264.halvingCrf * Math.log2(atBest / budget)));
}

function integerIn(value, min, max, name) {
  const n = Number(value);
  if (!Number.isInteger(n) || n < min || n > max) throw new IoError(`${name} must be an integer from ${min} to ${max}, got ${value}`);
  return n;
}

function oneOf(value, allowed, name) {
  if (!allowed.includes(value)) throw new IoError(`${name} must be one of ${allowed.join(", ")}, got ${value}`);
  return value;
}

// The project files as sorted POSIX paths relative to `dir`.
function projectFiles(dir, prefix = "") {
  return fs.readdirSync(path.join(dir, prefix), { withFileTypes: true })
    .filter((entry) => !entry.name.startsWith(".") && entry.name !== "node_modules")
    .flatMap((entry) => {
      const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
      return entry.isDirectory() ? projectFiles(dir, rel) : [rel];
    })
    .sort();
}

// Build the cache in a temporary sibling, then rename it: a cache directory with READY is complete.
async function prepareCache(dir, deadline) {
  if (fs.existsSync(path.join(dir, READY))) return;
  makeDir(path.dirname(dir));
  const tmp = `${dir}.tmp-${process.pid}-${randomBytes(4).toString("hex")}`;
  live.temps.add(tmp);
  log("preparing the Remotion cache (npm ci, browser download, bundle): one time for each cache key");
  fs.cpSync(VIDEO_DIR, tmp, { recursive: true, filter: (src) => path.basename(src) !== "node_modules" });
  await runStep("npm ci", "npm", ["ci", "--no-audit", "--no-fund", "--loglevel=error"], { cwd: tmp, deadline });
  await runStep("prepare", process.execPath, ["engine.mjs", "prepare"], { cwd: tmp, deadline });
  fs.writeFileSync(path.join(tmp, READY), `${new Date().toISOString()}\n`);
  if (fs.existsSync(dir) && !fs.existsSync(path.join(dir, READY))) fs.rmSync(dir, { recursive: true, force: true });
  try {
    fs.renameSync(tmp, dir);
  } catch (err) {
    if (!fs.existsSync(path.join(dir, READY))) throw err; // else a parallel run finished first: use its cache
  }
  removeTemp(tmp);
}

// Touch READY so that pruneCaches keeps this cache, then prune the others. A failed prune never fails the render.
function markUsed(dir) {
  try {
    const now = new Date();
    fs.utimesSync(path.join(dir, READY), now, now);
    pruneCaches(dir);
  } catch (err) {
    log(`cache prune skipped: ${err.message}`);
  }
}

// Render the report page in a temporary directory, take dark screenshots of its sections with the
// browser of the cache, and put the ones that the scenes show where the bundle serves them.
// Returns { <shot id>: { src, width, height } }: {} when a step fails, with one stderr line. Only
// the end of the run deadline stops the run.
async function captureShots(dir, propsFile, deadline) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "visual-explainer-shots-"));
  live.temps.add(tmp);
  try {
    await runStep("report page", process.execPath, [RENDER_PAGE, "--props", path.resolve(propsFile), "--out", tmp], { cwd: tmp, deadline });
    const chrome = findChrome(dir);
    const shotsDir = path.join(tmp, "shots");
    await runStep("screenshots", process.execPath, [SHOOT, "--page", path.join(tmp, "index.html"), "--out", shotsDir, "--theme", "dark",
      ...(chrome ? ["--chrome", chrome] : [])], { cwd: tmp, deadline });
    const served = makeDir(path.join(dir, "bundle", "public", "shots"));
    const shots = {};
    for (const id of SHOT_IDS) {
      const file = path.join(shotsDir, `${id}.png`);
      if (!fs.existsSync(file)) continue;
      const bytes = fs.readFileSync(file);
      const size = pngSize(bytes);
      if (!size) continue;
      const name = `${createHash("sha256").update(bytes).digest("hex").slice(0, 16)}.png`;
      if (!fs.existsSync(path.join(served, name))) fs.writeFileSync(path.join(served, name), bytes);
      shots[id] = { src: `shots/${name}`, ...size };
    }
    log(`screenshots: ${Object.keys(shots).join(", ") || "none of " + SHOT_IDS.join(", ")}`);
    return shots;
  } catch (err) {
    if (err.message.startsWith("timed out")) throw err;
    log(`screenshots skipped, the scenes draw their native view: ${err.message.split("\n")[0]}`);
    return {};
  } finally {
    removeTemp(tmp);
  }
}

// findChrome(dir) -> the chrome-headless-shell binary that Remotion downloaded into the cache, or
// null (shoot.mjs then finds a browser itself).
export function findChrome(dir) {
  const root = path.join(dir, "node_modules", ".remotion", "chrome-headless-shell");
  const walk = (at, depth) => {
    for (const entry of fs.readdirSync(at, { withFileTypes: true })) {
      const full = path.join(at, entry.name);
      if (entry.isFile() && entry.name === "chrome-headless-shell") return full;
      if (entry.isDirectory() && depth > 0) {
        const found = walk(full, depth - 1);
        if (found) return found;
      }
    }
    return null;
  };
  try {
    return walk(root, 3);
  } catch {
    return null;
  }
}

// pngSize(bytes) -> { width, height } from the IHDR chunk, or null for a file that is not a PNG.
export function pngSize(bytes) {
  if (bytes.length < 24 || bytes.toString("latin1", 1, 4) !== "PNG") return null;
  return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
}

// Speak the text of each scene, then put the clips where the bundle serves them (staticFile).
// Returns { voiceover: { <scene id>: { src, seconds } }, engine }. `engine` is the engine line
// of the result: "elevenlabs", "say" or "say (fallback: <reason>)". A failure is a StepError (exit 3).
async function narrate(dir, scenes, option, deadline) {
  const left = deadline.at - Date.now();
  if (left <= 0) throw new StepError(`timed out after ${deadline.label} before the text to speech`);
  let result;
  try {
    result = await speak(scenes.map(({ id, text }) => ({ id, text })), option, {
      dir: path.join(cacheRoot(process.env), "tts"), env: process.env, signal: AbortSignal.timeout(left),
      onFallback: (reason) => log(`ElevenLabs failed (${reason}): the video uses the macOS voice`),
    });
  } catch (err) {
    throw new StepError(err.message.startsWith("text to speech") ? err.message : `text to speech: ${err.message}`);
  }
  const { settings } = result;
  log(`tts: ${settings.engine} voice ${settings.voice} model ${settings.model}: ${result.clips.length} clips, ${result.cached} cached, ${result.sentChars} characters sent`);
  const served = makeDir(path.join(dir, "bundle", "public", "tts"));
  const voiceover = Object.fromEntries(result.clips.map((clip) => {
    const name = path.basename(clip.file);
    if (!fs.existsSync(path.join(served, name))) fs.copyFileSync(clip.file, path.join(served, name));
    return [clip.id, { src: `tts/${name}`, seconds: clip.seconds }];
  }));
  return { voiceover, engine: result.fallback ? `${settings.engine} (fallback: ${result.fallback})` : settings.engine };
}

// Render each attempt until one fits maxMb, then move it into place. Returns the absolute path of
// <out>/video.mp4.
async function renderVideo(dir, inputs, attempts, out, maxMb, deadline) {
  const target = path.join(out, "video.mp4");
  const tmp = path.join(out, `.video-${process.pid}.tmp.mp4`);
  const input = path.join(out, `.video-${process.pid}.input.json`);
  live.temps.add(tmp);
  live.temps.add(input);
  let size = 0;
  for (const { label, ...video } of attempts) {
    log(`rendering ${label}`);
    fs.writeFileSync(input, JSON.stringify({ ...inputs, video }));
    await runStep(`render ${label}`, process.execPath, ["engine.mjs", "render", "--input", input, "--out", tmp],
      { cwd: dir, deadline, echo: true });
    size = fs.statSync(tmp).size;
    if (size === 0) throw new StepError(`render ${label} wrote an empty file`);
    if (size <= maxMb * BYTES_PER_MB) {
      fs.renameSync(tmp, target);
      removeTemp(tmp);
      removeTemp(input);
      return target;
    }
    log(`the video has ${mb(size)} MB at ${label}, above --max-mb ${maxMb}`);
  }
  removeTemp(tmp);
  removeTemp(input);
  throw new StepError(`the video has ${mb(size)} MB at ${attempts.at(-1).label}, above --max-mb ${maxMb}`);
}

// Write <out>/preview.gif from the middle of each scene. A failure only warns: the MP4 is the result.
async function writePreview(video, board, out, deadline) {
  const target = path.join(out, "preview.gif");
  const tmp = path.join(out, `.preview-${process.pid}.tmp.gif`);
  live.temps.add(tmp);
  const gifDeadline = { at: Math.min(deadline.at, Date.now() + PREVIEW_TIMEOUT_MS), label: "the preview time limit" };
  try {
    for (const attempt of PREVIEW_ATTEMPTS) {
      await runStep("preview.gif", "ffmpeg", previewArgs(video, board, attempt, tmp), { cwd: out, deadline: gifDeadline });
      const size = fs.statSync(tmp).size;
      if (size > 0 && size <= PREVIEW_MAX_BYTES) {
        fs.renameSync(tmp, target);
        log(`preview: ${target} (${mb(size)} MB, ${attempt.width} px, ${attempt.fps} fps)`);
        return;
      }
    }
    log(`preview.gif skipped: above ${mb(PREVIEW_MAX_BYTES)} MB at ${PREVIEW_ATTEMPTS.at(-1).width} px`);
  } catch (err) {
    log(`preview.gif skipped: ${err.message.split("\n")[0]}`);
  } finally {
    removeTemp(tmp);
  }
}

// previewArgs(video, board, { fps, width }, out) -> the ffmpeg arguments: one input of
// PREVIEW_SECONDS for each scene, concatenated, with a palette made for these frames.
export function previewArgs(video, board, { fps, width }, out) {
  const starts = board.scenes.map((s) => Math.max(0, (s.from + s.durationInFrames / 2) / board.fps - PREVIEW_SECONDS / 2));
  const inputs = starts.flatMap((t) => ["-ss", t.toFixed(2), "-t", String(PREVIEW_SECONDS), "-i", video]);
  const concat = `${starts.map((_, i) => `[${i}:v]`).join("")}concat=n=${starts.length}:v=1:a=0`;
  const filter = `${concat},fps=${fps},scale=${width}:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4`;
  return ["-y", "-v", "error", ...inputs, "-filter_complex", filter, "-loop", "0", out];
}

// Run one child in its own process group, bounded by the deadline. On failure the error carries the
// last output lines. `echo` passes the child's stderr through (render progress).
function runStep(label, command, args, { cwd, deadline, echo = false }) {
  return new Promise((resolve, reject) => {
    const left = deadline.at - Date.now();
    if (left <= 0) return reject(new StepError(`timed out after ${deadline.label} before ${label}`));
    const tail = [];
    const env = { ...process.env, npm_config_update_notifier: "false" };
    for (const name of SECRET_ENV) delete env[name]; // npm, Remotion, Chrome and ffmpeg never need a key
    const child = spawn(command, args, { cwd, detached: true, stdio: ["ignore", "pipe", "pipe"], env });
    live.group = child.pid;
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      signalGroup(child.pid, "SIGTERM");
      setTimeout(() => signalGroup(child.pid, "SIGKILL"), KILL_GRACE_MS).unref();
    }, left);
    const collect = (chunk, passThrough) => {
      for (const line of chunk.toString().split("\n").filter(Boolean)) {
        tail.push(line);
        if (tail.length > TAIL_LINES) tail.shift();
        if (passThrough) log(line);
      }
    };
    child.stdout.on("data", (chunk) => collect(chunk, false));
    child.stderr.on("data", (chunk) => collect(chunk, echo));
    child.on("error", (err) => {
      clearTimeout(timer);
      reject(new StepError(`${label} could not start: ${err.message}`));
    });
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      if (timedOut) return reject(new StepError(`timed out after ${deadline.label} during ${label}`));
      if (code === 0) return resolve();
      reject(new StepError(`${label} failed (${signal ?? `exit ${code}`})${tail.length ? `:\n${tail.join("\n")}` : ""}`));
    });
  });
}

// Signal the whole process group of a step: node, npm and the Chrome processes of a render.
function signalGroup(pid, signal) {
  try {
    process.kill(-pid, signal);
  } catch {
    // the group is gone
  }
}

// Kill what is left of the last step, remove the temporary paths, report, exit.
function stop(code, message) {
  if (live.group) signalGroup(live.group, "SIGKILL");
  for (const temp of live.temps) fs.rmSync(temp, { recursive: true, force: true });
  fail(code, message);
}

function removeTemp(temp) {
  fs.rmSync(temp, { recursive: true, force: true });
  live.temps.delete(temp);
}

function makeDir(dir) {
  try {
    fs.mkdirSync(dir, { recursive: true });
    return dir;
  } catch (err) {
    throw new IoError(`cannot create ${dir}: ${err.message}`);
  }
}

function mb(bytes) {
  return (bytes / BYTES_PER_MB).toFixed(1);
}

function log(message) {
  process.stderr.write(`render-video: ${message}\n`);
}

function fail(code, message) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

// True when node runs this file, false when a test imports it. Node resolves the symlinked skill
// directory for import.meta.url, so compare real paths.
function isMain() {
  try {
    return fs.realpathSync(process.argv[1]) === fileURLToPath(import.meta.url);
  } catch {
    return false;
  }
}
