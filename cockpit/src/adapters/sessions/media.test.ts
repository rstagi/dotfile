import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test } from "vitest";
import { sessionMedia, writtenMediaPaths } from "./media.ts";
import type { SessionSnapshot } from "./session.ts";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

test("session-written images appear once, update after a write, and reject paths outside the cwd", async () => {
  const root = await mkdtemp(join(tmpdir(), "cockpit-media-"));
  roots.push(root);
  const cwd = join(root, "project");
  await mkdir(cwd);
  const image = join(cwd, "example.webp");
  await writeFile(image, "Synthetic image.");
  await writeFile(join(root, "outside.png"), "Must not appear.");
  const session: SessionSnapshot = { runId: "example", meta: { cwd, sessionId: "example-session" }, pendingAsks: {} };
  const paths = writtenMediaPaths([
    { id: "write", runId: "example", timestamp: "2026-11-01T12:00:00Z", type: "action", action: { id: "write", tool: "Write", summary: "example.webp", input: { file_path: "example.webp" } } },
    { id: "patch", runId: "example", timestamp: "2026-11-01T12:00:00Z", type: "action", action: { id: "patch", tool: "apply_patch", summary: "Add an image", input: "*** Begin Patch\n*** Add File: ../outside.png\n*** End Patch" } },
  ], cwd);
  const first = await sessionMedia(session, root, paths, {});
  expect(first.events).toMatchObject([{ type: "media", media: { mimeType: "image/webp", title: "example.webp" } }]);
  expect(first.events).toHaveLength(1);
  expect((await sessionMedia(session, root, paths, first.signatures)).events).toEqual([]);
  await writeFile(image, "Updated synthetic image with a different size.");
  const updated = await sessionMedia(session, root, paths, first.signatures);
  expect(updated.events).toHaveLength(1);
  expect(updated.events[0].id).not.toBe(first.events[0].id);
});

test("a scratch directory symlink cannot expose another directory", async () => {
  const root = await mkdtemp(join(tmpdir(), "cockpit-media-"));
  roots.push(root);
  const chat = join(root, ".beluga/chats/chat-example");
  await mkdir(join(chat, ".beluga"), { recursive: true });
  const outside = join(root, "outside");
  await mkdir(outside);
  await writeFile(join(outside, "private.png"), "Must not appear.");
  await symlink(outside, join(chat, ".beluga/scratch"));
  expect((await sessionMedia({ runId: "example", meta: { cwd: chat }, pendingAsks: {} }, root, [], {})).events).toEqual([]);
});
