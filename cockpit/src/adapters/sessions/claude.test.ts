import { readFile } from "node:fs/promises";
import { expect, test } from "vitest";
import { adaptClaude } from "./claude.ts";
import type { JsonlLine } from "./tailer.ts";

const context = {
  path: "/home/example/.claude/projects/example/session.jsonl", userHome: "/home/example",
  modifiedAt: Date.parse("2026-11-01T12:00:03Z"), now: Date.parse("2026-11-01T12:00:04Z"),
};

test("Claude fixture emits session metadata, tool actions and an unanswered question", async () => {
  const lines = fixtureLines(await readFile(new URL("../../../fixtures/claude-session.jsonl", import.meta.url), "utf8"));
  const result = adaptClaude(lines, context);
  expect(result.state.meta).toEqual({
    source: "claude", title: "Inspect the example source", status: "active",
    sessionId: "claude-example", cwd: "/home/example/.beluga/chats/chat-demo/project",
    branch: "feat/example", isSidechain: true, chatId: "chat-demo",
  });
  expect(result.events.filter((event) => event.type === "run.meta").at(-1)).toMatchObject({
    meta: { title: "Inspect the example source", sessionId: "claude-example", chatId: "chat-demo" },
  });
  expect(result.events.find((event) => event.type === "action")).toMatchObject({
    timestamp: "2026-11-01T12:00:01Z", sessionId: "claude-example",
    action: { id: "tool-read", tool: "Read", summary: "src/example.ts", sessionId: "claude-example", timestamp: "2026-11-01T12:00:01Z" },
  });
  expect(result.events.find((event) => event.type === "ask")).toMatchObject({
    ask: { id: "tool-ask", text: "Which example should I use?", resolved: false, sessionId: "claude-example" },
  });
  expect(adaptClaude(lines, context)).toEqual(result);
});

function fixtureLines(text: string): JsonlLine[] {
  let offset = 0;
  return text.trimEnd().split("\n").map((text) => {
    const line = { text, offset };
    offset += Buffer.byteLength(text) + 1;
    return line;
  });
}
