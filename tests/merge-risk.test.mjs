import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const BUILDER = path.join(ROOT, "loop-jev-risk-input.mjs");
const POLICY = path.join(ROOT, "loop-jev-risk.mjs");
const SCOPE = ["none", "possible", "likely"];
const RISKS = ["low", "medium", "high"];
const REQUIRED_GATES = [
  "verified-exit-zero", "jev-risk", "full-diff-skim", "stall-check",
  "clean-worktree-check", "reread-steering-notes", "serialized-merge", "post-merge-verify",
];

test("builds bounded risk input from every changed path without patch text", async () => {
  const repo = await fixtureRepo();
  const result = await run(BUILDER, {
    repositoryRoot: repo, base: "HEAD^", head: "HEAD", phase: "5", attempt: 1,
    doneWhen: "Builder sees all paths; PASSWORD=plan-secret", verification: { exitCode: 0, summary: "tests passed; token=verify-secret" },
  });

  assert.equal(result.code, 0);
  assert.deepEqual(result.json.state.changedPaths.map(({ path: value }) => value), [".env", "src/app.js", "src/extra.js", "src/tab\tname.js"]);
  assert.equal(result.json.state.diff.files, 4);
  assert.equal(Object.hasOwn(result.json.state, "patchExcerpts"), false);
  assert.match(JSON.stringify(result.json), /\[REDACTED/);
  assert.doesNotMatch(JSON.stringify(result.json), /super-secret|ghp_1234567890|plan-secret|verify-secret/);
  assert.ok(Buffer.byteLength(JSON.stringify(result.json)) <= 48 * 1024);
  assert.deepEqual(Object.keys(result.json.questions), ["scopeGap", "changeRisk"]);
});

test("omits secret content when a secret file is renamed to an ordinary path", async () => {
  const repo = await bareRepo();
  await writeFile(path.join(repo, ".env"), "FOO=private-value-123456789\n");
  await command("git", ["-C", repo, "add", ".env"]);
  await command("git", ["-C", repo, "commit", "-qm", "secret"]);
  await command("git", ["-C", repo, "mv", ".env", "config.txt"]);
  await command("git", ["-C", repo, "commit", "-qm", "rename"]);

  const result = await run(BUILDER, riskRequest(repo));

  assert.equal(result.code, 0);
  assert.deepEqual(result.json.state.changedPaths, [{ status: "R", path: "config.txt", previousPath: ".env" }]);
  assert.doesNotMatch(JSON.stringify(result.json), /private-value-123456789/);
});

test("omits patch text when Git reports a secret rename as a deletion and addition", async () => {
  const repo = await bareRepo();
  await command("git", ["-C", repo, "config", "diff.renames", "false"]);
  await writeFile(path.join(repo, ".env"), "FOO=private-value-123456789\n");
  await command("git", ["-C", repo, "add", ".env"]);
  await command("git", ["-C", repo, "commit", "-qm", "secret"]);
  await command("git", ["-C", repo, "mv", ".env", "config.txt"]);
  await command("git", ["-C", repo, "commit", "-qm", "rename"]);

  const result = await run(BUILDER, riskRequest(repo));

  assert.equal(result.code, 0);
  assert.deepEqual(result.json.state.changedPaths, [
    { status: "D", path: ".env" }, { status: "A", path: "config.txt" },
  ]);
  assert.doesNotMatch(JSON.stringify(result.json), /private-value-123456789/);
});

test("omits content from common env file names", async () => {
  for (const secretPath of [".envrc", "prod.env", "prod.env.local"]) {
    const repo = await bareRepo();
    await writeFile(path.join(repo, "base.txt"), "base\n");
    await command("git", ["-C", repo, "add", "base.txt"]);
    await command("git", ["-C", repo, "commit", "-qm", "base"]);
    await writeFile(path.join(repo, secretPath), "FOO=private-value-123456789\n");
    await command("git", ["-C", repo, "add", secretPath]);
    await command("git", ["-C", repo, "commit", "-qm", "env file"]);

    const result = await run(BUILDER, riskRequest(repo));

    assert.equal(result.code, 0, secretPath);
    assert.doesNotMatch(JSON.stringify(result.json), /private-value-123456789/, secretPath);
  }
});

test("omits credential URLs in ordinary changed files", async () => {
  const repo = await bareRepo();
  await writeFile(path.join(repo, "base.txt"), "base\n");
  await command("git", ["-C", repo, "add", "base.txt"]);
  await command("git", ["-C", repo, "commit", "-qm", "base"]);
  await writeFile(path.join(repo, "app.js"), "const db = 'postgres://admin:hunter2@db/app';\n");
  await command("git", ["-C", repo, "add", "app.js"]);
  await command("git", ["-C", repo, "commit", "-qm", "app"]);

  const result = await run(BUILDER, riskRequest(repo));

  assert.equal(result.code, 0);
  assert.deepEqual(result.json.state.changedPaths, [{ status: "A", path: "app.js" }]);
  assert.doesNotMatch(JSON.stringify(result.json), /admin|hunter2/);
});

test("uses the merge base for paths and stats after parallel lanes diverge", async () => {
  const repo = await bareRepo();
  await writeFile(path.join(repo, "base.txt"), "base\n");
  await command("git", ["-C", repo, "add", "base.txt"]);
  await command("git", ["-C", repo, "commit", "-qm", "base"]);
  await command("git", ["-C", repo, "branch", "-M", "main"]);
  await command("git", ["-C", repo, "switch", "-qc", "lane-b"]);
  await writeFile(path.join(repo, "b.txt"), "b change\n");
  await command("git", ["-C", repo, "add", "b.txt"]);
  await command("git", ["-C", repo, "commit", "-qm", "lane b"]);
  await command("git", ["-C", repo, "switch", "-q", "main"]);
  await writeFile(path.join(repo, "a.txt"), "a change\n");
  await command("git", ["-C", repo, "add", "a.txt"]);
  await command("git", ["-C", repo, "commit", "-qm", "lane a"]);

  const result = await run(BUILDER, riskRequest(repo, { base: "main", head: "lane-b" }));

  assert.equal(result.code, 0);
  assert.deepEqual(result.json.state.changedPaths, [{ status: "A", path: "b.txt" }]);
  assert.deepEqual(result.json.state.diff, { files: 1, additions: 1, deletions: 0 });
  assert.doesNotMatch(JSON.stringify(result.json), /a change/);
});

test("records advice separately from mandatory full-skim disposition", async () => {
  const result = await run(POLICY, input());
  assert.equal(result.code, 0);
  assert.equal(result.json.candidate, "scope-gap:possible · risk:high");
  assert.equal(result.json.appliedAction, "focused-full-diff-skim");
  assert.equal(result.json.confidence, 0.91);
  assert.deepEqual(result.json.requiredGates, REQUIRED_GATES);
  assert.deepEqual(result.json.completedGates, REQUIRED_GATES.slice(0, 6));
  assert.deepEqual(result.json.remainingGates, REQUIRED_GATES.slice(6));
  assert.deepEqual(result.json.focus, ["scope gaps", "high-risk changes"]);
  assert.equal(result.json.evidenceChecked, true);
});

test("low-risk advice still records the complete mandatory gate sequence", async () => {
  const low = input();
  low.jev.answers.scopeGap = { type: "choice", choice: "none", confidence: 0.91, probabilities: distribution(SCOPE, "none") };
  low.jev.answers.changeRisk = { type: "choice", choice: "low", confidence: 0.91, probabilities: distribution(RISKS, "low") };
  low.focus = ["routine changes"];
  const result = await run(POLICY, low);
  assert.equal(result.code, 0);
  assert.equal(result.json.candidate, "scope-gap:none · risk:low");
  assert.equal(result.json.appliedAction, "focused-full-diff-skim");
  assert.deepEqual(result.json.requiredGates, REQUIRED_GATES);
});

for (const [name, override] of [
  ["error", { jev: { version: 1, status: "fallback", stage: "merge-risk", mode: "active", reason: "api_error" } }],
  ["shadow", { jev: { ...input().jev, mode: "shadow" } }],
  ["low confidence", { jev: { ...input().jev, status: "fallback", reason: "low_confidence" } }],
]) {
  test(`${name} risk still requires every deterministic gate`, async () => {
    const result = await run(POLICY, input({ ...override, action: "full-diff-skim", focus: [] }));
    assert.equal(result.code, 0);
    assert.equal(result.json.appliedAction, "full-diff-skim");
    assert.deepEqual(result.json.requiredGates, REQUIRED_GATES);
    assert.deepEqual(result.json.remainingGates, ["serialized-merge", "post-merge-verify"]);
  });
}

test("rejects any disposition that skips or reorders a deterministic gate", async () => {
  for (const completedGates of [
    ["verified-exit-zero", "jev-risk", "stall-check", "clean-worktree-check", "reread-steering-notes"],
    ["verified-exit-zero", "jev-risk", "stall-check", "full-diff-skim", "clean-worktree-check", "reread-steering-notes"],
    REQUIRED_GATES.slice(0, 5),
  ]) {
    const result = await run(POLICY, input({ completedGates }));
    assert.equal(result.code, 2);
    assert.match(result.json.error, /^required-gate:/);
  }
});

test("rejects dirty, stalled, unverified, or mismatched-attempt evidence", async () => {
  for (const overrides of [
    { headBefore: "abc123" }, { worktreeClean: false }, { verificationExitCode: 1 },
    { decisionHead: "old-head" }, { fullDiffSkimmed: false },
  ]) {
    const result = await run(POLICY, input(overrides));
    assert.equal(result.code, 2);
  }
});

function input(overrides = {}) {
  const jev = {
    version: 1, status: "ok", stage: "merge-risk", mode: "active", model: "jev-test", confidence: 0.91,
    answers: {
      scopeGap: { type: "choice", choice: "possible", confidence: 0.91, probabilities: distribution(SCOPE, "possible") },
      changeRisk: { type: "choice", choice: "high", confidence: 0.91, probabilities: distribution(RISKS, "high") },
    },
  };
  return {
    phase: "5", attempt: 1, headBefore: "base123", headAfter: "abc123", decisionHead: "abc123",
    verificationExitCode: 0, worktreeClean: true, fullDiffSkimmed: true,
    completedGates: REQUIRED_GATES.slice(0, 6), action: "focused-full-diff-skim",
    focus: ["scope gaps", "high-risk changes"],
    evidenceSources: ["done-when", "changed-paths", "diff-stat", "patch-excerpts", "verification", "full-diff"],
    jev, ts: "2026-09-27T10:00:00Z", ...overrides,
  };
}

function distribution(labels, selected) {
  const remainder = 0.09 / (labels.length - 1);
  return Object.fromEntries(labels.map((label) => [label, label === selected ? 0.91 : remainder]));
}

async function fixtureRepo() {
  const dir = await mkdtemp(path.join(os.tmpdir(), "loop-risk-"));
  await command("git", ["init", "-q", dir]);
  await command("git", ["-C", dir, "config", "user.email", "test@example.com"]);
  await command("git", ["-C", dir, "config", "user.name", "Test"]);
  await writeFile(path.join(dir, "base.txt"), "base\n");
  await command("git", ["-C", dir, "add", "."]);
  await command("git", ["-C", dir, "commit", "-qm", "base"]);
  await mkdir(path.join(dir, "src"));
  await writeFile(path.join(dir, ".env"), "TOKEN=super-secret\n");
  await writeFile(path.join(dir, "src", "app.js"), "const token = 'ghp_1234567890';\n");
  await writeFile(path.join(dir, "src", "extra.js"), "export const answer = 42;\n");
  await writeFile(path.join(dir, "src", "tab\tname.js"), "export const unusual = true;\n");
  await command("git", ["-C", dir, "add", ".env", "src"]);
  await command("git", ["-C", dir, "commit", "-qm", "change"]);
  return dir;
}

async function bareRepo() {
  const dir = await mkdtemp(path.join(os.tmpdir(), "loop-risk-"));
  await command("git", ["init", "-q", dir]);
  await command("git", ["-C", dir, "config", "user.email", "test@example.com"]);
  await command("git", ["-C", dir, "config", "user.name", "Test"]);
  return dir;
}

function riskRequest(repositoryRoot, overrides = {}) {
  return {
    repositoryRoot, base: "HEAD^", head: "HEAD", phase: "5", attempt: 1,
    doneWhen: "Review the changed paths", verification: { exitCode: 0, summary: "passed" },
    ...overrides,
  };
}

async function run(file, payload) {
  const child = spawn(process.execPath, [file], { stdio: ["pipe", "pipe", "pipe"] });
  let stdout = "";
  child.stdout.on("data", (chunk) => { stdout += chunk; });
  child.stdin.end(JSON.stringify(payload));
  const [code] = await new Promise((resolve) => child.on("close", (...args) => resolve(args)));
  return { code, json: JSON.parse(stdout) };
}

async function command(binary, args) {
  const child = spawn(binary, args, { stdio: "ignore" });
  const [code] = await new Promise((resolve) => child.on("close", (...args) => resolve(args)));
  assert.equal(code, 0, `${binary} ${args.join(" ")}`);
}
