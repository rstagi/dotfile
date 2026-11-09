import { readFileSync, statSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import type { LoopState } from "./index.ts";

export interface LoopSession {
  runId: string;
  phaseId: string;
  laneId?: string;
}

/** Match the cwd to its phase. A sibling path does not match a worktree prefix. */
export function matchLoopSession(state: LoopState, cwd: string): LoopSession | undefined {
  if (!state.runId) return undefined;
  const matches = Object.entries(state.phases ?? {}).filter(([, phase]) => {
    if (!phase.worktree) return false;
    const path = relative(resolve(phase.worktree), resolve(cwd));
    return path !== ".." && !path.startsWith(`..${sep}`) && !isAbsolute(path);
  });
  // A nested worktree is more specific than its parent worktree.
  matches.sort(([, a], [, b]) => b.worktree!.length - a.worktree!.length);
  const match = matches[0];
  return match ? { runId: state.runId, phaseId: match[0], ...(match[1].lane ? { laneId: match[1].lane } : {}) } : undefined;
}

/** Discover Loop sources from a cwd, including the source of a linked Git worktree. */
export function discoverLoopDirectories(cwd: string): string[] {
  const directories = new Set<string>();
  const add = (dir: string) => {
    try { if (statSync(join(dir, "plan.md")).isFile()) directories.add(dir); } catch { /* The source is absent. */ }
  };
  for (let current = resolve(cwd); ; current = dirname(current)) {
    add(join(current, ".loop"));
    try {
      const git = readFileSync(join(current, ".git"), "utf8").match(/^gitdir:\s*(.+)/)?.[1];
      if (git) {
        const gitdir = resolve(current, git.trim());
        const common = readFileSync(join(gitdir, "commondir"), "utf8").trim();
        add(join(dirname(resolve(gitdir, common)), ".loop"));
      }
    } catch { /* This directory is not a linked worktree. */ }
    if (dirname(current) === current) break;
  }
  return [...directories];
}
