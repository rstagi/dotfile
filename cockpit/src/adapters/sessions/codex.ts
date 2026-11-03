import type { JsonlLine } from "./tailer.ts";
import type { SessionContext, SessionResult, SessionSnapshot } from "./session.ts";
import { asObject, parseInput, parseLine, SessionBuilder, string } from "./session.ts";

export function adaptCodex(lines: JsonlLine[], context: SessionContext, previous?: SessionSnapshot): SessionResult {
  const builder = new SessionBuilder("codex", context, previous);
  for (const line of lines) {
    const row = parseLine(line);
    if (!row) continue;
    builder.line(line, row);
    const payload = asObject(row.payload);
    if (row.type === "session_meta") {
      const source = asObject(payload.source);
      builder.metadata({
        sessionId: string(payload.id), cwd: string(payload.cwd), branch: string(asObject(payload.git).branch),
        title: string(payload.title), isSidechain: payload.source === "subagent" || !!source.subagent,
      });
    } else if (row.type === "turn_context") {
      builder.metadata({ cwd: string(payload.cwd), branch: string(asObject(payload.git).branch) });
    } else if (row.type === "event_msg" && payload.type === "thread_name_updated") {
      builder.metadata({ title: string(payload.thread_name) ?? string(payload.title) });
    } else if (row.type === "response_item") {
      if ((payload.type === "custom_tool_call" || payload.type === "function_call") && typeof payload.name === "string") {
        const input = payload.type === "custom_tool_call" ? payload.input : parseInput(payload.arguments);
        builder.tool(string(payload.call_id) ?? `tool:${line.offset}`, payload.name, input);
      } else if (payload.type === "function_call_output" || payload.type === "custom_tool_call_output") {
        if (typeof payload.call_id === "string") builder.answer(payload.call_id);
      } else if (payload.type === "message") {
        const content = Array.isArray(payload.content) ? payload.content : [];
        const text = content.map((part) => string(asObject(part).text)).filter(Boolean).join("\n");
        if (payload.role === "user") builder.userTurn(text);
        if (payload.role === "assistant" && (!payload.phase || payload.phase === "final_answer")) {
          builder.finalMessage(text);
        }
      }
    }
  }
  return builder.finish();
}
