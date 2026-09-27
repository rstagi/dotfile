#!/usr/bin/env node

import fs from "node:fs";

fs.appendFileSync(process.env.FAKE_JEV_LOG, `${await readStdin()}\n`);
process.stdout.write(`${process.env.FAKE_JEV_RESPONSE}\n`);

async function readStdin() {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  return Buffer.concat(chunks).toString("utf8").trim();
}
