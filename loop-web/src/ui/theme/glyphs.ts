import type { Problem } from "../../model/types.ts";

/** Glyph + colour token for each problem class shown in the rail and drawer. */
export function problemStyle(cls: Problem["class"]): { glyph: string; color: string; label: string } {
  switch (cls) {
    case "hil": return { glyph: "⚠", color: "var(--amber)", label: "HIL pause" };
    case "question": return { glyph: "?", color: "var(--violet)", label: "Question" };
    case "blocked": return { glyph: "⊘", color: "var(--red)", label: "Blocked" };
    case "verify-fail": return { glyph: "✗", color: "var(--red)", label: "Verify failed" };
    case "stall": return { glyph: "◌", color: "var(--red)", label: "Stalled" };
    case "crash": return { glyph: "✕", color: "var(--red)", label: "Crash" };
    case "timeout": return { glyph: "⏱", color: "var(--red)", label: "Timeout" };
    case "chain-exhausted": return { glyph: "⛓", color: "var(--red)", label: "Chain exhausted" };
    default: return { glyph: "•", color: "var(--ink-soft)", label: cls };
  }
}

/** The review verdict pill shared by the PR-review node face and the drawer. Keys off the
 * STRUCTURED outcome only (never the free-form summary prose). Null ⇒ no verdict yet. */
export function reviewPill(outcome: string | null | undefined): { label: string; color: string } | null {
  if (outcome === "done") return { label: "APPROVED", color: "var(--green)" };
  if (outcome === "blocked" || outcome === "question")
    return { label: "CHANGES REQUESTED", color: "var(--red)" };
  return null;
}

/** Lifecycle-dot colour for a loop's status in the selector. */
export function loopStatusColor(status: string): string {
  switch (status) {
    case "active": return "var(--aqua)";
    case "finished": return "var(--green)";
    case "paused": return "var(--amber)";
    case "planned": return "var(--violet)"; // plan registered, not yet running
    case "archived": return "var(--ink-dim)";
    default: return "var(--steel)";
  }
}

/** Colour token for a node's resolved UI state (used by the drawer's state line). */
export function uiColor(ui: string): string {
  switch (ui) {
    case "running": return "var(--aqua)";
    case "done": return "var(--green)";
    case "awaiting": return "var(--amber)";
    case "blocked":
    case "problem": return "var(--red)";
    default: return "var(--steel)";
  }
}
