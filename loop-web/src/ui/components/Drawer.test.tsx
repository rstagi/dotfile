import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { GraphNode } from "../../model/types.ts";
import { Drawer } from "./Drawer.tsx";

function node(decisions?: GraphNode["decisions"]): GraphNode {
  return {
    id: "2", kind: "phase", title: "Build feature", phase: "2", lane: "A", repository: "primary",
    status: "running", ui: "running", pulse: null, runtime: null, notePending: false, noteMarkdown: null,
    decisions,
  };
}

describe("Drawer Jev decisions", () => {
  it("shows proposed versus applied action, confidence, fallback, and model", () => {
    const html = renderToStaticMarkup(<Drawer node={node([{
      runId: "r", phase: "2", attempt: 1, stage: "route", mode: "shadow", candidate: "light",
      confidence: 0.91, probabilities: { light: 0.91, default: 0.09 }, appliedAction: "default",
      fallbackReason: "shadow-mode", resolvedModel: "systemone", ts: "2026-09-27T10:00:00Z",
    }])} pr={null} plan={null} runId={null} onClose={() => {}} />);
    expect(html).toContain("Jev decisions");
    expect(html).toContain("proposed");
    expect(html).toContain("light");
    expect(html).toContain("applied");
    expect(html).toContain("default");
    expect(html).toContain("91%");
    expect(html).toContain("shadow-mode");
    expect(html).toContain("systemone");
  });

  it("renders no Jev section for a pre-Jev node", () => {
    expect(renderToStaticMarkup(<Drawer node={node()} pr={null} plan={null} runId={null} onClose={() => {}} />))
      .not.toContain("Jev decisions");
  });
});
