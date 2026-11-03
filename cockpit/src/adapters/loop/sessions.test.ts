import { afterEach, expect, it } from "vitest";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { discoverLoopDirectories, matchLoopSession } from "./sessions.ts";
import type { LoopState } from "./index.ts";

const temporary: string[] = [];

afterEach(() => {
  for (const dir of temporary.splice(0)) rmSync(dir, { recursive: true, force: true });
});

it("maps the runner cwd to the phase worktree in state", () => {
  const state: LoopState = {
    runId: "loop-checkout-revamp-2026-08-03",
    phases: { "3": { lane: "A", worktree: "/Users/dev/.loop/worktrees/shop-feat-wire-checkout" } },
  };
  expect(matchLoopSession(state, "/Users/dev/.loop/worktrees/shop-feat-wire-checkout/"))
    .toEqual({ runId: "loop-checkout-revamp-2026-08-03", phaseId: "3", laneId: "A" });
  expect(matchLoopSession(state, "/Users/dev/.loop/worktrees/shop-feat-wire-checkout/src"))
    .toEqual({ runId: "loop-checkout-revamp-2026-08-03", phaseId: "3", laneId: "A" });
  expect(matchLoopSession(state, "/Users/dev/.loop/worktrees/shop-feat-wire-checkout-other")).toBeUndefined();
  expect(matchLoopSession(state, "/Users/dev/.loop/worktrees/shop-integration")).toBeUndefined();
  expect(matchLoopSession({}, "/tmp/example")).toBeUndefined();
});

it("discovers an ancestor Loop folder and the source checkout of a linked worktree", () => {
  const root = mkdtempSync(join(tmpdir(), "cockpit-discovery-"));
  temporary.push(root);
  const source = join(root, "source");
  const lane = join(root, "lane");
  const gitdir = join(source, ".git", "worktrees", "lane");
  mkdirSync(gitdir, { recursive: true });
  mkdirSync(join(source, ".loop"));
  writeFileSync(join(source, ".loop", "plan.md"), "# Test plan");
  mkdirSync(join(lane, "src"), { recursive: true });
  writeFileSync(join(lane, ".git"), `gitdir: ${gitdir}\n`);
  writeFileSync(join(gitdir, "commondir"), "../..\n");
  expect(discoverLoopDirectories(join(lane, "src"))).toEqual([join(source, ".loop")]);
  expect(discoverLoopDirectories(source)).toEqual([join(source, ".loop")]);
  expect(discoverLoopDirectories(join(root, "unrelated"))).toEqual([]);
});
