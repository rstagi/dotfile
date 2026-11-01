import { appendFile, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test } from "vitest";
import { JsonlTailer } from "./tailer.ts";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

test("stored byte offsets skip old lines across passes and restarts", async () => {
  const root = await mkdtemp(join(tmpdir(), "cockpit-tail-"));
  roots.push(root);
  const source = join(root, "session.jsonl");
  await writeFile(source, '{"text":"café"}\n');
  const tailer = await JsonlTailer.open(join(root, "observer"));
  expect((await tailer.read(source))?.lines).toEqual([{ offset: 0, text: '{"text":"café"}' }]);
  expect((await tailer.read(source))?.lines).toEqual([]);
  await tailer.save();
  const restarted = await JsonlTailer.open(join(root, "observer"));
  expect((await restarted.read(source))?.lines).toEqual([]);
  await appendFile(source, '{"next":true}\n');
  expect((await restarted.read(source))?.lines).toEqual([{ offset: 17, text: '{"next":true}' }]);
  expect(await readFile(source, "utf8")).toBe('{"text":"café"}\n{"next":true}\n');
});

test("an incomplete UTF-8 line waits for LF and a truncated file starts at zero", async () => {
  const root = await mkdtemp(join(tmpdir(), "cockpit-tail-"));
  roots.push(root);
  const source = join(root, "session.jsonl");
  const tailer = await JsonlTailer.open(join(root, "observer"));
  const line = Buffer.from('{"text":"é"}\n');
  await writeFile(source, line.subarray(0, 10));
  expect((await tailer.read(source))?.lines).toEqual([]);
  await tailer.save();
  const restarted = await JsonlTailer.open(join(root, "observer"));
  await appendFile(source, line.subarray(10));
  expect((await restarted.read(source))?.lines).toEqual([{ offset: 0, text: '{"text":"é"}' }]);
  await writeFile(source, '{}\n');
  expect(await restarted.read(source)).toMatchObject({ lines: [{ offset: 0, text: '{}' }], reset: true });
  expect(await restarted.read(join(root, "missing.jsonl"))).toBeUndefined();
});
