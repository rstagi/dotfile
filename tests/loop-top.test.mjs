import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { inferLoop, phaseRows } from "../loop-top.mjs";

const CLI = path.join(import.meta.dirname, "..", "loop-top.mjs");

const loop = (over) => ({
  runId: "r1",
  status: "active",
  updatedAt: "2026-09-29T08:00:00Z",
  integrationBranch: "feat/x",
  repositories: ["acme/api"],
  coordinatorDir: "/w/coord",
  worktrees: [],
  branches: [],
  ...over,
});

test("inferLoop: cwd inside the coordinator worktree picks that loop", () => {
  const loops = [loop({ runId: "a", coordinatorDir: "/w/a" }), loop({ runId: "b", coordinatorDir: "/w/b" })];
  assert.equal(inferLoop({ cwd: "/w/b/src/lib" }, loops), "b");
  assert.equal(inferLoop({ cwd: "/w/b" }, loops), "b");
  assert.equal(inferLoop({ cwd: "/w/bb" }, loops), null);
});

test("inferLoop: runId from an ancestor .loop/state.json wins; archived loops never match", () => {
  const loops = [
    loop({ runId: "old", coordinatorDir: "/w/a", status: "archived" }),
    loop({ runId: "live", coordinatorDir: "/elsewhere" }),
  ];
  assert.equal(inferLoop({ cwd: "/w/a", localRunId: "live" }, loops), "live");
  assert.equal(inferLoop({ cwd: "/w/a" }, loops), null);
});

test("inferLoop: ties within a rule go to the most recently updated loop", () => {
  const loops = [
    loop({ runId: "older", coordinatorDir: "/w/a", updatedAt: "2026-09-01T00:00:00Z" }),
    loop({ runId: "newer", coordinatorDir: "/w/a", updatedAt: "2026-09-20T00:00:00Z" }),
  ];
  assert.equal(inferLoop({ cwd: "/w/a" }, loops), "newer");
});

test("inferLoop: cwd inside a phase worktree picks that loop, before any branch match", () => {
  const loops = [
    loop({ runId: "lane", worktrees: ["/home/.loop/worktrees/lane/acme--api/lane-a"] }),
    loop({ runId: "branchy", integrationBranch: "feat/a", updatedAt: "2026-12-01T00:00:00Z" }),
  ];
  const cwd = "/home/.loop/worktrees/lane/acme--api/lane-a/pkg";
  assert.equal(inferLoop({ cwd, branch: "feat/a", originSlug: "acme/api" }, loops), "lane");
});

test("inferLoop: git branch matches integration/phase branch only when origin repo is in the loop", () => {
  const loops = [
    loop({ runId: "int", integrationBranch: "feat/int", repositories: ["acme/api"] }),
    loop({ runId: "ph", integrationBranch: "feat/other", branches: ["fix/p1"], repositories: ["acme/web"] }),
  ];
  assert.equal(inferLoop({ cwd: "/x", branch: "feat/int", originSlug: "acme/api" }, loops), "int");
  assert.equal(inferLoop({ cwd: "/x", branch: "fix/p1", originSlug: "acme/web" }, loops), "ph");
  assert.equal(inferLoop({ cwd: "/x", branch: "feat/int", originSlug: "acme/web" }, loops), null);
  assert.equal(inferLoop({ cwd: "/x", branch: null, originSlug: null }, loops), null);
});

const node = (over) => ({
  id: "1", kind: "phase", title: "T", phase: "1", lane: "A", repository: "acme/api",
  status: "todo", ui: "todo", runtime: null, ...over,
});
const rt = (over) => ({
  attempt: 1, branch: null, engine: null, model: null, lastHeartbeatAgeSec: null,
  problem: null, awaiting: false, hilOpen: false, ...over,
});

test("phaseRows: phases in natural order (2a < 2b < 10), plan node dropped, worktree from state", () => {
  const snapshot = { graph: { nodes: [
    node({ id: "plan", kind: "plan", phase: null }),
    node({ id: "10", phase: "10", title: "Ten" }),
    node({ id: "2b", phase: "2b", title: "Two b", lane: "B" }),
    node({ id: "2a", phase: "2a", title: "Two a", status: "merged", ui: "done",
      runtime: rt({ engine: "codex", branch: "feat/rt-branch" }) }),
  ] } };
  const state = { phases: { "2a": { branch: "feat/two-a", worktree: "/wt/lane-a" } } };
  const rows = phaseRows(snapshot, state);
  assert.deepEqual(rows.map((r) => r.label), ["2a", "2b", "10"]);
  assert.deepEqual(rows[0], {
    id: "2a", label: "2a", title: "Two a", lane: "A", model: "codex", modelOverride: null, paused: false, noted: false, review: null, started: true, deps: [],
    icon: "✓", state: "done", branch: "feat/two-a", worktree: "/wt/lane-a",
  });
  assert.equal(rows[1].icon, "·");
  assert.equal(rows[1].state, "todo");
  assert.equal(rows[1].model, null);
  assert.equal(rows[1].worktree, null);
});

test("phaseRows: state text for running, HIL wait, problem, blocked and pr-review", () => {
  const snapshot = { graph: { nodes: [
    node({ id: "1", phase: "1", ui: "running", runtime: rt({ attempt: 1, lastHeartbeatAgeSec: 30 }) }),
    node({ id: "2", phase: "2", ui: "running", runtime: rt({ attempt: 2, lastHeartbeatAgeSec: 480 }) }),
    node({ id: "3", phase: "3", ui: "awaiting", runtime: rt({ awaiting: true }) }),
    node({ id: "4", phase: "4", ui: "running", runtime: rt({ hilOpen: true }) }),
    node({ id: "5", phase: "5", ui: "problem", runtime: rt({ problem: "verify-fail" }) }),
    node({ id: "6", phase: "6", ui: "blocked", runtime: rt({ problem: "blocked" }) }),
    node({ id: "7", phase: "7", kind: "pr-review", lane: "review", title: "Review PRs", ui: "todo" }),
  ] } };
  const rows = phaseRows(snapshot, null);
  assert.deepEqual(rows.map((r) => [r.icon, r.state]), [
    ["◐", "running · hb 30s"],
    ["◐", "attempt 2 · hb 8m"],
    ["⏸", "waiting on you (orchestrator)"],
    ["⏸", "waiting on you (orchestrator)"],
    ["✗", "verify-fail"],
    ["✗", "blocked"],
    ["·", "todo"],
  ]);
  assert.equal(rows[6].title, "Review PRs");
});

// ---- CLI smoke: `loop-top --once` against a fake daemon and a store-only fixture ----

function fixture({ snapshot = null } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "loop-top-"));
  const coord = path.join(root, "coord");
  const store = path.join(root, "store");
  fs.mkdirSync(path.join(coord, ".loop"), { recursive: true });
  fs.mkdirSync(store);
  const snap = (title) => ({ graph: { nodes: [
    node({ id: "plan", kind: "plan", phase: null, title: "The plan", ui: "running" }),
    node({ id: "1", phase: "1", title, ui: "done", status: "merged", runtime: rt({ engine: "codex" }) }),
    node({ id: "2", phase: "2", title: "Second", ui: "todo" }),
  ], edges: [{ source: "1", target: "2", kind: "depends", blocking: false }] } });
  const record = {
    runId: "loop-demo", status: "active", loopDir: path.join(coord, ".loop"),
    integrationBranch: "feat/demo", startedAt: "2026-09-28T10:00:00Z", updatedAt: "2026-09-28T11:00:00Z",
    repositories: { "acme/api": { integrationBranch: "feat/demo" } },
    lastSnapshot: snapshot ?? snap("Stored title"),
    lastState: { phases: { "1": { branch: "feat/one", worktree: "/wt/lane-a" } } },
  };
  fs.writeFileSync(path.join(store, "loop-demo.json"), JSON.stringify(record));
  return { root, coord, store, liveSnapshot: snap("Live title") };
}

function fakeDaemon(snapshot) {
  const server = http.createServer((req, res) => {
    res.setHeader("Content-Type", "application/json");
    if (req.url === "/api/loops") {
      return res.end(JSON.stringify([{ runId: "loop-demo", status: "active", integrationBranch: "feat/demo",
        repositories: [{ slug: "acme/api", integrationBranch: "feat/demo" }], updatedAt: "2026-09-28T11:00:00Z", startedAt: "2026-09-28T10:00:00Z" }]));
    }
    if (req.url === "/api/loops/loop-demo/snapshot") return res.end(JSON.stringify(snapshot));
    res.statusCode = 404;
    res.end("{}");
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

function runCli(args, { cwd, env }) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [CLI, ...args], { cwd, env: { PATH: process.env.PATH, ...env } });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("close", (code) => resolve({ code, stdout, stderr }));
  });
}

test("loop-top --once: infers the loop from cwd and renders the daemon snapshot", async (t) => {
  const fx = fixture();
  const server = await fakeDaemon(fx.liveSnapshot);
  t.after(() => { server.close(); fs.rmSync(fx.root, { recursive: true, force: true }); });
  const env = { LOOP_DAEMON_URL: `http://127.0.0.1:${server.address().port}`, LOOP_STORE_DIR: fx.store };

  const out = await runCli(["--once", "--worktrees"], { cwd: fx.coord, env });
  assert.equal(out.code, 0, out.stderr);
  const lines = out.stdout.split("\n");
  assert.match(lines[0], /^loop-demo · active · .* · feat\/demo$/);
  assert.match(lines[1], /^repos: acme\/api$/);
  assert.match(out.stdout, /✓ 1 +Live title +A +codex +done/);
  assert.match(out.stdout, /feat\/one → \/wt\/lane-a/);
  assert.match(out.stdout, /· 2 +Second +A +— +← 1 +todo/);
  assert.doesNotMatch(out.stdout, /Stored title|The plan/);
  assert.match(out.stdout, /· live$/m);
});

test("loop-top --once: daemon down falls back to the store record", async (t) => {
  const fx = fixture();
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const env = { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store };

  const out = await runCli(["--once"], { cwd: path.join(fx.coord, ".loop"), env });
  assert.equal(out.code, 0, out.stderr);
  assert.match(out.stdout, /✓ 1 +Stored title/);
  assert.match(out.stdout, /daemon offline · from store$/m);
});

test("loop-top --once: no inferable loop exits non-zero with a hint", async (t) => {
  const fx = fixture();
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const env = { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store };

  const out = await runCli(["--once"], { cwd: fx.root, env });
  assert.equal(out.code, 1);
  assert.match(out.stderr, /no loop/i);
  const explicit = await runCli(["--once", "loop-demo"], { cwd: fx.root, env });
  assert.equal(explicit.code, 0, explicit.stderr);
});

test("phaseRows: deps from `depends` edges; a todo phase with unsatisfied deps says what it waits on", () => {
  const snapshot = { graph: {
    nodes: [
      node({ id: "plan", kind: "plan", phase: null }),
      node({ id: "1", phase: "1", ui: "done" }),
      node({ id: "2a", phase: "2a", ui: "running" }),
      node({ id: "2b", phase: "2b", ui: "done" }),
      node({ id: "3", phase: "3", ui: "todo" }),
    ],
    edges: [
      { source: "plan", target: "1", kind: "plan-to-lane", blocking: false },
      { source: "1", target: "2a", kind: "depends", blocking: false },
      { source: "1", target: "2b", kind: "depends", blocking: false },
      { source: "2b", target: "3", kind: "depends", blocking: false },
      { source: "2a", target: "3", kind: "depends", blocking: true },
    ],
  } };
  const rows = phaseRows(snapshot, null);
  assert.deepEqual(rows.map((r) => r.deps), [[], ["1"], ["1"], ["2a", "2b"]]);
  assert.equal(rows[3].state, "waiting on 2a");
  assert.equal(rows[3].icon, "·");
});

test("loop-top --once --graph: phases drawn as git-log rails (forks, joins, transitive edges dropped)", async (t) => {
  // 1 → 2 → {3, 8}; 3 → 4 → 5 → 6; 8 → 6 → 7; 8 → 7 is transitive (via 6) and not drawn.
  const ids = ["1", "2", "3", "4", "5", "6", "7", "8"];
  const ui = { 6: "awaiting", 8: "awaiting" };
  const deps = [["1", "2"], ["2", "3"], ["2", "8"], ["3", "4"], ["4", "5"], ["5", "6"], ["8", "6"], ["6", "7"], ["8", "7"]];
  const snapshot = { graph: {
    nodes: ids.map((id) => node({ id, phase: id, title: `P${id}`, ui: ui[id] ?? "done" })),
    edges: deps.map(([source, target]) => ({ source, target, kind: "depends", blocking: false })),
  } };
  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const env = { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store };

  const out = await runCli(["--once", "--graph", "loop-demo"], { cwd: fx.root, env });
  assert.equal(out.code, 0, out.stderr);
  const body = out.stdout.split("\n").slice(3, -3).map((l) => l.replace(/ +$/, ""));
  assert.deepEqual(body.map((l) => l.replace(/^(.{6}\S*)\s+(P\d).*$/, "$1 $2")), [
    "  ✓   1 P1",
    "  ✓   2 P2",
    "  ├─╮",
    "  ✓ │ 3 P3",
    "  ✓ │ 4 P4",
    "  ✓ │ 5 P5",
    "  │ ⏸ 8 P8",
    "  ├─╯",
    "  ⏸   6 P6",
    "  ✓   7 P7",
  ]);
});

test("phaseRows: actual engine:model, a user model override, and paused phases", () => {
  const snapshot = { graph: { nodes: [
    node({ id: "1", phase: "1", ui: "done", runtime: rt({ engine: "codex", model: "gpt-6-sol" }) }),
    node({ id: "2", phase: "2", ui: "paused", paused: true, runtime: rt({ engine: "claude", model: "claude-sonnet-5" }) }),
    node({ id: "3", phase: "3", ui: "todo", modelOverride: "claude:claude-opus-5-5" }),
  ] } };
  const rows = phaseRows(snapshot, null);
  assert.deepEqual(rows.map((r) => [r.model, r.modelOverride, r.paused]), [
    ["codex:gpt-6-sol", null, false],
    ["claude:claude-sonnet-5", null, true],
    [null, "claude:claude-opus-5-5", false],
  ]);
  assert.deepEqual(rows.map((r) => [r.icon, r.state]), [["✓", "done"], ["‖", "paused"], ["·", "todo"]]);
});

test("phaseRows: a resumed attempt shows the model that actually ran (from the attempt history)", () => {
  const attempts = [
    { k: 1, engine: "codex", model: "gpt-5.6-sol", outcome: "done" },
    { k: 2, engine: "codex", model: "resume", outcome: "done" },
  ];
  const snapshot = { graph: { nodes: [
    node({ id: "1", phase: "1", ui: "done", runtime: rt({ engine: "codex", model: "resume", attempts }) }),
    node({ id: "2", phase: "2", ui: "done", runtime: rt({ engine: "claude", model: "resume", attempts: [] }) }),
  ] } };
  assert.deepEqual(phaseRows(snapshot, null).map((r) => r.model), ["codex:gpt-5.6-sol", "claude"]);
});

test("phaseRows: `started` mirrors the daemon's model-override rule (status past todo, or an attempt)", () => {
  const snapshot = { graph: { nodes: [
    node({ id: "1", phase: "1", status: "todo", ui: "todo" }),
    node({ id: "2", phase: "2", status: "running", ui: "running" }),
    node({ id: "3", phase: "3", status: "todo", ui: "paused", paused: true }),
    node({ id: "4", phase: "4", status: "todo", ui: "todo", runtime: rt({ attempt: 1 }) }),
  ] } };
  assert.deepEqual(phaseRows(snapshot, null).map((r) => r.started), [false, true, false, true]);
});

test("phaseRows + --once: an annotated phase (non-empty note) is flagged and shows 📝 before its title", async (t) => {
  const snapshot = { graph: { nodes: [
    node({ id: "1", phase: "1", title: "Noted", ui: "todo", noteMarkdown: "use pnpm" }),
    node({ id: "2", phase: "2", title: "Blank", ui: "todo", noteMarkdown: "  \n" }),
    node({ id: "3", phase: "3", title: "Plain", ui: "todo" }),
  ] } };
  assert.deepEqual(phaseRows(snapshot, null).map((r) => r.noted), [true, false, false]);

  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.match(out.stdout, /· 1 +📝 Noted/);
  assert.match(out.stdout, /· 3 +Plain/);
});

test("phaseRows + --once: review phases carry their tier; the model column shows tier × rounds", async (t) => {
  const snapshot = { graph: { nodes: [
    node({ id: "1", phase: "1", title: "Work", ui: "done" }),
    node({ id: "2", phase: "2", kind: "pr-review", lane: "review", title: "Deep review", ui: "todo", review: { tier: "max", rounds: 2 } }),
    node({ id: "3", phase: "3", kind: "pr-review", lane: "review", title: "Default review", ui: "todo", review: { tier: "medium", rounds: null } }),
  ] } };
  assert.deepEqual(phaseRows(snapshot, null).map((r) => r.review), [null, { tier: "max", rounds: 2 }, { tier: "medium", rounds: null }]);

  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.match(out.stdout, /2 +Deep review +review +max review ×2 /);
  assert.match(out.stdout, /3 +Default review +review +medium review ×3 /); // tier default from loop-models.conf
});
