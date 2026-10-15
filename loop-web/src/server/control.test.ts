import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { spawn } from "node:child_process";
import type { ChildProcess } from "node:child_process";
import { once } from "node:events";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { Snapshot } from "../model/types.ts";

const SERVER = fileURLToPath(new URL("../../server/index.mjs", import.meta.url));
const PLAN = [
  "# Review control", "", "## Phases", "",
  "### Phase 1 — Build `[lane: A]` `[status: done]`", "",
  "### Phase 2 — Review `[lane: review]` `[status: todo]` `[kind: pr-review]` `[review: medium]` `[rounds: 4]` `[inner: 5]`",
  "- **Depends on:** Phase 1",
  "- **Notes:** Keep [rounds: 4] and [inner: 5] in prose.", "",
].join("\n");

let root: string;
let planFile: string;
let url: string;
let child: ChildProcess | undefined;

describe("review tier control endpoint", () => {
  beforeEach(async () => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "loop-review-control-"));
    planFile = path.join(root, "plan.md");
    fs.writeFileSync(planFile, PLAN);
    fs.writeFileSync(path.join(root, "state.json"), JSON.stringify({ runId: "tier-test" }));
    const port = await freePort();
    url = `http://127.0.0.1:${port}/api/loops/tier-test/control`;
    const server = spawn(process.execPath, [SERVER, "--dir", root, "--port", String(port)], {
      env: { PATH: process.env.PATH }, stdio: ["ignore", "pipe", "pipe"],
    });
    child = server;
    await new Promise<void>((resolve, reject) => {
      let output = "";
      const timer = setTimeout(() => reject(new Error(`Server did not start: ${output}`)), 5000);
      server.once("error", (error) => { clearTimeout(timer); reject(error); });
      server.once("exit", (code) => { clearTimeout(timer); reject(new Error(`Server exited ${code}: ${output}`)); });
      server.stderr.on("data", (data) => { output += data; });
      server.stdout.on("data", (data) => {
        output += data;
        if (output.includes("Loop Observatory (legacy) on")) { clearTimeout(timer); resolve(); }
      });
    });
  });

  afterEach(async () => {
    if (child && child.exitCode === null && child.signalCode === null) {
      const exited = once(child, "exit");
      child.kill();
      await exited;
    }
    child = undefined;
    fs.rmSync(root, { recursive: true, force: true });
  });

  it("reads review ledgers into the snapshot and preserves the live final attempt", async () => {
    const runs = path.join(root, "runs");
    const final = path.join(runs, "review-p2-acme--api-a4");
    fs.mkdirSync(final, { recursive: true });
    fs.writeFileSync(path.join(final, "leg.json"), JSON.stringify({ engine: "claude", model: "opus" }));
    fs.writeFileSync(path.join(runs, "review-p2-acme--api.ledger.tsv"), [
      "a1\tround1\treview\tgpt\tAPPROVE\truns/review-p2-acme--api-a1",
      "a2\tround1\tcritic\tclaude\tAGREE\truns/review-p2-acme--api-a2",
      "a3\tround1\tconfirm\tgrok\tAGREE\truns/review-p2-acme--api-a3",
    ].join("\n"));
    fs.writeFileSync(path.join(runs, "review-p9-acme--other.ledger.tsv"), "a1\tround1\treview\tgpt\tNEEDS_CHANGES\truns/other\n");
    const response = await control({ action: "review", phase: "2", tier: "standard" });
    expect(response.status).toBe(200);
    const snapshot = await (await fetch(url.replace(/control$/, "snapshot"))).json() as Snapshot;
    const stages = snapshot.graph.nodes.find((n) => n.id === "2")!.reviewStages!;
    expect(stages.slice(0, 3).map((r) => [r.role, r.verdict, r.state])).toEqual([
      ["review", "APPROVE", "done"], ["critic", "AGREE", "done"], ["confirm", "AGREE", "done"],
    ]);
    expect(stages.at(-1)).toMatchObject({ k: 4, role: "final", state: "running", model: "opus" });
    expect(stages.filter((r) => r.stage === "round2").every((r) => r.state === "skipped")).toBe(true);
    expect(new Set(stages.map((r) => r.repository))).toEqual(new Set(["acme/api"]));
  });

  it("reads pinned review limits so a started pipeline renders its real rounds", async () => {
    const runs = path.join(root, "runs");
    fs.mkdirSync(runs, { recursive: true });
    fs.writeFileSync(path.join(runs, "review-p2-acme--api.ledger.config.json"), JSON.stringify({ tier: "standard", rounds: 1, inner: 1 }));
    expect((await control({ action: "review", phase: "2", tier: "standard" })).status).toBe(200);
    const snapshot = await (await fetch(url.replace(/control$/, "snapshot"))).json() as Snapshot;
    const stages = snapshot.graph.nodes.find((n) => n.id === "2")!.reviewStages!;
    expect(stages.map((r) => r.stage)).not.toContain("round2");
  });

  it.each(["shallow", "medium", "huge"])("rejects tier %s without editing the plan", async (tier) => {
    const response = await control({ action: "review", phase: "2", tier });
    expect(response.status).toBe(400);
    expect((await response.json()).error).toMatch(/standard\|max/);
    expect(fs.readFileSync(planFile, "utf8")).toBe(PLAN);
  });

  it.each([
    ["standard", true], ["max", true], ["standard", false], ["max", false],
  ])("sets %s and resets round caps (backticked tags: %s)", async (tier, backticked) => {
    const input = backticked ? PLAN : PLAN.replaceAll("`", "");
    fs.writeFileSync(planFile, input);
    const response = await control({ action: "review", phase: "2", tier });
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({ ok: true, action: "review", phase: "2", tier });
    const updated = fs.readFileSync(planFile, "utf8");
    expect(updated.slice(0, updated.indexOf("### Phase 2"))).toBe(input.slice(0, input.indexOf("### Phase 2")));
    const header = updated.split("\n").find((line) => line.startsWith("### Phase 2"))!;
    expect(header).toBe(backticked
      ? `### Phase 2 — Review \`[lane: review]\` \`[status: todo]\` \`[kind: pr-review]\` \`[review: ${tier}]\``
      : `### Phase 2 — Review [lane: review] [status: todo] [kind: pr-review] \`[review: ${tier}]\``);
    expect(updated.slice(updated.indexOf("\n", updated.indexOf("### Phase 2")))).toBe(
      "\n- **Depends on:** Phase 1\n- **Notes:** Keep [rounds: 4] and [inner: 5] in prose.\n",
    );
  });

  it.each([true, false])("refuses an already-started review (backticked tags: %s)", async (backticked) => {
    const started = PLAN.replace("[status: todo]", "[status: in-progress]");
    const input = backticked ? started : started.replaceAll("`", "");
    fs.writeFileSync(planFile, input);
    const response = await control({ action: "review", phase: "2", tier: "standard" });
    expect(response.status).toBe(409);
    expect(fs.readFileSync(planFile, "utf8")).toBe(input);
  });
});

function control(body: Record<string, unknown>) {
  return fetch(url, {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
  });
}

async function freePort(): Promise<number> {
  const server = net.createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("No TCP port allocated");
  await new Promise<void>((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  return address.port;
}
