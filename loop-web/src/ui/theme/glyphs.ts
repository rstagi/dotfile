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
