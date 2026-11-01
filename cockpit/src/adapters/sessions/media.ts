import { lstat, readdir, realpath } from "node:fs/promises";
import { basename, extname, isAbsolute, join, relative, resolve, sep } from "node:path";
import type { CockpitEvent } from "../../core/events.ts";
import type { SessionSnapshot } from "./session.ts";
import { asObject, belugaChatId, digest } from "./session.ts";

export interface MediaScan {
  events: CockpitEvent[];
  signatures: Record<string, string>;
}

const MIME_TYPES: Record<string, string> = {
  ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".gif": "image/gif",
  ".webp": "image/webp", ".avif": "image/avif", ".bmp": "image/bmp",
};

export async function sessionMedia(
  session: SessionSnapshot, userHome: string, candidates: string[], previous: Record<string, string>,
): Promise<MediaScan> {
  const signatures = { ...previous };
  const events: CockpitEvent[] = [];
  const cwd = session.meta.cwd;
  if (!cwd) return { events, signatures };
  const chatId = belugaChatId(cwd, userHome);
  const scratch = chatId ? join(userHome, ".beluga/chats", chatId, ".beluga/scratch") : undefined;
  const files = new Map<string, string>();
  if (scratch) {
    const canonicalHome = await optionalRealpath(userHome);
    const canonicalScratch = await optionalRealpath(scratch);
    if (canonicalHome && canonicalScratch === join(canonicalHome, ".beluga/chats", chatId!, ".beluga/scratch")) {
      for (const path of await regularFiles(scratch)) files.set(path, canonicalScratch);
    }
  }
  const canonicalCwd = await optionalRealpath(cwd);
  if (canonicalCwd) {
    for (const path of candidates) files.set(path, canonicalCwd);
  }
  for (const [path, allowedRoot] of files) {
    const mimeType = MIME_TYPES[extname(path).toLowerCase()];
    if (!mimeType) continue;
    const canonical = await optionalRealpath(path);
    if (!canonical || !inside(allowedRoot, canonical)) continue;
    const info = await optionalStat(path);
    if (!info?.isFile() || info.isSymbolicLink()) continue;
    const signature = `${info.mtimeMs}:${info.size}`;
    if (signatures[canonical] === signature) continue;
    signatures[canonical] = signature;
    const id = `media-${digest(canonical)}`;
    events.push({
      id: `${session.runId}:${id}:${signature}`, runId: session.runId,
      sessionId: session.meta.sessionId, timestamp: new Date(info.mtimeMs).toISOString(), type: "media",
      media: { id, path: canonical, mimeType, title: basename(path) },
    });
  }
  return { events, signatures };
}

export function writtenMediaPaths(events: CockpitEvent[], cwd: string | undefined): string[] {
  if (!cwd) return [];
  const paths: string[] = [];
  for (const event of events) {
    if (event.type !== "action" || !/(?:^|\.)(?:Write|Edit|write_file|apply_patch)$/.test(event.action.tool)) continue;
    const input = event.action.input;
    const object = asObject(input);
    const path = object.file_path ?? object.path;
    if (typeof path === "string") paths.push(resolve(cwd, path));
    if (typeof input === "string") {
      for (const match of input.matchAll(/^\*\*\* (?:Add|Update) File: (.+)$/gm)) paths.push(resolve(cwd, match[1]));
    }
  }
  return paths.filter((path) => !!MIME_TYPES[extname(path).toLowerCase()]);
}

export async function regularFiles(root: string): Promise<string[]> {
  const info = await optionalStat(root);
  if (!info?.isDirectory() || info.isSymbolicLink()) return [];
  const files: string[] = [];
  const entries = await readdir(root, { withFileTypes: true });
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) files.push(...await regularFiles(path));
    else if (entry.isFile()) files.push(path);
  }
  return files;
}

function inside(root: string, path: string): boolean {
  const local = relative(root, path);
  return !!local && local !== ".." && !local.startsWith(`..${sep}`) && !isAbsolute(local);
}

async function optionalRealpath(path: string): Promise<string | undefined> {
  try { return await realpath(path); } catch (error) {
    if (missing(error)) return undefined;
    throw error;
  }
}

async function optionalStat(path: string) {
  try { return await lstat(path); } catch (error) {
    if (missing(error)) return undefined;
    throw error;
  }
}

function missing(error: unknown): boolean {
  return ["ENOENT", "ENOTDIR"].includes((error as NodeJS.ErrnoException).code ?? "");
}
