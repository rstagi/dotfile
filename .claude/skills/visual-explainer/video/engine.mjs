#!/usr/bin/env node
// Remotion steps for scripts/render-video.mjs. It runs only in the cache copy of this project, with
// that copy as the working directory: Remotion puts its browser in ./node_modules/.remotion.
//
//   node engine.mjs prepare                         download the browser, bundle src/ to bundle/
//   node engine.mjs render --input <file> --out <file.mp4>
//
// The input file is { props, voiceover, video }. The voiceover clips are in bundle/public/tts/.
// `video` holds the encoder settings: fps, scale, concurrency, and either crf with x264Preset or
// hardwareAcceleration with videoBitrate.
// No licenseKey: Remotion then sends no usage event. The free license covers this use (see SKILL.md).
import fs from "node:fs";
import path from "node:path";
import { bundle } from "@remotion/bundler";
import { ensureBrowser, renderMedia, selectComposition } from "@remotion/renderer";

const BUNDLE_DIR = path.resolve("bundle");
const LOG_LEVEL = "warn";
const AUDIO_BITRATE = "64k"; // enough for one voice, and it keeps a 10 minute video under the size limit

const [command, ...rest] = process.argv.slice(2);
try {
  if (command === "prepare") await prepare();
  else if (command === "render") await render(options(rest));
  else throw new Error("usage: engine.mjs prepare | render --input <file> --out <file.mp4>");
} catch (err) {
  process.stderr.write(`engine: ${err.stack ?? err.message}\n`);
  process.exit(1);
}

async function prepare() {
  await ensureBrowser({ logLevel: LOG_LEVEL });
  await bundle({ entryPoint: path.resolve("src", "index.jsx"), outDir: BUNDLE_DIR, enableCaching: false });
}

async function render({ input, out }) {
  const { props, voiceover, video } = JSON.parse(fs.readFileSync(input, "utf8"));
  const inputProps = { ...props, voiceover, fps: video.fps };
  const chromiumOptions = video.gl ? { gl: video.gl } : {};
  const composition = await selectComposition({ serveUrl: BUNDLE_DIR, id: props.mode, inputProps, chromiumOptions, logLevel: LOG_LEVEL });
  const encoder = video.hardwareAcceleration === "disable"
    ? { crf: video.crf, x264Preset: video.x264Preset }
    : { hardwareAcceleration: video.hardwareAcceleration, videoBitrate: video.videoBitrate };
  const started = Date.now();
  let reported = -1;
  const { slowestFrames } = await renderMedia({
    composition,
    serveUrl: BUNDLE_DIR,
    codec: "h264",
    ...encoder,
    scale: video.scale,
    concurrency: video.concurrency,
    chromiumOptions,
    audioBitrate: AUDIO_BITRATE,
    overwrite: true,
    outputLocation: out,
    inputProps,
    logLevel: LOG_LEVEL,
    onProgress: ({ progress }) => {
      const step = Math.floor(progress * 10) * 10;
      if (step > reported) {
        reported = step;
        process.stderr.write(`render ${step}%\n`);
      }
    },
  });
  const slowest = slowestFrames.slice(0, 3).map((f) => `${f.frame}:${Math.round(f.time)}ms`).join(" ");
  process.stderr.write(`render done: ${composition.durationInFrames} frames in ${((Date.now() - started) / 1000).toFixed(1)} s, slowest frames ${slowest}\n`);
}

function options(argv) {
  const opts = {};
  for (let i = 0; i < argv.length; i += 2) opts[argv[i].replace(/^--/, "")] = argv[i + 1];
  if (!opts.input || !opts.out) throw new Error("render needs --input and --out");
  return { input: opts.input, out: opts.out };
}
