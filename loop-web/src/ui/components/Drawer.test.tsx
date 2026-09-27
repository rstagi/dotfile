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
    expect(html).toContain("default");
    expect(html).toContain("91%");
    expect(html).toContain("shadow-mode");
    expect(html).toContain("systemone");
    expect(html).toContain("proposed profile");
    expect(html).toContain("actual profile");
  });

  it("renders no Jev section for a pre-Jev node", () => {
    expect(renderToStaticMarkup(<Drawer node={node()} pr={null} plan={null} runId={null} onClose={() => {}} />))
      .not.toContain("Jev decisions");
  });

  it("labels question triage advice, disposition, and checked evidence", () => {
    const html = renderToStaticMarkup(<Drawer node={node([{
      runId: "r", phase: "2", attempt: 2, stage: "question", mode: "active",
      candidate: "human-preference", confidence: 0.88,
      probabilities: { "human-preference": 0.88, uncertain: 0.12 },
      appliedAction: "raise-hil", fallbackReason: null, resolvedModel: "systemone",
      evidenceChecked: true, evidenceSources: ["question", "plan"], ts: "2026-09-27T10:00:00Z",
    }])} pr={null} plan={null} runId={null} onClose={() => {}} />);
    expect(html).toContain("triage suggestion");
    expect(html).toContain("actual action");
    expect(html).toContain("human-preference");
    expect(html).toContain("raise-hil");
    expect(html).toContain("question, plan");
  });

  it("labels merge risk and the actual full-skim disposition", () => {
    const html = renderToStaticMarkup(<Drawer node={node([{
      runId: "r", phase: "5", attempt: 1, stage: "merge-risk", mode: "active",
      candidate: "scope-gap:possible · risk:high", confidence: 0.91,
      probabilities: { "risk:high": 0.91 }, appliedAction: "focused-full-diff-skim",
      fallbackReason: null, resolvedModel: "jev-test", ts: "2026-09-27T10:00:00Z",
    }])} pr={null} plan={null} runId={null} onClose={() => {}} />);
    expect(html).toContain("risk judgment");
    expect(html).toContain("scope-gap:possible · risk:high");
    expect(html).toContain("actual disposition");
    expect(html).toContain("focused-full-diff-skim");
  });

  it("shows each same-second merge head with its gates and focus", () => {
    const decision = {
      runId: "r", phase: "2", attempt: 1, stage: "merge-risk", mode: "active" as const,
      candidate: "scope-gap:possible · risk:medium", confidence: 0.82,
      probabilities: { "risk:medium": 0.82 }, appliedAction: "focused-full-diff-skim",
      fallbackReason: null, resolvedModel: "systemone", ts: "2026-09-27T10:00:00Z",
      requiredGates: ["verified-exit-zero", "full-diff-skim", "serialized-merge"],
      completedGates: ["verified-exit-zero", "full-diff-skim"],
      remainingGates: ["serialized-merge"],
    };
    const html = renderToStaticMarkup(<Drawer node={node([
      { ...decision, head: "abc123", focus: ["src/payment.ts"] },
      { ...decision, head: "def456", focus: ["src/cart.ts"] },
    ])} pr={null} plan={null} runId={null} onClose={() => {}} />);

    expect(html.match(/class="jev-decision"/g)).toHaveLength(2);
    expect(html).toContain("abc123");
    expect(html).toContain("def456");
    expect(html).toContain("required gates");
    expect(html).toContain("completed gates");
    expect(html).toContain("remaining gates");
    expect(html).toContain("verified-exit-zero");
    expect(html).toContain("serialized-merge");
    expect(html).toContain("src/payment.ts");
    expect(html).toContain("src/cart.ts");
  });
});
