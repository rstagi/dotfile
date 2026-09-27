import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { GraphNode, PrInfo } from "../../model/types.ts";
import { PhaseNode } from "./PhaseNode.tsx";

vi.mock("@xyflow/react", () => ({
  Handle: () => null,
  Position: { Left: "left", Right: "right" },
}));

function render(over: Partial<GraphNode>, pr?: PrInfo | null): string {
  const node = {
    id: "2",
    kind: "phase",
    title: "Build feature",
    phase: "2",
    lane: "A",
    repository: "acme/api",
    status: "running",
    ui: "running",
    pulse: null,
    runtime: null,
    notePending: false,
    noteMarkdown: null,
    ...over,
  } as GraphNode;
  return renderToStaticMarkup(PhaseNode({ data: { node, pr }, selected: false } as never));
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

describe("PhaseNode repository badge", () => {
  it("renders the repository slug on phase nodes", () => {
    expect(render({ repository: "acme/api" })).toContain("acme/api");
  });

  it("shows the latest Jev decision badge and omits it for pre-Jev nodes", () => {
    expect(render({ decisions: [{
      runId: "r", phase: "2", attempt: 1, stage: "route", mode: "shadow", candidate: "light",
      confidence: 0.91, probabilities: { light: 0.91 }, appliedAction: "default",
      fallbackReason: "shadow-mode", resolvedModel: "systemone", ts: "2026-09-27T10:00:00Z",
    }] })).toContain("JEV · ROUTE · SHADOW");
    expect(render({ decisions: undefined })).not.toContain("JEV");
  });
});

describe("PhaseNode explicit review rounds", () => {
  it("does not show a prior approval on a new todo review phase", () => {
    const html = render(
      { kind: "pr-review", phase: "4", repository: null, status: "todo", ui: "todo" },
      {
        url: "https://github.com/acme/api/pull/1",
        outcome: "done",
        verdict: "approved in the previous round",
        reviewPresent: true,
        reportPath: null,
        commentUrl: null,
        reviewSlug: null,
        reviewAttempt: null,
      },
    );
    expect(html).not.toContain("APPROVED");
    expect(html).toContain("awaiting review");
  });
});
