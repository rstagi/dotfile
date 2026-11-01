import type { JsonlLine } from "./tailer.ts";
import type { SessionContext, SessionResult, SessionSnapshot } from "./session.ts";
import { asObject, parseLine, SessionBuilder, string } from "./session.ts";

export function adaptClaude(lines: JsonlLine[], context: SessionContext, previous?: SessionSnapshot): SessionResult {
  const builder = new SessionBuilder("claude", context, previous);
  for (const line of lines) {
    const row = parseLine(line);
    if (!row) continue;
    builder.line(line, row);
    builder.metadata({
      sessionId: string(row.sessionId), cwd: string(row.cwd), branch: string(row.gitBranch),
      isSidechain: typeof row.isSidechain === "boolean" ? row.isSidechain : undefined,
      title: string(row.aiTitle) ?? string(row["ai-title"]),
    });
    const message = asObject(row.message);
    const content = message.content;
    if (row.type === "user") {
      const prompt = typeof content === "string" ? content : Array.isArray(content)
        ? content.map((part) => string(asObject(part).text)).filter(Boolean).join("\n") : "";
      builder.userTurn(prompt);
    }
    if (!Array.isArray(content)) continue;
    const text: string[] = [];
    let hasTools = false;
    for (const [index, value] of content.entries()) {
      const block = asObject(value);
      if (row.type === "assistant" && block.type === "tool_use" && typeof block.name === "string") {
        hasTools = true;
        builder.tool(string(block.id) ?? `tool:${line.offset}:${index}`, block.name, block.input);
      } else if (row.type === "user" && block.type === "tool_result" && typeof block.tool_use_id === "string") {
        builder.answer(block.tool_use_id);
      } else if (block.type === "text" && typeof block.text === "string") {
        text.push(block.text);
      }
    }
    if (row.type === "assistant" && !hasTools) builder.finalMessage(text.join("\n"));
  }
  return builder.finish();
}
