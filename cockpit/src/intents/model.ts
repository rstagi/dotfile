import { spawn } from "node:child_process";

export type NamingModel = "haiku" | "sonnet";
export type ModelCaller = (model: NamingModel, prompt: string) => Promise<string>;
const MAX_OUTPUT_BYTES = 1024 * 1024;

// The prompt uses stdin. No transcript text enters a shell command.
export const callClaude: ModelCaller = (model, prompt) => new Promise((resolve, reject) => {
  const child = spawn("claude", ["-p", "--model", model, "--output-format", "text",
    "--tools", "", "--disable-slash-commands", "--strict-mcp-config",
    "--setting-sources", "", "--no-session-persistence"], { stdio: ["pipe", "pipe", "ignore"] });
  let output = "";
  const timer = setTimeout(() => {
    child.kill("SIGKILL");
    reject(new Error("The naming model exceeded its time limit."));
  }, 60_000);
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    output += chunk;
    if (Buffer.byteLength(output) > MAX_OUTPUT_BYTES) {
      child.kill("SIGKILL");
      reject(new Error("The naming model exceeded its output limit."));
    }
  });
  child.on("error", (error) => { clearTimeout(timer); reject(error); });
  child.on("close", (code) => {
    clearTimeout(timer);
    if (code === 0) resolve(output.trim());
    else reject(new Error("The naming model failed."));
  });
  child.stdin.on("error", () => { /* The close event reports an early process exit. */ });
  child.stdin.end(prompt);
});
