// Source: ~/Dev/ratel-ai/ratel-cloud/apps/web/lib/intent-engine/names-job.ts (central-phrase step)
// namePhrases source: ~/Dev/ratel-ai/ratel-cloud/apps/web/lib/intent-engine/names.ts
import { centralPhrases } from "./naming.ts";

export const NAME_PHRASES = 6;

export function namePhrases(members: readonly string[]): string[] {
  return centralPhrases(members, Math.min(NAME_PHRASES, members.length));
}

export function intentPhrases(children: readonly string[], members: ReadonlyMap<string, readonly string[]>): string[] {
  return namePhrases(children.flatMap((id) => members.get(id) ?? []));
}
