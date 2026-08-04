// Per-loop persistent store: one JSON file per loop at `~/.kestral/loops/<runId>.json`,
// kept FOREVER (no eviction — only the per-loop events[] is capped, in store-serde). Writes
// are atomic (mktemp + renameSync in the same dir, mirroring loop-state.sh) and debounced
// per loop; `finish`/SIGTERM/SIGINT flush synchronously so nothing is lost on shutdown.
//
// Pure serialize/parse live in ../src/model/store-serde.ts (tested); this file is the thin
// I/O shell around them.

import fs from "node:fs";
import path from "node:path";
import { serializeRecord, parseStoreFile } from "../src/model/store-serde.ts";

const DEBOUNCE_MS = 500;

export function createStore(storeDir) {
  fs.mkdirSync(storeDir, { recursive: true });
  const resolvedDir = path.resolve(storeDir);
  const timers = new Map(); // runId → timeout
  const pending = new Map(); // runId → latest record awaiting write
  let tmpSeq = 0;

  /** Sanitize a runId to a filename-safe stem; reject anything that could escape the dir. */
  function fileFor(runId) {
    if (typeof runId !== "string") return null;
    const safe = runId.replace(/[^A-Za-z0-9._-]/g, "_");
    if (!safe || safe === "." || safe === "..") return null;
    const p = path.resolve(resolvedDir, `${safe}.json`);
    return isContained(p, resolvedDir) ? p : null;
  }

  function writeNow(record) {
    const dest = fileFor(record.runId);
    if (!dest) return;
    const tmp = path.join(resolvedDir, `.${path.basename(dest)}.${process.pid}.${tmpSeq++}.tmp`);
    try {
      fs.writeFileSync(tmp, serializeRecord(record));
      fs.renameSync(tmp, dest);
    } catch (err) {
      try {
        fs.rmSync(tmp, { force: true });
      } catch {
        /* ignore */
      }
      console.error(`[loop-web] store write failed for ${record.runId}: ${err?.message ?? err}`);
    }
  }

  /** Debounced write — coalesces a burst of ingests into one atomic write per loop. */
  function save(record) {
    pending.set(record.runId, record);
    if (timers.has(record.runId)) return;
    const t = setTimeout(() => {
      timers.delete(record.runId);
      const rec = pending.get(record.runId);
      pending.delete(record.runId);
      if (rec) writeNow(rec);
    }, DEBOUNCE_MS);
    t.unref?.();
    timers.set(record.runId, t);
  }

  /** Synchronous write NOW (loop finish) — cancels any pending debounce for this loop. */
  function flush(record) {
    const t = timers.get(record.runId);
    if (t) {
      clearTimeout(t);
      timers.delete(record.runId);
    }
    pending.delete(record.runId);
    writeNow(record);
  }

  /** Flush every pending write synchronously (SIGTERM/SIGINT before exit). */
  function flushAll() {
    for (const rec of pending.values()) writeNow(rec);
    pending.clear();
    for (const t of timers.values()) clearTimeout(t);
    timers.clear();
  }

  /** Load every stored loop at startup. A corrupt file is skipped + warned, never fatal. */
  function loadAll() {
    const out = new Map();
    let names = [];
    try {
      names = fs.readdirSync(resolvedDir);
    } catch {
      return out;
    }
    for (const name of names) {
      if (!name.endsWith(".json") || name.startsWith(".")) continue;
      let text;
      try {
        text = fs.readFileSync(path.join(resolvedDir, name), "utf8");
      } catch {
        continue;
      }
      const rec = parseStoreFile(text);
      if (!rec) {
        console.error(`[loop-web] skipping corrupt store file: ${name}`);
        continue;
      }
      out.set(rec.runId, rec);
    }
    return out;
  }

  return { save, flush, flushAll, loadAll, fileFor };
}

/** True iff `abs` is `base` or lives under it (the trailing sep stops a `<base>-x` sibling). */
function isContained(abs, base) {
  return abs === base || abs.startsWith(base + path.sep);
}
