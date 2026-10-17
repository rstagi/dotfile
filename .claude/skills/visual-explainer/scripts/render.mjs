#!/usr/bin/env node
// visual-explainer renderer: props JSON -> one self-contained index.html. Zero dependencies.
//
//   node render.mjs --props <file> --out <dir>   validate, then write <dir>/index.html and print its path
//   node render.mjs --props <file> --check       validate only
//
// Exit: 0 valid (page written unless --check) · 1 invalid props, one "<field path>: <error>" line
// each on stderr · 2 usage or I/O error.
import fs from "node:fs";
import path from "node:path";
import { renderPage } from "./scenes.mjs";
import { formatErrors, validate } from "./validate.mjs";

const SKILL_DIR = path.join(import.meta.dirname, "..");
const TEMPLATE = path.join(SKILL_DIR, "template", "page.html");
const MODES = ["plan", "ship"];
const USAGE = "usage: render.mjs --props <file> (--out <dir> | --check)";

main(process.argv.slice(2));

function main(argv) {
  const args = parseArgs(argv);
  if (!args) fail(USAGE);
  const errors = [];
  const props = readProps(args.props, errors);
  if (!errors.length) errors.push(...validate(loadSchema(props.mode), props));
  if (errors.length) {
    process.stderr.write(`${formatErrors(errors)}\n`);
    process.exit(1);
  }
  if (args.check) return;
  const html = renderPage(fs.readFileSync(TEMPLATE, "utf8"), props);
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
    else if ((flag === "--props" || flag === "--out") && argv[i + 1]) args[flag.slice(2)] = argv[++i];
    else return null;
  }
  return args.props && (args.check !== Boolean(args.out)) ? args : null;
}

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(2);
}

// The mode selects the schema, so check it first: one clear error beats a cascade.
function readProps(file, errors) {
  let text;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch (err) {
    fail(`render.mjs: cannot read ${file}: ${err.message}`);
  }
  let props;
  try {
    props = JSON.parse(text);
  } catch (err) {
    errors.push({ path: "$", message: `invalid JSON (${err.message})` });
    return null;
  }
  if (!MODES.includes(props?.mode)) errors.push({ path: "$.mode", message: `must be one of ${MODES.join(", ")}` });
  return props;
}

function loadSchema(mode) {
  return JSON.parse(fs.readFileSync(path.join(SKILL_DIR, "schema", `${mode}.schema.json`), "utf8"));
}
