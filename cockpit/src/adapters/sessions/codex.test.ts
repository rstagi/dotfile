import { readFile } from "node:fs/promises";
import { expect, test } from "vitest";
import { adaptCodex } from "./codex.ts";

const context = {
  path: "/home/example/.codex/sessions/2026/11/01/rollout-example.jsonl", userHome: "/home/example",
  modifiedAt: Date.parse("2026-11-01T12:01:03Z"), now: Date.parse("2026-11-01T12:01:04Z"),
};

test("Codex fixture emits metadata, custom and function tools, and a final question", async () => {
  const text = await readFile(new URL("../../../fixtures/codex-session.jsonl", import.meta.url), "utf8");
  let offset = 0;
  const lines = text.trimEnd().split("\n").map((text) => {
    const line = { text, offset };
    offset += Buffer.byteLength(text) + 1;
    return line;
  });
  const result = adaptCodex(lines, context);
  expect(result.state.meta).toEqual({
    source: "codex", title: "Update the example", status: "active", sessionId: "codex-example",
    cwd: "/home/example/.beluga/chats/chat-demo/project", branch: "feat/example", isSidechain: false, chatId: "chat-demo",
  });
  expect(result.events.filter((event) => event.type === "action")).toMatchObject([
    { timestamp: "2026-11-01T12:01:01Z", action: { id: "codex-patch", tool: "apply_patch", summary: "*** Begin Patch *** Add File: example.txt +Example *** End Patch", sessionId: "codex-example" } },
    { timestamp: "2026-11-01T12:01:02Z", action: { id: "codex-shell", tool: "exec_command", summary: "cat example.txt", input: { cmd: "cat example.txt" } } },
  ]);
  expect(result.events.find((event) => event.type === "ask")).toMatchObject({ ask: { text: "Should I use the small example?", resolved: false } });
});

test("the first user prompt supplies a title when session metadata has none", () => {
  const result = adaptCodex([
    { offset: 0, text: JSON.stringify({ type: "session_meta", payload: { id: "child-example", cwd: "/tmp/example", source: { subagent: { thread_spawn: { parent_thread_id: "parent-example" } } } } }) },
    { offset: 100, text: JSON.stringify({ type: "response_item", payload: { type: "message", role: "user", content: [{ type: "input_text", text: "Update the example parser\nUse the fixture." }] } }) },
  ], context);
  expect(result.state.meta).toMatchObject({ title: "Update the example parser", isSidechain: true, sessionId: "child-example" });
});
