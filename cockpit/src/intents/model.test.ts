import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test } from "vitest";
import { callClaude } from "./model.ts";

const homes: string[] = [];
const originalPath = process.env.PATH;
afterEach(async () => {
  process.env.PATH = originalPath;
  await Promise.all(homes.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

test("calls the selected CLI model without tools and sends the prompt through stdin", async () => {
  const home = await mkdtemp(join(tmpdir(), "cockpit-model-"));
  homes.push(home);
  const bin = join(home, "bin");
  await mkdir(bin);
  const received = join(home, "received.json");
  await writeFile(join(bin, "claude"), `#!${process.execPath}
import { writeFileSync } from 'node:fs';
let input = '';
for await (const chunk of process.stdin) input += chunk;
writeFileSync(${JSON.stringify(received)}, JSON.stringify({ args: process.argv.slice(2), input }));
process.stdout.write('{"text":"Keep session history"}');
`, { mode: 0o700 });
  process.env.PATH = `${bin}:${originalPath}`;
  const prompt = 'Name this goal. $(touch forbidden) `touch forbidden`';
  for (const model of ["haiku", "sonnet"] as const) {
    expect(await callClaude(model, prompt)).toBe('{"text":"Keep session history"}');
    expect(JSON.parse(await readFile(received, "utf8"))).toEqual({
      args: ["-p", "--model", model, "--output-format", "text", "--tools", "", "--disable-slash-commands", "--strict-mcp-config",
        "--setting-sources", "", "--no-session-persistence"], input: prompt,
    });
  }
}, 30_000);

test("rejects a failed CLI process so the processor can use rule names", async () => {
  const home = await mkdtemp(join(tmpdir(), "cockpit-model-failure-"));
  homes.push(home);
  await writeFile(join(home, "claude"), `#!${process.execPath}\nprocess.exit(1);\n`, { mode: 0o700 });
  process.env.PATH = `${home}:${originalPath}`;
  await expect(callClaude("haiku", "Name this goal.")).rejects.toThrow("The naming model failed.");
}, 30_000);
