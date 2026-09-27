import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const CLIENT = path.join(ROOT, "loop-jev.mjs");
const REPLAY = path.join(ROOT, "loop-jev-replay.mjs");
const FIXTURES = path.join(ROOT, "tests", "fixtures", "jev");
const VALID_INPUT = {
  stage: "route",
  state: { phase: "Add bounded client" },
  questions: {
    profile: {
      type: "choice",
      instructions: "Which profile should run this phase?",
      criteria: { default: "Established path", light: "Small low-risk task" },
    },
    risk: {
      type: "score",
      instructions: "How risky is this phase?",
      criteria: ["Low", "Medium", "High"],
    },
    proceed: { type: "noul", instructions: "Is the task ready?" },
  },
};

test("loads a local key file for direct Jev calls and keeps explicit off disabled", async (t) => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "loop-jev-key-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const keyFile = path.join(dir, "typesafe-api-key");
  await writeFile(keyFile, "file-key\n", { mode: 0o600 });
  const authorizations = [];
  const server = http.createServer(async (request, response) => {
    authorizations.push(request.headers.authorization);
    await readBody(request);
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({
      model: "jev-test", answers: { profile: { type: "choice", choice: "light",
        probabilities: { default: 0.1, light: 0.9 }, confidence: 0.9 } },
      usage: { input_tokens: 1, output_tokens: 1 },
    }));
  });
  t.after(() => server.close());
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const input = { stage: "route", state: { phase: "1" }, questions: {
    profile: { type: "choice", instructions: "Choose", criteria: { default: "Default", light: "Light" } },
  } };
  const env = { LOOP_JEV_KEY_FILE: keyFile,
    TYPESAFE_API_URL: `http://127.0.0.1:${server.address().port}/v1/systemone` };

  const routed = await runClient(input, env);
  assert.equal(routed.code, 0, routed.stderr);
  assert.equal(JSON.parse(routed.stdout).status, "ok");
  assert.equal(JSON.parse(routed.stdout).mode, "shadow");
  assert.deepEqual(authorizations, ["Bearer file-key"]);

  const disabled = await runClient(input, { ...env, LOOP_JEV_MODE: "off" });
  assert.equal(JSON.parse(disabled.stdout).reason, "disabled");
  assert.deepEqual(authorizations, ["Bearer file-key"]);

  const override = await runClient(input, { ...env, TYPESAFE_API_KEY: "env-key" });
  assert.equal(JSON.parse(override.stdout).status, "ok");
  assert.deepEqual(authorizations, ["Bearer file-key", "Bearer env-key"]);

  const newlyAvailable = await runClient(input, { ...env, LOOP_JEV_MODE: "off", LOOP_JEV_MODE_EXPLICIT: "0" });
  assert.equal(JSON.parse(newlyAvailable.stdout).mode, "shadow");
  assert.deepEqual(authorizations, ["Bearer file-key", "Bearer env-key", "Bearer file-key"]);
});

test("returns validated Choice, Score, and Noul answers", async (t) => {
  const server = http.createServer(async (request, response) => {
    assert.equal(request.method, "POST");
    assert.equal(request.url, "/v1/systemone");
    assert.equal(request.headers.authorization, "Bearer test-key");
    const body = JSON.parse(await readBody(request));
    assert.deepEqual(body, {
      state: VALID_INPUT.state,
      questions: VALID_INPUT.questions,
      model: "jev-latest",
    });

    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({
      model: "jev-1.13.0",
      answers: {
        profile: {
          type: "choice",
          choice: "light",
          probabilities: { default: 0.05, light: 0.95 },
          confidence: 0.9,
        },
        risk: {
          type: "score",
          score: 0.2,
          legend: { 0: "Low", 1: "Medium", 2: "High" },
          probabilities: { 0: 0.85, 1: 0.1, 2: 0.05 },
          confidence: 0.8,
        },
        proceed: { type: "noul", noul: 0.97 },
      },
      usage: { input_tokens: 50, output_tokens: 12 },
    }));
  });
  t.after(() => server.close());
  server.listen(0, "127.0.0.1");
  await once(server, "listening");

  const result = await runClient(VALID_INPUT, {
    TYPESAFE_API_KEY: "test-key",
    TYPESAFE_API_URL: `http://127.0.0.1:${server.address().port}/v1/systemone`,
    LOOP_JEV_MODE: "shadow",
    LOOP_JEV_ROUTE_MIN_CONFIDENCE: "0.7",
  });

  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), {
    version: 1,
    status: "ok",
    stage: "route",
    mode: "shadow",
    model: "jev-1.13.0",
    answers: {
      profile: {
        type: "choice",
        choice: "light",
        probabilities: { default: 0.05, light: 0.95 },
        confidence: 0.9,
      },
      risk: {
        type: "score",
        score: 0.2,
        probabilities: { 0: 0.85, 1: 0.1, 2: 0.05 },
        confidence: 0.8,
      },
      proceed: { type: "noul", noul: 0.97, confidence: 0.94 },
    },
    confidence: 0.8,
    usage: { inputTokens: 50, outputTokens: 12 },
  });
  assert.equal(result.stderr, "");
});

test("returns a timeout fallback within the configured bound", async (t) => {
  const server = http.createServer(() => {});
  t.after(() => {
    server.closeAllConnections();
    server.close();
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");

  const startedAt = Date.now();
  const result = await runClient(VALID_INPUT, {
    TYPESAFE_API_KEY: "test-key",
    TYPESAFE_API_URL: `http://127.0.0.1:${server.address().port}/v1/systemone`,
    LOOP_JEV_TIMEOUT_MS: "50",
  });

  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), {
    version: 1,
    status: "fallback",
    stage: "route",
    mode: "shadow",
    reason: "timeout",
  });
  assert.ok(Date.now() - startedAt < 1_000);
  assert.equal(result.stderr, "");
});

test("defaults to off and returns fallback when credentials are missing", async () => {
  const result = await runClient(VALID_INPUT);

  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), {
    version: 1,
    status: "fallback",
    stage: "route",
    mode: "off",
    reason: "missing_credentials",
  });
  assert.equal(result.stderr, "");
});

test("rejects a malformed typed response", async (t) => {
  const server = http.createServer((_request, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({
      model: "jev-1.13.0",
      answers: {
        profile: { type: "choice", choice: "invented", probabilities: {}, confidence: 2 },
      },
      usage: { input_tokens: 1, output_tokens: 1 },
    }));
  });
  t.after(() => server.close());
  server.listen(0, "127.0.0.1");
  await once(server, "listening");

  const result = await runClient(VALID_INPUT, {
    TYPESAFE_API_KEY: "test-key",
    TYPESAFE_API_URL: `http://127.0.0.1:${server.address().port}/v1/systemone`,
  });

  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), {
    version: 1,
    status: "fallback",
    stage: "route",
    mode: "shadow",
    reason: "invalid_response",
  });
  assert.equal(result.stderr, "");
});

test("returns secret-safe fallback for API errors", async (t) => {
  const secret = "secret-must-not-escape";
  const server = http.createServer((_request, response) => {
    response.writeHead(401, { "content-type": "application/json" });
    response.end(JSON.stringify({ detail: `invalid credential ${secret}` }));
  });
  t.after(() => server.close());
  server.listen(0, "127.0.0.1");
  await once(server, "listening");

  const result = await runClient(VALID_INPUT, {
    TYPESAFE_API_KEY: secret,
    TYPESAFE_API_URL: `http://127.0.0.1:${server.address().port}/v1/systemone`,
  });

  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), {
    version: 1,
    status: "fallback",
    stage: "route",
    mode: "shadow",
    reason: "api_error",
  });
  assert.equal(`${result.stdout}${result.stderr}`.includes(secret), false);
});

test("returns typed fallback when confidence is below the stage threshold", async (t) => {
  const server = http.createServer((_request, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({
      model: "jev-1.13.0",
      answers: {
        profile: {
          type: "choice",
          choice: "default",
          probabilities: { default: 0.55, light: 0.45 },
          confidence: 0.1,
        },
        risk: {
          type: "score",
          score: 1,
          legend: { 0: "Low", 1: "Medium", 2: "High" },
          probabilities: { 0: 0.2, 1: 0.6, 2: 0.2 },
          confidence: 0.4,
        },
        proceed: { type: "noul", noul: 0.7 },
      },
      usage: { input_tokens: 50, output_tokens: 12 },
    }));
  });
  t.after(() => server.close());
  server.listen(0, "127.0.0.1");
  await once(server, "listening");

  const result = await runClient(VALID_INPUT, {
    TYPESAFE_API_KEY: "test-key",
    TYPESAFE_API_URL: `http://127.0.0.1:${server.address().port}/v1/systemone`,
    LOOP_JEV_MODE: "active",
    LOOP_JEV_ROUTE_MIN_CONFIDENCE: "0.75",
  });
  const output = JSON.parse(result.stdout);

  assert.equal(result.code, 0);
  assert.equal(output.status, "fallback");
  assert.equal(output.reason, "low_confidence");
  assert.equal(output.mode, "active");
  assert.equal(output.confidence, 0.1);
  assert.equal(output.answers.profile.choice, "default");
  assert.equal(`${result.stdout}${result.stderr}`.includes("test-key"), false);
});

test("off mode bypasses the vendor even when credentials exist", async () => {
  const result = await runClient(VALID_INPUT, {
    TYPESAFE_API_KEY: "test-key",
    TYPESAFE_API_URL: "http://127.0.0.1:1/must-not-be-called",
    LOOP_JEV_MODE: "off",
  });

  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), {
    version: 1,
    status: "fallback",
    stage: "route",
    mode: "off",
    reason: "disabled",
  });
  assert.equal(result.stderr, "");
});

test("explicit off mode is disabled without credentials", async () => {
  const result = await runClient(VALID_INPUT, { LOOP_JEV_MODE: "off" });

  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), {
    version: 1,
    status: "fallback",
    stage: "route",
    mode: "off",
    reason: "disabled",
  });
});

test("replays all stages and reports agreement, fallback, and recorded metrics", async () => {
  const result = await runProcess(process.execPath, [
    REPLAY,
    path.join(FIXTURES, "replay-live.json"),
    path.join(FIXTURES, "replay-archived.json"),
  ]);

  assert.equal(result.code, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  assert.deepEqual(report.stages, {
    route: { total: 3, comparable: 1, agreements: 0, agreementRate: 0, fallbacks: 2, fallbackRate: 2 / 3 },
    question: { total: 1, comparable: 1, agreements: 1, agreementRate: 1, fallbacks: 0, fallbackRate: 0 },
    "merge-risk": { total: 2, comparable: 1, agreements: 1, agreementRate: 1, fallbacks: 1, fallbackRate: 0.5 },
  });
  assert.deepEqual(report.overall, {
    total: 6, comparable: 3, agreements: 2, agreementRate: 2 / 3, fallbacks: 3, fallbackRate: 0.5,
  });
  assert.deepEqual(report.metrics, {
    latency: { recorded: 2, averageMs: 150 },
    runnerRetries: { recorded: 1, total: 2 },
    cost: { recorded: 1, totalUsd: 0.0042 },
  });
  assert.match(report.notice, /synthetic fixtures.*no savings/i);
});

test("archive reload replay keeps decisions and marks absent metrics unavailable", async () => {
  const result = await runProcess(process.execPath, [REPLAY, path.join(FIXTURES, "replay-archived.json")]);

  assert.equal(result.code, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  assert.equal(report.sourceCount, 1);
  assert.equal(report.overall.total, 3);
  assert.deepEqual(report.metrics, {
    latency: "unavailable",
    runnerRetries: "unavailable",
    cost: "unavailable",
  });
  assert.deepEqual(report.fallbackReasons, { api_error: 1, disabled: 1, missing_credentials: 1 });
});

test("rejects stdin larger than the fixed input bound", async () => {
  const result = await runClient(`{"padding":"${"x".repeat(70_000)}"}`, {
    TYPESAFE_API_KEY: "test-key",
  });

  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), {
    version: 1,
    status: "fallback",
    stage: null,
    mode: "shadow",
    reason: "input_too_large",
  });
  assert.equal(result.stderr, "");
});

test("shell config uses the same mode defaults and bounds", async () => {
  const command = "source ./loop-models.conf; print -r -- \"$LOOP_JEV_MODE|$LOOP_JEV_ROUTE_MIN_CONFIDENCE|$LOOP_JEV_TIMEOUT_MS\"";
  const withoutKey = await runProcess("zsh", ["-c", command]);
  const withKey = await runProcess("zsh", ["-c", command], { TYPESAFE_API_KEY: "test-key" });
  const invalid = await runProcess("zsh", ["-c", command], {
    TYPESAFE_API_KEY: "test-key",
    LOOP_JEV_MODE: "invalid",
  });

  assert.equal(withoutKey.stdout, "off|0.8|5000");
  assert.equal(withKey.stdout, "shadow|0.8|5000");
  assert.equal(invalid.stdout, "off|0.8|5000");
});

test("shell config recognizes a local key file without exporting its contents", async (t) => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "loop-jev-config-key-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const keyFile = path.join(dir, "typesafe-api-key");
  await writeFile(keyFile, "file-key\n", { mode: 0o600 });
  const command = "source ./loop-models.conf; source ./loop-models.conf; print -r -- \"$LOOP_JEV_MODE|$LOOP_JEV_MODE_EXPLICIT|${TYPESAFE_API_KEY:-absent}\"";
  const result = await runProcess("zsh", ["-c", command], { LOOP_JEV_KEY_FILE: keyFile });

  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout, "shadow|0|absent");
});

test("sourced config distinguishes missing credentials from explicit off", async () => {
  const command = "source ./loop-models.conf; node ./loop-jev.mjs";
  const implicit = await runProcess("zsh", ["-c", command], {}, VALID_INPUT);
  const explicit = await runProcess("zsh", ["-c", command], { LOOP_JEV_MODE: "off" }, VALID_INPUT);

  assert.equal(implicit.code, 0, implicit.stderr);
  assert.equal(JSON.parse(implicit.stdout).reason, "missing_credentials");
  assert.equal(explicit.code, 0, explicit.stderr);
  assert.equal(JSON.parse(explicit.stdout).reason, "disabled");
});

test("implicit off remains implicit when supervisor and runner both source config", async () => {
  const command = "source ./loop-models.conf; source ./loop-models.conf; node ./loop-jev.mjs";
  const result = await runProcess("zsh", ["-c", command], {}, VALID_INPUT);

  assert.equal(result.code, 0, result.stderr);
  assert.equal(JSON.parse(result.stdout).reason, "missing_credentials");
});

test("returns invalid-input fallback for malformed JSON", async () => {
  const result = await runClient("{", { TYPESAFE_API_KEY: "test-key" });

  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), {
    version: 1,
    status: "fallback",
    stage: null,
    mode: "shadow",
    reason: "invalid_input",
  });
  assert.equal(result.stderr, "");
});

test("classifies a non-object API body as an invalid response", async (t) => {
  const server = http.createServer((_request, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end("null");
  });
  t.after(() => server.close());
  server.listen(0, "127.0.0.1");
  await once(server, "listening");

  const result = await runClient(VALID_INPUT, {
    TYPESAFE_API_KEY: "test-key",
    TYPESAFE_API_URL: `http://127.0.0.1:${server.address().port}/v1/systemone`,
  });

  assert.equal(result.code, 0);
  assert.equal(JSON.parse(result.stdout).reason, "invalid_response");
  assert.equal(result.stderr, "");
});

function runClient(input, extraEnv = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [CLIENT], {
      cwd: ROOT,
      env: { PATH: process.env.PATH, LOOP_JEV_KEY_FILE: path.join(ROOT, ".nonexistent-jev-key"), ...extraEnv },
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8").on("data", (chunk) => { stdout += chunk; });
    child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
    child.on("error", reject);
    child.on("close", (code) => resolve({ code, stdout: stdout.trim(), stderr: stderr.trim() }));
    child.stdin.end(typeof input === "string" ? input : JSON.stringify(input));
  });
}

function runProcess(command, args, extraEnv = {}, input) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: ROOT,
      env: { PATH: process.env.PATH, LOOP_JEV_KEY_FILE: path.join(ROOT, ".nonexistent-jev-key"), ...extraEnv },
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8").on("data", (chunk) => { stdout += chunk; });
    child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
    child.on("error", reject);
    child.on("close", (code) => resolve({ code, stdout: stdout.trim(), stderr: stderr.trim() }));
    child.stdin.end(input === undefined ? "" : JSON.stringify(input));
  });
}

async function readBody(stream) {
  let body = "";
  stream.setEncoding("utf8");
  for await (const chunk of stream) body += chunk;
  return body;
}
