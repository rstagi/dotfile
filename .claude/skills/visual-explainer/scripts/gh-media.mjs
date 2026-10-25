#!/usr/bin/env node
// visual-explainer GitHub media: upload the screenshots of one run to the orphan branch
// `explainer-media` of a GitHub repository, then print a Markdown block for a PR review or
// comment. It uses the gh CLI and the git data API, so it needs no local clone.
//
//   node gh-media.mjs --repo <owner/repo> --key <key> --dir <out> [--url <page url>] [--branch <name>] [--dry-run]
//
// <key> names the folder on the branch, for example `pr-27` or `m4-ship`: letters, digits, ".", "_"
// and "-". <out> is the output directory of the run: props.json and shots/*.png.
// --url links the header screenshot to the published page. --dry-run uploads nothing and prints the
// block with the URLs that an upload would give.
//
// Exit: 0 block printed on stdout · 2 usage or I/O error · 3 gh or GitHub API failure.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const DEFAULT_BRANCH = "explainer-media";
const KEY = /^[A-Za-z0-9._-]{1,80}$/;
const REPO = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const MAX_FILE_BYTES = 50_000_000; // GitHub warns above 50 MB and rejects blobs above 100 MB
// The sections that the block shows, in order. The first ones are open, the rest are in a <details>.
const OPEN_SHOTS = { plan: ["header", "tldr"], ship: ["header", "tldr"] };
const MORE_SHOTS = {
  plan: ["done-when", "phases", "architecture", "decisions", "rabbit-holes", "media"],
  ship: ["done-when", "changes", "findings", "hand-test", "rollback", "media"],
};
const USAGE = "usage: gh-media.mjs --repo <owner/repo> --key <key> --dir <out> [--url <page url>] [--branch <name>] [--dry-run]";

main(process.argv.slice(2));

function main(argv) {
  const args = parseArgs(argv);
  if (!args) fail(USAGE, 2);
  const props = readJson(path.join(args.dir, "props.json"));
  const mode = props.mode === "plan" ? "plan" : "ship";
  const files = collectFiles(args.dir, mode);
  if (!files.length) fail(`gh-media: no shots in ${args.dir}`, 2);
  if (!args.dryRun) upload(args, files);
  process.stdout.write(`${markdown(args, props, mode, files)}\n`);
}

// The files to upload: [{name, abs}] with `name` relative to the key folder.
function collectFiles(dir, mode) {
  const shots = path.join(dir, "shots");
  const files = [];
  for (const id of [...OPEN_SHOTS[mode], ...MORE_SHOTS[mode]]) {
    const abs = path.join(shots, `${id}.png`);
    if (fs.existsSync(abs)) files.push({ name: `${id}.png`, abs, shot: id });
  }
  for (const file of files) {
    if (fs.statSync(file.abs).size > MAX_FILE_BYTES) fail(`gh-media: ${file.abs} is larger than ${MAX_FILE_BYTES} bytes`, 2);
  }
  return files;
}

// One commit on the media branch with every file under <key>/. The commit keeps the folders of
// other keys: its tree is based on the tree of the branch head.
function upload(args, files) {
  const repo = `repos/${args.repo}`;
  const head = api(["GET", `${repo}/git/ref/heads/${args.branch}`], null, { allow404: true });
  const parent = head?.object?.sha ?? null;
  const baseTree = parent ? api(["GET", `${repo}/git/commits/${parent}`]).tree.sha : null;
  const tree = files.map((file) => ({
    path: `${args.key}/${file.name}`,
    mode: "100644",
    type: "blob",
    sha: api(["POST", `${repo}/git/blobs`], { content: fs.readFileSync(file.abs).toString("base64"), encoding: "base64" }).sha,
  }));
  const treeSha = api(["POST", `${repo}/git/trees`], baseTree ? { base_tree: baseTree, tree } : { tree }).sha;
  const commit = api(["POST", `${repo}/git/commits`], {
    message: `explainer media: ${args.key}`,
    tree: treeSha,
    parents: parent ? [parent] : [],
  }).sha;
  if (parent) api(["PATCH", `${repo}/git/refs/heads/${args.branch}`], { sha: commit, force: false });
  else api(["POST", `${repo}/git/refs`], { ref: `refs/heads/${args.branch}`, sha: commit });
}

function markdown(args, props, mode, files) {
  const blob = (name) => `https://github.com/${args.repo}/blob/${args.branch}/${args.key}/${name}`;
  const image = (name) => `${blob(name)}?raw=true`;
  const have = new Set(files.map((f) => f.name));
  const title = String(props.meta?.title ?? "Explainer").replace(/[\[\]]/g, "");
  const lines = [`<!-- visual-explainer:${args.key} -->`, `### Visual ${mode === "plan" ? "plan" : "review"}: ${title}`, ""];
  if (args.url) lines.push(`Interactive page: ${args.url}`, "");
  for (const id of OPEN_SHOTS[mode]) {
    if (!have.has(`${id}.png`)) continue;
    const img = `![${id}](${image(`${id}.png`)})`;
    lines.push(id === "header" && args.url ? `[${img}](${args.url})` : img, "");
  }
  const more = MORE_SHOTS[mode].filter((id) => have.has(`${id}.png`));
  if (more.length) {
    lines.push("<details><summary>All sections</summary>", "");
    for (const id of more) lines.push(`![${id}](${image(`${id}.png`)})`, "");
    lines.push("</details>");
  }
  return lines.join("\n").trimEnd();
}

// Runs `gh api` with a JSON body from a temporary file. Returns the parsed JSON response, or null
// for a 404 when allow404 is set. Exits 3 on any other failure, without the request body.
function api([method, route], body = null, { allow404 = false } = {}) {
  const argv = ["api", "--method", method, route];
  let tmp = null;
  if (body) {
    tmp = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "gh-media-")), "body.json");
    fs.writeFileSync(tmp, JSON.stringify(body));
    argv.push("--input", tmp);
  }
  const res = spawnSync("gh", argv, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  if (tmp) fs.rmSync(path.dirname(tmp), { recursive: true, force: true });
  if (res.error) fail(`gh-media: cannot run gh: ${res.error.message}`, 3);
  if (res.status !== 0) {
    if (allow404 && /HTTP 404/.test(res.stderr)) return null;
    fail(`gh-media: gh api ${method} ${route} failed: ${res.stderr.trim().split("\n").slice(-1)[0]}`, 3);
  }
  return res.stdout.trim() ? JSON.parse(res.stdout) : {};
}

// Returns null when the arguments do not form one valid call.
function parseArgs(argv) {
  const args = { branch: DEFAULT_BRANCH, dryRun: false };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag === "--dry-run") args.dryRun = true;
    else if (["--repo", "--key", "--dir", "--url", "--branch"].includes(flag) && argv[i + 1]) args[flag.slice(2)] = argv[++i];
    else return null;
  }
  if (!args.repo || !args.key || !args.dir) return null;
  if (!REPO.test(args.repo) || !KEY.test(args.key) || !KEY.test(args.branch)) return null;
  if (args.url && !/^https?:\/\/[^\s"'<>()]+$/.test(args.url)) return null;
  args.dir = path.resolve(args.dir);
  return args;
}

function readJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (err) {
    return fail(`gh-media: cannot read ${file}: ${err.message}`, 2);
  }
}

function fail(message, code) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}
