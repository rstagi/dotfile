import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { GraphNode, PrInfo } from "../../model/types.ts";
import { PhaseNode, reviewNodePr } from "./PhaseNode.tsx";

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

describe("PhaseNode merge gate", () => {
  it("labels a parked gate AWAITING MERGE with the resume hint", () => {
    const html = render({ kind: "pr-review", phase: "2", repository: null, status: "running", ui: "awaiting", awaitingMerge: true, gate: "merge" });
    expect(html).toContain("AWAITING MERGE");
    expect(html).toContain("/loop-execute resume");
  });

  it("keeps the normal label when not parked", () => {
    const html = render({ kind: "pr-review", phase: "2", repository: null, status: "todo", ui: "todo", gate: "merge" });
    expect(html).not.toContain("AWAITING MERGE");
  });
});

describe("PhaseNode historical gates", () => {
  const currentPr: PrInfo = {
    url: "https://github.com/acme/web/pull/42", outcome: "blocked", verdict: "m2 broken", reviewPresent: true,
    reportPath: null, commentUrl: null, reviewSlug: null, reviewAttempt: null,
  };
  const gate: Partial<GraphNode> = {
    kind: "pr-review", phase: "2", repository: null, lane: "review", milestone: 1, gate: "merge", status: "done", ui: "done",
    prs: { "acme/api": "https://github.com/acme/api/pull/9" },
  };

  it("never hands a historical gate the current milestone's PR", () => {
    expect(reviewNodePr(gate as GraphNode, 2, currentPr)).toBeNull();
    expect(reviewNodePr({ ...gate, milestone: 2 } as GraphNode, 2, currentPr)).toBe(currentPr);
  });

  it("renders the gate's own recorded PRs instead of 'no PR yet'", () => {
    const html = render(gate, reviewNodePr(gate as GraphNode, 2, currentPr));
    expect(html).toContain('href="https://github.com/acme/api/pull/9"');
    expect(html).not.toContain("pull/42");
    expect(html).not.toContain("no PR yet");
  });

  it("renders recorded PRs on a parked gate", () => {
    const html = render({ ...gate, status: "running", ui: "awaiting", awaitingMerge: true }, null);
    expect(html).toContain('href="https://github.com/acme/api/pull/9"');
    expect(html).toContain("/loop-execute resume");
  });
  it("puts the resume hint on its own row and lets parked PR badges wrap unclipped", () => {
    const prs = { "acme/api": "https://github.com/acme/api/pull/9", "acme/web": "https://github.com/acme/web/pull/12" };
    const html = render({ ...gate, status: "running", ui: "awaiting", awaitingMerge: true, prs }, null);
    expect(html).toContain("node--parked");
    expect(html).toMatch(/<div class="node__resume"[^>]*>\/loop-execute resume<\/div>/);
    const row = html.match(/<div class="node__prs">(.*?)<\/div>/)?.[1] ?? "";
    expect(row).toContain("api #9");
    expect(row).toContain("web #12");
    expect(row.match(/node__badge node__badge--pr/g)).toHaveLength(2);
  });
});
