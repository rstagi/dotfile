#!/usr/bin/env node
// visual-explainer renderer: props JSON -> one self-contained index.html. Zero dependencies.
//
//   node render.mjs --props <file> --out <dir> [--video <path>]   validate, then write <dir>/index.html and print its path
//   node render.mjs --props <file> --check                       validate only
//
// --video embeds a video block at the top of the page. The path is relative to <dir>, ends in
// .mp4 and has no ".." segment (render-video.mjs writes <dir>/video.mp4).
//
// Exit: 0 valid (page written unless --check) · 1 invalid props, one "<field path>: <error>" line
// each on stderr · 2 usage or I/O error.
import fs from "node:fs";
import path from "node:path";
import { checkProps } from "./props.mjs";
import { renderPage } from "./scenes.mjs";
import { formatErrors } from "./validate.mjs";

const SKILL_DIR = path.join(import.meta.dirname, "..");
const TEMPLATE = path.join(SKILL_DIR, "template", "page.html");
const USAGE = "usage: render.mjs --props <file> (--out <dir> | --check) [--video <path>, with --out only]";
// Letters, digits, ".", "_", "-" and "/" only: no scheme, no absolute path, no "%"-encoded dots.
const VIDEO_PATH = /^[A-Za-z0-9_.-]+(?:\/[A-Za-z0-9_.-]+)*\.mp4$/;

main(process.argv.slice(2));

function main(argv) {
  const args = parseArgs(argv);
  if (!args) fail(USAGE);
  if (args.video !== undefined && !isVideoPath(args.video)) {
    fail(`render.mjs: --video must be a relative path that ends in .mp4 and has no ".." segment: ${args.video}`);
  }
  let checked;
  try {
    checked = checkProps(args.props);
  } catch (err) {
    fail(`render.mjs: ${err.message}`);
  }
  if (checked.errors.length) {
    process.stderr.write(`${formatErrors(checked.errors)}\n`);
    process.exit(1);
  }
  if (args.check) return;
  const html = renderPage(fs.readFileSync(TEMPLATE, "utf8"), checked.props, { video: args.video });
  const page = path.join(path.resolve(args.out), "index.html");
  try {
    fs.mkdirSync(path.dirname(page), { recursive: true });
    fs.writeFileSync(page, html);
  } catch (err) {
    fail(`render.mjs: cannot write ${page}: ${err.message}`);
  }
  process.stdout.write(`${page}\n`);
}

// Returns null when the arguments do not form exactly one valid call.
function parseArgs(argv) {
  const args = { check: false };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag === "--check") args.check = true;
    else if (["--props", "--out", "--video"].includes(flag) && argv[i + 1]) args[flag.slice(2)] = argv[++i];
    else return null;
  }
  if (args.video !== undefined && !args.out) return null;
  return args.props && (args.check !== Boolean(args.out)) ? args : null;
}

function isVideoPath(value) {
  return VIDEO_PATH.test(value) && !value.split("/").includes("..");
}

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(2);
}
