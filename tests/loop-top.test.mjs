import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fitRows, gateText, inferLoop, milestoneTitles, phaseRows, pickerRows, pipelineSummary, renderDetails, tierChoices } from "../loop-top.mjs";

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
    id: "2a", label: "2a", title: "Two a", lane: "A", model: "codex", modelOverride: null, planned: null, paused: false, noted: false, review: null, stages: [], started: true, deps: [],
    icon: "✓", milestone: null, gate: null, state: "done", branch: "feat/two-a", worktree: "/wt/lane-a",
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
  fs.mkdirSync(path.join(root, "bin"));
  fs.writeFileSync(path.join(root, "bin", "ws"), "#!/bin/sh\nexit 1\n", { mode: 0o755 });
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
    const child = spawn(process.execPath, [CLI, ...args], { cwd, env: { HOME: path.dirname(env.LOOP_STORE_DIR), WS_SOCK: path.join(env.LOOP_STORE_DIR, "wsd.sock"), PATH: `${path.join(path.dirname(env.LOOP_STORE_DIR), "bin")}:${process.env.PATH}`, ...env } });
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
  assert.match(out.stdout, /· 2 +Second +A +\(codex:@sol\) +← 1 +todo/); // planned: first CHAIN_TASK leg
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
    node({ id: "3", phase: "3", kind: "pr-review", lane: "review", title: "Default review", ui: "todo", review: { tier: "standard", rounds: null } }),
  ] } };
  assert.deepEqual(phaseRows(snapshot, null).map((r) => r.review), [null, { tier: "max", rounds: 2 }, { tier: "standard", rounds: null }]);

  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.match(out.stdout, /2 +Deep review +review +max review ×2 /);
  assert.match(out.stdout, /3 +Default review +review +standard review ×2 /);
});

test("--once: shared rounds apply to every tier; explicit historical rounds stay visible", async (t) => {
  const snapshot = { graph: { nodes: [
    node({ id: "1", phase: "1", kind: "pr-review", review: { tier: "standard", rounds: null } }),
    node({ id: "2", phase: "2", kind: "pr-review", review: { tier: "max", rounds: null } }),
    node({ id: "3", phase: "3", kind: "pr-review", review: { tier: "medium", rounds: 3 } }),
    node({ id: "4", phase: "4", kind: "pr-review", review: { tier: "shallow", rounds: 1 } }),
  ] } };
  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const conf = path.join(fx.root, "models.conf");
  fs.writeFileSync(conf, "LOOP_REVIEW_ROUNDS_DEFAULT=4\nLOOP_REVIEW_ROUNDS_MAX=5\n");
  const env = { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store, LOOP_MODELS_CONF: conf };
  const configured = await runCli(["--once", "loop-demo"], { cwd: fx.root, env });
  assert.equal(configured.code, 0, configured.stderr);
  assert.match(configured.stdout, /standard review ×4/);
  assert.match(configured.stdout, /max review ×4/);
  assert.match(configured.stdout, /medium review ×3/);
  assert.match(configured.stdout, /shallow review ×1/);
  fs.writeFileSync(conf, "");
  const fallback = await runCli(["--once", "loop-demo"], { cwd: fx.root, env });
  assert.equal(fallback.code, 0, fallback.stderr);
  assert.match(fallback.stdout, /standard review ×2/);
  assert.match(fallback.stdout, /max review ×2/);
});

const stage = (k, stg, chain, state, repository = "acme/api", over = {}) =>
  ({ repository, k, stage: stg, chain, state, engine: null, model: null, summary: null, ...over });
const SHALLOW2 = (s1, s2, s3, repository) => [
  stage(1, "round1", "review-adv-a", s1, repository),
  stage(2, "fix1", "review-fix", s2, repository),
  stage(3, "final", "review-final", s3, repository),
];

const EARLY_STOP = (finalState = "running") => [
  stage(1, "round1", "review-gpt", "done", "acme/api", { role: "review", provider: "gpt", verdict: "APPROVE" }),
  stage(2, "round1", "review-claude", "done", "acme/api", { role: "critic", provider: "claude", verdict: "AGREE" }),
  stage(3, "round1", "review-grok", "done", "acme/api", { role: "confirm", provider: "grok", verdict: "AGREE" }),
  stage(-1, "fix1", "review-fix", "skipped"),
  stage(-2, "round2", "review-claude", "skipped"),
  stage(-3, "round2", "review-grok", "skipped"),
  stage(4, "final", "review-final", finalState),
];

test("--once: clean early stop shows final as current and skips outside pending progress", async (t) => {
  const reviewStages = EARLY_STOP();
  const snapshot = { graph: { nodes: [
    node({ kind: "pr-review", ui: "running", status: "running", review: { tier: "standard", rounds: 2, inner: 2 }, reviewStages }),
  ] } };
  assert.equal(phaseRows(snapshot, null)[0].state, "final · 3/4");
  assert.equal(pipelineSummary(reviewStages)[0].text, "round1 ✓✓✓ › fix1 ↷ › round2 ↷↷ › final ◐");
  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.equal(out.code, 0, out.stderr);
  assert.match(out.stdout, /final · 3\/4/);
  assert.match(out.stdout, /round1 ✓✓✓ › fix1 ↷ › round2 ↷↷ › final ◐/);
});

test("--once: an entirely planned AR pipeline stays todo until a run starts", async (t) => {
  const snapshot = { graph: { nodes: [
    node({ kind: "pr-review", review: { tier: "standard", rounds: 2, inner: 2 }, reviewStages: [
      stage(1, "round1", "review-gpt", "planned"), stage(2, "round1", "review-claude", "planned"),
      stage(3, "final", "review-final", "planned"),
    ] }),
  ] } };
  assert.equal(phaseRows(snapshot, null)[0].state, "todo");
  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.equal(out.code, 0, out.stderr);
  assert.doesNotMatch(out.stdout, /round1|final ·/);
});

test("renderDetails: AR chains resolve the tier's provider models and show available tiers", () => {
  for (const [tier, gpt, claude] of [["standard", "codex:@sol", "claude:opus"], ["max", "codex:@astra", "claude:fable"]]) {
    const snapshot = { graph: { nodes: [node({ kind: "pr-review", review: { tier, rounds: 2, inner: 2 }, reviewStages: [
      stage(1, "round1", "review-gpt", "planned"), stage(2, "round1", "review-claude", "planned"),
      stage(3, "round1", "review-grok", "planned"), stage(4, "fix1", "review-fix", "planned"),
      stage(5, "final", "review-final", "planned"),
    ] })] } };
    const text = renderDetails({ ref: loop(), snapshot, state: null, source: "store", at: new Date() }, "1", { width: 180, color: false });
    assert.match(text, /tier: standard \| max/);
    assert.ok(text.includes(`(${gpt})`), text);
    assert.ok(text.includes(`(${claude})`), text);
    assert.match(text, /grok +· planned +\(grok:default\)/);
    assert.match(text, /fix +· planned +\(claude:opus\)/);
    assert.match(text, /final +· planned +\(claude:opus\)/);
  }
});

test("phaseRows: a started review phase's state names its current sub-stage + progress", () => {
  const review = (id, ui, reviewStages) => node({ id, phase: id, kind: "pr-review", lane: "review", title: "R", ui, status: "running", review: { tier: "shallow", rounds: 2 }, reviewStages });
  const snapshot = { graph: { nodes: [
    review("1", "running", SHALLOW2("done", "running", "todo")),
    review("2", "running", [...SHALLOW2("done", "done", "todo"), ...SHALLOW2("running", "todo", "todo", "acme/web")]),
    review("3", "running", SHALLOW2("failed", "todo", "todo")),
    review("4", "todo", SHALLOW2("todo", "todo", "todo")),
    review("5", "done", SHALLOW2("done", "done", "done")),
  ] } };
  const rows = phaseRows(snapshot, null);
  assert.deepEqual(rows.map((r) => r.state), ["fix1 · 1/3", "final, round1 · 2/6", "round1 failed", "todo", "done"]);
  assert.equal(rows[0].stages.length, 3);
});

test("pipelineSummary: one line per repository, runs grouped by sub-stage", () => {
  assert.deepEqual(pipelineSummary(SHALLOW2("done", "running", "todo")), [
    { repository: "acme/api", text: "round1 ✓ › fix1 ◐ › final ·" },
  ]);
  const medium = [
    stage(1, "round1", "review-adv-a", "done"), stage(2, "round1", "review-adv-b", "failed"),
    stage(3, "final", "review-final", "todo"),
  ];
  assert.deepEqual(pipelineSummary([...medium, ...SHALLOW2("question", "todo", "todo", "acme/web")]).map((l) => l.text), [
    "round1 ✓✗ › final ·",
    "round1 ⏸ › fix1 · › final ·",
  ]);
});

test("--once: a running review phase shows its sub-stage pipeline under the row", async (t) => {
  const snapshot = { graph: { nodes: [
    node({ id: "1", phase: "1", title: "Work", ui: "done" }),
    node({ id: "2", phase: "2", kind: "pr-review", lane: "review", title: "Review", ui: "running", status: "running",
      review: { tier: "shallow", rounds: 2 }, reviewStages: SHALLOW2("done", "running", "todo") }),
  ] } };
  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.match(out.stdout, /2 +Review .* fix1 · 1\/3/);
  assert.match(out.stdout, /\n +round1 ✓ › fix1 ◐ › final ·\n/);
});

// ---- archive / unarchive: a manual flag on the daemon record (worktrees untouched) ----

const SERVER = path.join(import.meta.dirname, "..", "loop-web", "server", "index.mjs");

async function realDaemon(store) {
  const port = 7900 + Math.floor(Math.random() * 500);
  const child = spawn(process.execPath, [SERVER, "--daemon", "--port", String(port)], {
    env: { PATH: process.env.PATH, LOOP_STORE_DIR: store }, stdio: "ignore",
  });
  const url = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 50; i++) {
    const ok = await fetch(`${url}/api/health`).then((r) => r.ok).catch(() => false);
    if (ok) return { url, stop: () => new Promise((r) => { child.once("exit", r); child.kill(); }) };
    await new Promise((r) => setTimeout(r, 100));
  }
  child.kill();
  throw new Error("daemon did not start");
}

const listed = async (url) => Object.fromEntries((await (await fetch(`${url}/api/loops`)).json()).map((l) => [l.runId, l.status]));
const stored = (fx) => JSON.parse(fs.readFileSync(path.join(fx.store, "loop-demo.json"), "utf8"));

test("loop-top archive: archives the inferred loop on the daemon (persisted); unarchive restores it", async (t) => {
  const fx = fixture();
  const daemon = await realDaemon(fx.store);
  t.after(async () => { await daemon.stop(); fs.rmSync(fx.root, { recursive: true, force: true }); });
  const env = { LOOP_DAEMON_URL: daemon.url, LOOP_STORE_DIR: fx.store };
  assert.equal((await listed(daemon.url))["loop-demo"], "active");

  const out = await runCli(["archive", "--yes"], { cwd: fx.coord, env });
  assert.equal(out.code, 0, out.stderr);
  assert.match(out.stdout, /archived loop-demo/);
  assert.equal((await listed(daemon.url))["loop-demo"], "archived");
  await new Promise((r) => setTimeout(r, 700)); // store writes are debounced
  assert.equal(stored(fx).archived, true);
  assert.ok(fs.existsSync(path.join(fx.coord, ".loop")), "worktree untouched");

  const un = await runCli(["unarchive", "loop-demo"], { cwd: fx.root, env });
  assert.equal(un.code, 0, un.stderr);
  assert.equal((await listed(daemon.url))["loop-demo"], "active");
  const declined = await runCli(["archive", "loop-demo"], { cwd: fx.root, env }); // no tty, no --yes
  assert.match(declined.stdout, /not archived/);
  assert.equal((await listed(daemon.url))["loop-demo"], "active");
});

test("loop-top archive: unknown loop fails; no inferable loop fails with a hint", async (t) => {
  const fx = fixture();
  const daemon = await realDaemon(fx.store);
  t.after(async () => { await daemon.stop(); fs.rmSync(fx.root, { recursive: true, force: true }); });
  const env = { LOOP_DAEMON_URL: daemon.url, LOOP_STORE_DIR: fx.store };
  const bad = await runCli(["archive", "nope", "--yes"], { cwd: fx.root, env });
  assert.notEqual(bad.code, 0);
  assert.match(bad.stderr, /no such loop/);
  const none = await runCli(["archive", "--yes"], { cwd: fx.root, env });
  assert.notEqual(none.code, 0);
  assert.match(none.stderr, /pass a runId/);
});

test("loop-top archive: daemon down → flag written to the store record; hidden from --once inference", async (t) => {
  const fx = fixture();
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const env = { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store };
  const out = await runCli(["archive", "loop-demo", "--yes"], { cwd: fx.root, env });
  assert.equal(out.code, 0, out.stderr);
  assert.equal(stored(fx).archived, true);
  const once = await runCli(["--once"], { cwd: fx.coord, env });
  assert.notEqual(once.code, 0, "archived loop no longer inferred");
});

test("loop-top --once: phases without a model yet show the planned leg (override first, else CHAIN_TASK[0])", async (t) => {
  const fx = fixture();
  const conf = path.join(fx.root, "models.conf");
  fs.writeFileSync(conf, 'CHAIN_TASK=("claude:planned-model" "codex:@sol")\n');
  fs.mkdirSync(path.join(fx.coord, ".loop", "control"), { recursive: true });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const env = { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store, LOOP_MODELS_CONF: conf };
  const out = await runCli(["--once"], { cwd: fx.coord, env });
  assert.equal(out.code, 0, out.stderr);
  assert.match(out.stdout, /✓ 1 +Stored title +A +codex +done/, "actual model unchanged");
  assert.match(out.stdout, /· 2 +Second +A +\(claude:planned-model\)/);
});

test("pickerRows: active first, archived hidden, runId in the 2nd field (fzf {2})", () => {
  const rows = pickerRows([
    { runId: "fin", status: "finished", updatedAt: "2026-09-02T00:00:00Z" },
    { runId: "gone", status: "archived", updatedAt: "2026-09-03T00:00:00Z" },
    { runId: "live", status: "active", updatedAt: "2026-09-01T00:00:00Z" },
  ]);
  assert.deepEqual(rows.map((r) => r.split(/\s+/)[1]), ["live", "fin"]);
});

test("loop-top --picker-rows: lists pickable loops; an archived one drops out (picker reload after `a`)", async (t) => {
  const fx = fixture();
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const env = { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store };
  const before = await runCli(["--picker-rows"], { cwd: fx.root, env });
  assert.equal(before.code, 0, before.stderr);
  assert.match(before.stdout, /^active +loop-demo /m);
  await runCli(["archive", "loop-demo", "--yes"], { cwd: fx.root, env });
  const after = await runCli(["--picker-rows"], { cwd: fx.root, env });
  assert.doesNotMatch(after.stdout, /loop-demo/);
});

// ---- review tier: `m` on a review phase picks a tier; the daemon rewrites the plan tag ----

test("control review: rewrites the review phase's [review:] tag in plan.md; guards bad input + started phases", async (t) => {
  const fx = fixture();
  const loopDir = path.join(fx.coord, ".loop");
  const planFile = path.join(loopDir, "plan.md");
  fs.writeFileSync(planFile, [
    "# Demo — Multi-Phase Plan", "",
    "### Phase 1 — Build `[lane: A]` `[status: done]`", "Build it.", "",
    "### Phase 2 — Review pull requests `[lane: review]` `[status: todo]` `[kind: pr-review]` `[review: medium]` `[rounds: 2]`",
    "Review it.", "",
  ].join("\n"));
  const rec = JSON.parse(fs.readFileSync(path.join(fx.store, "loop-demo.json"), "utf8"));
  fs.writeFileSync(path.join(fx.store, "loop-demo.json"), JSON.stringify({ ...rec, planFile }));
  const daemon = await realDaemon(fx.store);
  t.after(async () => { await daemon.stop(); fs.rmSync(fx.root, { recursive: true, force: true }); });
  const control = (body) => fetch(`${daemon.url}/api/loops/loop-demo/control`, {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
  });

  const ok = await control({ action: "review", phase: "2", tier: "max" });
  assert.equal(ok.status, 200, await ok.text());
  const header = fs.readFileSync(planFile, "utf8").split("\n").find((l) => l.startsWith("### Phase 2"));
  assert.match(header, /`\[review: max\]`$/);
  assert.doesNotMatch(header, /medium|rounds/, "old tier + rounds replaced (rounds back to the tier default)");
  assert.match(header, /`\[kind: pr-review\]`/, "other tags kept");

  assert.equal((await control({ action: "review", phase: "2", tier: "huge" })).status, 400);
  assert.equal((await control({ action: "review", phase: "1", tier: "max" })).status, 400, "not a review phase");
  fs.writeFileSync(planFile, fs.readFileSync(planFile, "utf8").replace("`[status: todo]` `[kind: pr-review]`", "`[status: running]` `[kind: pr-review]`"));
  assert.equal((await control({ action: "review", phase: "2", tier: "standard" })).status, 409, "already started");
});

test("control model: accepts grok legs alongside codex|claude; rejects unknown engines", async (t) => {
  const fx = fixture();
  const daemon = await realDaemon(fx.store);
  t.after(async () => { await daemon.stop(); fs.rmSync(fx.root, { recursive: true, force: true }); });
  const control = (body) => fetch(`${daemon.url}/api/loops/loop-demo/control`, {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
  });
  assert.equal((await control({ action: "model", phase: "2", leg: "grok:default" })).status, 200);
  assert.equal(fs.readFileSync(path.join(fx.coord, ".loop", "control", "model-2"), "utf8").trim(), "grok:default");
  const bad = await control({ action: "model", phase: "2", leg: "gemini:pro" });
  assert.equal(bad.status, 400);
  assert.match((await bad.json()).error, /codex\|claude\|grok/);
});

test("tierChoices: standard and max show shared rounds, inner cap and provider rotation", () => {
  const conf = [
    "LOOP_REVIEW_ROUNDS_DEFAULT=4", "LOOP_REVIEW_INNER_DEFAULT=3", "LOOP_REVIEW_ROUNDS_MAX=5",
    'CHAIN_REVIEW_STANDARD_GPT=("codex:@sol" "claude:sonnet")',
    'CHAIN_REVIEW_STANDARD_CLAUDE=("claude:opus")', 'CHAIN_REVIEW_STANDARD_GROK=("grok:default")',
    'CHAIN_REVIEW_STANDARD_FIX=("claude:opus")', 'CHAIN_REVIEW_STANDARD_FINAL=("claude:opus")',
    'CHAIN_REVIEW_MAX_GPT=("codex:@astra")', 'CHAIN_REVIEW_MAX_CLAUDE=("claude:fable")',
    'CHAIN_REVIEW_MAX_GROK=("grok:default")',
    'CHAIN_REVIEW_MAX_FIX=("claude:opus")', 'CHAIN_REVIEW_MAX_FINAL=("claude:opus")',
  ].join("\n");
  assert.deepEqual(tierChoices(conf, "standard"), [
    "standard ×4 · inner 3 · R/C rotation codex:@sol → claude:opus → grok:default · fix claude:opus · final claude:opus  (current)",
    "max      ×4 · inner 3 · R/C rotation codex:@astra → claude:fable → grok:default · fix claude:opus · final claude:opus",
  ]);
});

test("tierChoices: missing knobs fall back to two rounds and two inner exchanges", () => {
  const lines = tierChoices("", "medium");
  assert.equal(lines.length, 2);
  assert.match(lines[0], /^standard ×2 · inner 2 · R\/C rotation /);
  assert.match(lines[1], /^max +×2 · inner 2 · R\/C rotation /);
  assert.ok(lines.every((line) => !line.includes("(current)")));
});

// ---- milestones: grouped rows, parked merge gate, current-milestone header ----

const MILESTONES = [{ number: 1, title: "Core" }, { number: 2, title: "Polish" }];
const milestoneSnapshot = ({ parked = true, milestones = MILESTONES, effort = { milestone: 1, milestoneTotal: 2, integrationBranch: "feat/m1" } } = {}) => ({
  effort,
  graph: {
    milestones,
    nodes: [
      node({ id: "1", phase: "1", title: "Work", ui: "done", milestone: 1 }),
      node({ id: "2", phase: "2", kind: "pr-review", gate: "merge", lane: "review", title: "Gate", milestone: 1,
        ui: parked ? "awaiting" : "todo", ...(parked ? { awaitingMerge: true, prs: { "acme/web": "https://github.com/acme/web/pull/7", "acme/api": "https://github.com/acme/api/pull/12" } } : {}) }),
      node({ id: "3", phase: "3", title: "Next", milestone: 2 }),
    ],
    edges: [{ source: "2", target: "3", kind: "depends", blocking: true }],
  },
});

test("phaseRows: a parked merge gate says awaiting merge (not waiting on the orchestrator) and carries its PRs", () => {
  const [, gate, next] = phaseRows(milestoneSnapshot(), null);
  assert.deepEqual([gate.icon, gate.state, gate.milestone], ["⏸", "awaiting merge", 1]);
  assert.deepEqual(gate.gate.prs, [{ slug: "acme/api", number: "#12" }, { slug: "acme/web", number: "#7" }]);
  assert.equal(gateText(gate.gate.prs), "acme/api #12 · acme/web #7 · /loop-execute resume");
  assert.equal(next.gate, null);
  assert.equal(phaseRows(milestoneSnapshot({ parked: false }), null)[1].gate, null);
});

test("milestoneTitles: only when the plan has more than one milestone", () => {
  assert.deepEqual([...milestoneTitles(milestoneSnapshot())], [[1, "Core"], [2, "Polish"]]);
  assert.equal(milestoneTitles(milestoneSnapshot({ milestones: [MILESTONES[0]] })).size, 0);
  assert.equal(milestoneTitles({ graph: { nodes: [] } }).size, 0);
});

test("--once: rows grouped under M<n> headers, gate row shows PRs + resume hint, header names the current milestone's branch", async (t) => {
  const snapshot = milestoneSnapshot({ effort: { milestone: 1, milestoneTotal: 2, integrationBranch: "feat/m1" } });
  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  const lines = out.stdout.split("\n");
  assert.match(lines[0], /^loop-demo · active · .* · M1\/2 · feat\/m1$/);
  const at = (re) => lines.findIndex((l) => re.test(l));
  assert.ok(at(/^ +M1 — Core$/) >= 0 && at(/^ +M1 — Core$/) < at(/ 1 +Work/));
  assert.ok(at(/ 2 +Gate/) < at(/^ +M2 — Polish$/) && at(/^ +M2 — Polish$/) < at(/ 3 +Next/));
  assert.match(out.stdout, /⏸ 2 +Gate .* awaiting merge\n +acme\/api #12 · acme\/web #7 · \/loop-execute resume\n/);
});

test("--once: header shows milestone 2's branch, never milestone 1's, once the loop moved on", async (t) => {
  const snapshot = milestoneSnapshot({ parked: false, effort: { milestone: 2, milestoneTotal: 2, integrationBranch: "feat/m2" } });
  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.match(out.stdout.split("\n")[0], / · M2\/2 · feat\/m2$/);
  const nullBranch = milestoneSnapshot({ parked: false, effort: { milestone: 2, milestoneTotal: 2, integrationBranch: null } });
  const fx2 = fixture({ snapshot: nullBranch });
  t.after(() => fs.rmSync(fx2.root, { recursive: true, force: true }));
  const out2 = await runCli(["--once", "loop-demo"], { cwd: fx2.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx2.store } });
  assert.doesNotMatch(out2.stdout.split("\n")[0], /feat\/demo/); // stored loop-level branch is milestone 1's
});

test("--once: a single-milestone plan renders unchanged (no headers, no M<n> in the header)", async (t) => {
  const snapshot = milestoneSnapshot({ parked: false, milestones: [MILESTONES[0]], effort: { milestone: 1, milestoneTotal: 1, integrationBranch: "feat/demo" } });
  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.match(out.stdout.split("\n")[0], /^loop-demo · active · .* · feat\/demo$/);
  assert.doesNotMatch(out.stdout, /M1 — |M\d\/\d/);
});

test("--once: an archived milestone stub renders its phases as done under an archived heading", async (t) => {
  const { parsePlan } = await import("../loop-web/src/model/parse-plan.ts");
  const { buildGraph } = await import("../loop-web/src/model/build-graph.ts");
  const text = fs.readFileSync(path.join(import.meta.dirname, "fixtures", "milestones", "archived-m1", "plan.md"), "utf8");
  const snapshot = { effort: { milestone: 2, milestoneTotal: 3, integrationBranch: "feat/m2-web" }, graph: buildGraph(parsePlan(text), null) };
  const fx = fixture({ snapshot });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store, COLUMNS: "160", LINES: "80" } });
  assert.equal(out.code, 0, out.stderr);
  assert.match(out.stdout, /^ +M1 — Core API and web shell ship · archived$/m);
  assert.match(out.stdout, /^ +M2 — The web shell calls the API$/m);
  for (const id of ["1", "2", "9", "3"]) assert.match(out.stdout, new RegExp(`✓ ${id} +\\(archived\\) .*done`));
  assert.match(out.stdout, / 4 +Call the API from the web shell /);
});

test("pickerRows: awaiting-merge sits after active, before finished; the runId stays the 2nd field", () => {
  const rows = pickerRows([
    { runId: "fin", status: "finished", updatedAt: "2026-09-04T00:00:00Z" },
    { runId: "park", status: "awaiting-merge", updatedAt: "2026-09-03T00:00:00Z" },
    { runId: "live", status: "active", updatedAt: "2026-09-01T00:00:00Z" },
  ]);
  assert.deepEqual(rows.map((r) => r.split(/\s+/)[1]), ["live", "park", "fin"]);
  assert.match(rows[1], /^awaiting-merge  park /);
});

// A late phase 5 inserted into milestone 1 (stable ids): plan order 1, 5, 2 | 3, 4.
const lateSnapshot = () => ({
  effort: { milestone: 1, milestoneTotal: 2, integrationBranch: "feat/m1" },
  graph: {
    milestones: MILESTONES,
    nodes: [
      node({ id: "1", phase: "1", title: "Work", ui: "done", milestone: 1 }),
      node({ id: "5", phase: "5", title: "Late", ui: "done", milestone: 1 }),
      node({ id: "2", phase: "2", kind: "pr-review", gate: "merge", lane: "review", title: "Gate", ui: "done", milestone: 1 }),
      node({ id: "3", phase: "3", title: "Next", milestone: 2 }),
      node({ id: "4", phase: "4", title: "Last", milestone: 2 }),
    ],
    edges: [
      { source: "1", target: "5", kind: "depends", blocking: false },
      { source: "5", target: "2", kind: "depends", blocking: false },
      { source: "2", target: "3", kind: "depends", blocking: false },
      { source: "3", target: "4", kind: "depends", blocking: true },
    ],
  },
});

test("phaseRows: multi-milestone plans keep document order per milestone (late phase 5 stays in M1)", () => {
  assert.deepEqual(phaseRows(lateSnapshot(), null).map((r) => r.label), ["1", "5", "2", "3", "4"]);
});

for (const mode of [[], ["--graph"]]) {
  test(`--once ${mode.join(" ")}: late phase 5 in M1 → each milestone heading once, in order`, async (t) => {
    const fx = fixture({ snapshot: lateSnapshot() });
    t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
    const out = await runCli(["--once", ...mode, "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
    const lines = out.stdout.split("\n");
    const at = (re) => lines.findIndex((l) => re.test(l));
    assert.equal(lines.filter((l) => /M\d — /.test(l)).length, 2);
    assert.ok(at(/M1 — Core$/) < at(/ 5 +Late/) && at(/ 5 +Late/) < at(/ 2 +Gate/) && at(/ 2 +Gate/) < at(/M2 — Polish$/) && at(/M2 — Polish$/) < at(/ 3 +Next/));
  });
}

test("--once --graph: milestone headings keep the rail continuous", async (t) => {
  const fx = fixture({ snapshot: lateSnapshot() });
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const out = await runCli(["--once", "--graph", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.match(out.stdout, /\n +│ +M2 — Polish\n/);
});

test("fitRows: headings never count as hidden phases; a fully hidden milestone drops its heading", () => {
  const g = (text, over = {}) => ({ done: false, selected: false, lines: [text], ...over });
  const groups = [
    g("M1", { heading: true, milestone: 1 }), g("r1", { done: true, milestone: 1 }), g("r2", { done: true, milestone: 1 }),
    g("M2", { heading: true, milestone: 2 }), g("r3", { selected: true, milestone: 2 }), g("r4", { milestone: 2 }),
  ];
  assert.deepEqual(fitRows(groups, 4, (_s, x) => x), ["  ✓ 2 earlier done phase(s) hidden", "M2", "r3", "r4"]);
});


test("loop-top --once: shows quota windows from ws with source and age", async (t) => {
  const fx = fixture();
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  const windows = [
    { runner: "codex", account_id: "account-a", bucket: "premium", window: "five_hour", remaining_ratio: 0.75, source: "reported", age_secs: 42, status: "available" },
    { runner: "claude", account_id: "account-b", bucket: null, window: "weekly", remaining_ratio: null, source: "estimated", age_secs: 120, status: "unknown" },
    { runner: "grok", account_id: null, bucket: null, window: "unknown", remaining_ratio: null, source: null, age_secs: null, status: "unavailable" },
    { runner: "codex", account_id: "account-a", window: "expired", remaining_ratio: 1, source: "reported", age_secs: 60, status: "unavailable" },
  ];
  fs.writeFileSync(path.join(fx.root, "quota.json"), JSON.stringify({ windows }));
  fs.writeFileSync(path.join(fx.root, "bin", "ws"), `#!/bin/sh
[ "$1" = usage ] && [ "$2" = --json ] || exit 2
[ "$WS_SOCK" = "$HOME/store/wsd.sock" ] || exit 3
/bin/cat "$HOME/quota.json"
`, { mode: 0o755 });
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.equal(out.code, 0, out.stderr);
  assert.match(out.stdout, /Quota/);
  assert.match(out.stdout, /codex.*account-a.*premium.*five_hour.*75\.0%.*reported.*42s ago/);
  assert.match(out.stdout, /claude.*weekly.*unknown.*estimated.*2m ago/);
  assert.match(out.stdout, /grok.*unavailable/);
  assert.match(out.stdout, /expired.*unavailable/);
  assert.doesNotMatch(out.stdout, /100\.0%/);
});

for (const [name, script] of [
  ["missing executable", null],
  ["command failure", "#!/bin/sh\nexit 1\n"],
  ["unknown subcommand", "#!/bin/sh\necho 'unknown subcommand: usage' >&2\nexit 2\n"],
  ["invalid JSON", "#!/bin/sh\necho unavailable\n"],
  ["invalid response", "#!/bin/sh\necho '{}'\n"],
  ["invalid window", '#!/bin/sh\necho \'{"windows":[null]}\'\n'],
]) {
  test(`loop-top --once: omits quota after ${name}`, async (t) => {
    const fx = fixture();
    t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
    if (script) fs.writeFileSync(path.join(fx.root, "bin", "ws"), script, { mode: 0o755 });
    else fs.rmSync(path.join(fx.root, "bin", "ws"));
    const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: {
      LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store,
      ...(script ? {} : { PATH: path.join(fx.root, "bin") }),
    } });
    assert.equal(out.code, 0, out.stderr);
    assert.match(out.stdout, /Stored title/);
    assert.doesNotMatch(out.stdout, /Quota/);
  });
}

test("loop-top --once: terminates an unresponsive quota command", { timeout: 30_000 }, async (t) => {
  const fx = fixture();
  t.after(() => fs.rmSync(fx.root, { recursive: true, force: true }));
  fs.writeFileSync(path.join(fx.root, "bin", "ws"), `#!${process.execPath}
require("node:fs").writeFileSync(process.env.HOME + "/quota-started", String(process.pid));
setInterval(() => {}, 60_000);
`, { mode: 0o755 });
  const out = await runCli(["--once", "loop-demo"], { cwd: fx.root, env: { LOOP_DAEMON_URL: "http://127.0.0.1:9", LOOP_STORE_DIR: fx.store } });
  assert.equal(out.code, 0, out.stderr);
  assert.doesNotMatch(out.stdout, /Quota/);
  const pid = Number(fs.readFileSync(path.join(fx.root, "quota-started"), "utf8"));
  assert.throws(() => process.kill(pid, 0), { code: "ESRCH" });
});
