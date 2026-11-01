export interface LoopPhase {
  phase: string;
  title: string;
  lane: string;
  milestone: number;
  rawStatus: string;
  start: string | null;
  end: string | null;
  minutes: number | null;
  estimate: { min: number; max: number } | null;
  diff: { files: number; insertions: number; deletions: number } | null;
  verify: { passed: boolean; summary: string | null } | null;
  drift: { kind: string; detail: string }[];
}

export interface LoopCheckpoint {
  name: string;
  phases: LoopPhase[];
  milestones: { number: number; title: string }[];
}

export type CheckpointReader = (options: { dir: string; now: string; all: boolean }) => LoopCheckpoint;

export async function readCheckpoint(dir: string, now: string): Promise<LoopCheckpoint> {
  const url = new URL("../../../../loop-checkpoint.mjs", import.meta.url);
  const module = await import(url.href) as { checkpoint: CheckpointReader };
  return module.checkpoint({ dir, now, all: true });
}
