import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { GraphNode } from "../../model/types.ts";
import { PhaseNode } from "./PhaseNode.tsx";

vi.mock("@xyflow/react", () => ({
  Handle: () => null,
  Position: { Left: "left", Right: "right" },
}));

function render(over: Partial<GraphNode>): string {
  const node = {
    id: "2",
    kind: "phase",
    title: "Build feature",
    phase: "2",
    lane: "A",
    status: "running",
    ui: "running",
    pulse: null,
    runtime: null,
    notePending: false,
    noteMarkdown: null,
    ...over,
  } as GraphNode;
  return renderToStaticMarkup(PhaseNode({ data: { node }, selected: false } as never));
}

describe("PhaseNode steering note badge", () => {
  it("renders NOTE with the note text as its title when pending", () => {
    const html = render({ notePending: true, noteMarkdown: "rebase first" });
    expect(html).toContain('class="node__note"');
    expect(html).toContain('title="rebase first"');
    expect(html).toContain("NOTE");
  });

  it("omits NOTE when the note is no longer pending", () => {
    expect(render({ notePending: false, noteMarkdown: "stale" })).not.toContain("node__note");
  });
});
