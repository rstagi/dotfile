// Source: ~/Dev/ratel-ai/ratel-cloud/apps/web/lib/intent-engine/types.ts (type extraction)
export interface LevelNode {
  uuid: string;
  level: number;
  /** uuids one level down. */
  children: string[];
  label: string;
  terms: string[];
}

export interface LevelTree {
  /** levels[0] is unused (level 0 is the clusters); levels[k] holds level k's nodes. */
  levels: LevelNode[][];
  /** child uuid (any level) -> parent uuid one level up. */
  parentOf: Map<string, string>;
}
