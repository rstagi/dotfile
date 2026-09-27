import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REPLAY = path.join(ROOT, "loop-jev-replay.mjs");

test("replay follows the archived snapshot after a finished loop's worktree is removed", async (t) => {
  const dir = await mkdtemp(path.join(tmpdir(), "loop-jev-replay-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const loopDir = path.join(dir, "worktree");
  const storeFile = path.join(dir, "store.json");
  await mkdir(loopDir);
  await writeFile(storeFile, JSON.stringify({
    schemaVersion: 3,
    runId: "finished-loop",
    status: "finished",
    loopDir,
    decisions: [{ stage: "route", candidate: "light", appliedAction: "light", fallbackReason: null }],
    lastSnapshot: {
      decisions: [{ stage: "route", candidate: "light", appliedAction: "default", fallbackReason: "shadow-mode" }],
    },
  }));

  const live = runReplay(storeFile);
  assert.equal(live.overall.agreements, 1);
  assert.equal(live.overall.fallbacks, 0);

  await rm(loopDir, { recursive: true });
  const archived = runReplay(storeFile);
  assert.equal(archived.overall.agreements, 0);
  assert.equal(archived.overall.fallbacks, 1);
});

test("replay uses live decisions when a worktree exists despite an archived status label", async (t) => {
  const dir = await mkdtemp(path.join(tmpdir(), "loop-jev-replay-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const loopDir = path.join(dir, "worktree");
  const storeFile = path.join(dir, "store.json");
  await mkdir(loopDir);
  await writeFile(storeFile, JSON.stringify({
    runId: "reattached-loop",
    status: "archived",
    loopDir,
    decisions: [{ stage: "route", candidate: "light", appliedAction: "light" }],
    lastSnapshot: {
      decisions: [{ stage: "route", candidate: "light", appliedAction: "default" }],
    },
  }));

  assert.equal(runReplay(storeFile).overall.agreements, 1);
});

function runReplay(file) {
  const result = spawnSync(process.execPath, [REPLAY, file], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(result.stdout);
}
