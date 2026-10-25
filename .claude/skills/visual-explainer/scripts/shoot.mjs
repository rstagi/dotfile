#!/usr/bin/env node
// Screenshots of a rendered report page: page.png (the full page) and one <id>.png for each
// [data-shot] element. Zero dependencies: a headless Chrome over the DevTools protocol (CDP)
// through the global WebSocket of Node 22 or later.
//
//   node shoot.mjs --page <dir>/index.html --out <dir> [--chrome <bin>] [--width 1100] [--theme light|dark] [--scale 2]
//
// The script forces the theme (data-theme on <html> and the prefers-color-scheme media feature)
// and hides the walkthrough button. Each element PNG is the element box plus 16 px on each side.
// Chrome: --chrome, else $VE_CHROME, else Google Chrome in /Applications.
// Stdout: one absolute PNG path per line.
// Exit: 0 ok · 2 usage · 3 no browser, or a browser failure.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const USAGE = "usage: shoot.mjs --page <index.html> --out <dir> [--chrome <bin>] [--width <px>] [--theme light|dark] [--scale <n>]";
const DEFAULTS = { width: 1100, theme: "light", scale: 2 };
const PAD = 16;
const VIEW_HEIGHT = 900;
const TIMEOUT_MS = 60_000;
const PORT_TIMEOUT_MS = 15_000;
const SYSTEM_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
// The page script: theme, shot mode, every image decoded, then the boxes of the [data-shot] elements.
const PREPARE = (theme) => `(async () => {
  document.documentElement.setAttribute("data-theme", ${JSON.stringify(theme)});
  document.documentElement.classList.add("shot");
  await document.fonts.ready;
  await Promise.all([...document.images].map((img) => img.decode().catch(() => {})));
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  const doc = document.documentElement;
  const shots = [...document.querySelectorAll("[data-shot]")].map((el) => {
    const r = el.getBoundingClientRect();
    return { id: el.getAttribute("data-shot"), x: r.left + scrollX, y: r.top + scrollY, w: r.width, h: r.height };
  });
  return { width: doc.scrollWidth, height: doc.scrollHeight, shots };
})()`;

main(process.argv.slice(2)).catch((err) => fail(3, `shoot.mjs: ${err.message}`));

async function main(argv) {
  const args = parseArgs(argv);
  if (!args) fail(2, USAGE);
  const page = path.resolve(args.page);
  if (!fs.existsSync(page)) fail(2, `shoot.mjs: no page at ${page}`);
  const candidates = chromeCandidates(args.chrome);
  if (!candidates.length) fail(3, "shoot.mjs: no Chrome found; pass --chrome <bin> or set VE_CHROME");
  const out = path.resolve(args.out);
  fs.mkdirSync(out, { recursive: true });
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "ve-shoot-"));
  let browser = null;
  const timer = setTimeout(() => fail(3, `shoot.mjs: timed out after ${TIMEOUT_MS / 1000} s`, cleanup), TIMEOUT_MS);
  function cleanup() {
    clearTimeout(timer);
    browser?.kill("SIGKILL");
    // A killed Chrome can still write to its profile for a moment: retry, and never fail on it.
    try {
      fs.rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
    } catch (err) {
      process.stderr.write(`shoot.mjs: cannot remove ${profile}: ${err.message}\n`);
    }
  }
  try {
    let port;
    ({ browser, port } = await launch(candidates, profile));
    const files = await shoot(port, page, out, args);
    process.stdout.write(`${files.join("\n")}\n`);
  } catch (err) {
    fail(3, `shoot.mjs: ${err.message}`, cleanup);
  }
  cleanup();
  process.exit(0);
}

// Starts the first candidate that opens its DevTools port -> { browser, port }. A candidate that
// fails is killed, and the next one gets its own clean profile under `profile`. Throws with every
// failure reason.
async function launch(candidates, profile) {
  const reasons = [];
  for (const [i, chrome] of candidates.entries()) {
    const dir = path.join(profile, String(i));
    const browser = spawn(chrome, ["--headless", "--remote-debugging-port=0", `--user-data-dir=${dir}`, "--no-first-run", "--no-default-browser-check",
      // A mock keychain: a sandbox HOME has no login keychain, and Chrome would ask the user for one.
      "--use-mock-keychain", "--password-store=basic", "--hide-scrollbars", "--disable-gpu", "--disable-extensions", "--mute-audio", "about:blank"], { stdio: ["ignore", "ignore", "pipe"] });
    try {
      return { browser, port: await debugPort(browser) };
    } catch (err) {
      browser.kill("SIGKILL");
      reasons.push(`${chrome}: ${err.message}`);
    }
  }
  throw new Error(reasons.join("; "));
}

async function shoot(port, page, out, args) {
  const target = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" })).json();
  const cdp = await connect(target.webSocketDebuggerUrl);
  try {
    await cdp.send("Page.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: args.width, height: VIEW_HEIGHT, deviceScaleFactor: args.scale, mobile: false });
    await cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: args.theme }, { name: "prefers-reduced-motion", value: "reduce" }] });
    const loaded = cdp.once("Page.loadEventFired");
    await cdp.send("Page.navigate", { url: pathToFileURL(page).href });
    await loaded;
    const { result, exceptionDetails } = await cdp.send("Runtime.evaluate", { expression: PREPARE(args.theme), awaitPromise: true, returnByValue: true });
    if (exceptionDetails) throw new Error(`page script failed: ${exceptionDetails.text}`);
    const layout = result.value;
    const files = [];
    const capture = async (name, clip) => {
      const { data } = await cdp.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true, clip: { ...clip, scale: 1 } });
      const file = path.join(out, `${name}.png`);
      fs.writeFileSync(file, Buffer.from(data, "base64"));
      files.push(file);
    };
    await capture("page", { x: 0, y: 0, width: layout.width, height: layout.height });
    for (const s of layout.shots) {
      if (!s.w || !s.h) continue;
      const x = Math.max(0, Math.floor(s.x - PAD));
      const y = Math.max(0, Math.floor(s.y - PAD));
      const right = Math.min(layout.width, Math.ceil(s.x + s.w + PAD));
      const bottom = Math.min(layout.height, Math.ceil(s.y + s.h + PAD));
      await capture(safeName(s.id), { x, y, width: right - x, height: bottom - y });
    }
    return files;
  } finally {
    cdp.close();
  }
}

// Returns null when the arguments do not form one valid call.
function parseArgs(argv) {
  const args = { ...DEFAULTS };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    if (!["--page", "--out", "--chrome", "--width", "--theme", "--scale"].includes(flag) || value === undefined) return null;
    args[flag.slice(2)] = value;
    i += 1;
  }
  args.width = Number(args.width);
  args.scale = Number(args.scale);
  const ok = args.page && args.out && Number.isInteger(args.width) && args.width >= 280 && args.width <= 4000
    && args.scale > 0 && args.scale <= 4 && ["light", "dark"].includes(args.theme);
  return ok ? args : null;
}

// The Chrome binary to try: --chrome, else $VE_CHROME, else Google Chrome. None when the file
// does not exist.
function chromeCandidates(flag) {
  const candidate = flag || process.env.VE_CHROME || SYSTEM_CHROME;
  return fs.existsSync(candidate) ? [candidate] : [];
}

// The DevTools port: Chrome prints "DevTools listening on ws://127.0.0.1:<port>/..." on stderr.
function debugPort(browser) {
  return new Promise((resolve, reject) => {
    let text = "";
    const timer = setTimeout(() => reject(new Error(`no DevTools port after ${PORT_TIMEOUT_MS / 1000} s`)), PORT_TIMEOUT_MS);
    browser.on("error", (err) => reject(new Error(`cannot start Chrome: ${err.message}`)));
    browser.on("exit", (code, signal) => reject(new Error(`Chrome exited (${signal ?? `code ${code}`}) before it opened the DevTools port`)));
    browser.stderr.on("data", (chunk) => {
      text += chunk;
      const match = /DevTools listening on ws:\/\/[^:]+:(\d+)\//.exec(text);
      if (match) {
        clearTimeout(timer);
        resolve(Number(match[1]));
      }
    });
  });
}

// A minimal CDP client: send(method, params) -> result, once(event) -> params.
function connect(url) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const pending = new Map();
    const waiters = new Map();
    let next = 1;
    ws.addEventListener("error", () => reject(new Error("cannot connect to the DevTools socket")));
    ws.addEventListener("message", (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id && pending.has(msg.id)) {
        const { ok, ko, method } = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.error) ko(new Error(`${method}: ${msg.error.message}`));
        else ok(msg.result);
      } else if (msg.method && waiters.has(msg.method)) {
        waiters.get(msg.method)(msg.params);
        waiters.delete(msg.method);
      }
    });
    ws.addEventListener("open", () => resolve({
      send: (method, params = {}) => new Promise((ok, ko) => {
        const id = next++;
        pending.set(id, { ok, ko, method });
        ws.send(JSON.stringify({ id, method, params }));
      }),
      once: (method) => new Promise((ok) => waiters.set(method, ok)),
      close: () => ws.close(),
    }));
  });
}

function safeName(id) {
  return String(id).replace(/[^A-Za-z0-9_-]/g, "_");
}

function fail(code, message, cleanup) {
  cleanup?.();
  process.stderr.write(`${message}\n`);
  process.exit(code);
}
