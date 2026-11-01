import { expect, test } from "vitest";
import { adaptClaude } from "./claude.ts";
import { adaptCodex } from "./codex.ts";
import { belugaChatId, IDLE_MS } from "./session.ts";

const now = Date.parse("2026-11-01T12:00:00Z");
const context = { path: "/tmp/example.jsonl", userHome: "/home/example", modifiedAt: now, now };

function line(value: unknown, offset = 0) {
  return { offset, text: JSON.stringify(value) };
}

test("session status changes at two minutes without writes and returns active after a write", () => {
  const first = adaptClaude([line({ type: "user", sessionId: "example", cwd: "/tmp/example" })], context);
  const before = adaptClaude([], { ...context, now: now + IDLE_MS - 1 }, first.state);
  expect(before.events).toEqual([]);
  const idle = adaptClaude([], { ...context, now: now + IDLE_MS }, first.state);
  expect(idle.events).toMatchObject([{ type: "run.meta", meta: { status: "idle" } }]);
  expect(adaptClaude([], { ...context, now: now + IDLE_MS + 1 }, idle.state).events).toEqual([]);
  expect(adaptClaude([], { ...context, modifiedAt: now + IDLE_MS, now: now + IDLE_MS }, idle.state).state.meta.status).toBe("active");
});

test("a tool result resolves only its matching Claude question across passes", () => {
  const pending = adaptClaude([line({ type: "assistant", sessionId: "example", message: { content: [
    { type: "tool_use", id: "ask-1", name: "AskUserQuestion", input: { questions: [{ question: "Which file?" }] } },
    { type: "tool_use", id: "ask-2", name: "AskUserQuestion", input: { questions: [{ question: "Which branch?" }] } },
  ] } })], context);
  const answered = adaptClaude([line({ type: "user", message: { content: [
    { type: "tool_result", tool_use_id: "ask-1", content: "example.ts" },
  ] } }, 100)], context, pending.state);
  expect(answered.events.filter((event) => event.type === "ask")).toMatchObject([{ ask: { id: "ask-1", resolved: true, text: "Which file?" } }]);
  expect(Object.keys(answered.state.pendingAsks)).toEqual(["ask-2"]);
});

test("a final question resolves on the next Codex user turn", () => {
  const pending = adaptCodex([line({ type: "response_item", payload: { type: "message", role: "assistant", phase: "final_answer", content: [{ type: "output_text", text: "Which file?" }] } })], context);
  const answered = adaptCodex([line({ type: "response_item", payload: { type: "message", role: "user", content: [{ type: "input_text", text: "Use example.ts." }] } }, 100)], context, pending.state);
  expect(Object.keys(answered.state.pendingAsks)).toEqual([]);
  expect(answered.events.find((event) => event.type === "ask")).toMatchObject({ ask: { resolved: true } });
});

test("Beluga chat IDs require a cwd inside this user's chat root", () => {
  expect(belugaChatId("/home/example/.beluga/chats/chat-demo", "/home/example")).toBe("chat-demo");
  expect(belugaChatId("/home/example/.beluga/chats/chat-demo/nested/project", "/home/example")).toBe("chat-demo");
  for (const cwd of ["/home/example/.beluga/chats", "/home/example/.beluga/chats-else/chat-demo", "/other/.beluga/chats/chat-demo", "/home/example/.beluga/chats/../outside", ".beluga/chats/chat-demo"]) {
    expect(belugaChatId(cwd, "/home/example")).toBeUndefined();
  }
});

test("malformed and unrelated records do not emit actions", () => {
  for (const adapt of [adaptClaude, adaptCodex]) {
    const result = adapt([{ offset: 0, text: "invalid JSON" }, line(null, 100), line({ type: "unknown" }, 200)], context);
    expect(result.events.filter((event) => event.type === "action")).toEqual([]);
  }
});

test("metadata events have distinct IDs when one line changes metadata and the title", () => {
  const result = adaptClaude([line({ type: "user", sessionId: "example", cwd: "/tmp/example", message: { content: "Inspect the example" } })], context);
  expect(new Set(result.events.map((event) => event.id)).size).toBe(result.events.length);
});
