import { appendFile, mkdir, mkdtemp, readFile, rm, symlink, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test } from "vitest";
import { EventStore } from "../../core/store.ts";
import { SessionReader } from "./reader.ts";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

test("session polling discovers both formats and scratch media without repeating events after restart", async () => {
  const root = await mkdtemp(join(tmpdir(), "cockpit-sessions-"));
  roots.push(root);
  const home = join(root, "observer");
  let now = Date.parse("2026-11-01T12:01:05Z");
  const claudePath = join(root, ".claude/projects/example/session.jsonl");
  const codexPath = join(root, ".codex/sessions/2026/11/01/rollout.jsonl");
  for (const [path, fixture] of [[claudePath, "claude"], [codexPath, "codex"]]) {
    await mkdir(join(path, ".."), { recursive: true });
    const text = (await readFile(new URL(`../../../fixtures/${fixture}-session.jsonl`, import.meta.url), "utf8")).replaceAll("/home/example", root);
    await writeFile(path, text);
    await utimes(path, now / 1000, now / 1000);
  }
  const scratch = join(root, ".beluga/chats/chat-demo/.beluga/scratch");
  await mkdir(join(scratch, "nested"), { recursive: true });
  await writeFile(join(scratch, "example.png"), "synthetic image");
  await writeFile(join(scratch, "nested/flow.gif"), "synthetic animation");
  await writeFile(join(scratch, "notes.txt"), "Not media.");
  await writeFile(join(root, "outside.png"), "Must not appear.");
  await symlink(join(root, "outside.png"), join(scratch, "linked.png"));
  const original = await readFile(claudePath, "utf8");
  const reader = await SessionReader.open({ home, userHome: root, now: () => now });
  const store = await EventStore.open({ home, now: () => now });
  const events = await reader.poll();
  for (const event of events) await store.append(event);
  await reader.save();
  expect(store.snapshot().map((run) => run.meta.source).sort()).toEqual(["claude", "codex"]);
  expect(store.snapshot().every((run) => run.meta.chatId === "chat-demo")).toBe(true);
  expect(events.filter((event) => event.type === "media").map((event) => event.media.mimeType).sort()).toEqual(["image/gif", "image/gif", "image/png", "image/png"]);
  const restarted = await SessionReader.open({ home, userHome: root, now: () => now });
  expect(await restarted.poll()).toEqual([]);
  now += 120_000;
  const idle = await restarted.poll();
  expect(idle).toHaveLength(2);
  expect(idle.every((event) => event.type === "run.meta" && event.meta.status === "idle")).toBe(true);
  await appendFile(claudePath, JSON.stringify({ type: "user", timestamp: new Date(now).toISOString(), message: { content: [{ type: "tool_result", tool_use_id: "tool-ask", content: "Small" }] } }) + "\n");
  await utimes(claudePath, now / 1000, now / 1000);
  const resumed = await restarted.poll();
  expect(resumed.find((event) => event.type === "ask")).toMatchObject({ ask: { id: "tool-ask", resolved: true } });
  expect(resumed.filter((event) => event.type === "action")).toEqual([]);
  expect(await readFile(claudePath, "utf8")).toBe(original + JSON.stringify({ type: "user", timestamp: new Date(now).toISOString(), message: { content: [{ type: "tool_result", tool_use_id: "tool-ask", content: "Small" }] } }) + "\n");
});

test("missing session roots produce no events", async () => {
  const root = await mkdtemp(join(tmpdir(), "cockpit-sessions-"));
  roots.push(root);
  const reader = await SessionReader.open({ home: join(root, "observer"), userHome: root });
  expect(await reader.poll()).toEqual([]);
  await reader.save();
  expect(await reader.poll()).toEqual([]);
});
