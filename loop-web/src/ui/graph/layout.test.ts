import { describe, expect, it } from "vitest";
import { buildGraph } from "../../model/build-graph.ts";
import { parsePlan } from "../../model/parse-plan.ts";
import { NODE_H, NODE_W, computeLayout } from "./layout.ts";

const PLAN = `# Split — Multi-Phase Plan
## Loop config
- **Integration branch:** \`feat/split\`
## Repositories
### \`acme/api\`
- **Verify:** \`true\`
### \`acme/web\`
- **Verify:** \`true\`
## Phases
### Phase 1 — API \`[lane: A]\` \`[status: todo]\`
- **Repository:** \`acme/api\`
- **Depends on:** none
### Phase 2 — Web \`[lane: A]\` \`[status: todo]\`
- **Repository:** \`acme/web\`
- **Depends on:** Phase 1
`;

const REVIEW_ROUNDS = `# Editable — Multi-Phase Plan
## Phases
### Phase 1 — Initial work \`[lane: A]\` \`[status: done]\`
- **Depends on:** none
### Phase 2 — First review \`[lane: review]\` \`[status: done]\` \`[kind: pr-review]\`
- **Depends on:** Phase 1
### Phase 3 — Follow-up \`[lane: A]\` \`[status: todo]\`
- **Depends on:** Phase 2
### Phase 4 — Final review \`[lane: review]\` \`[status: todo]\` \`[kind: pr-review]\`
- **Depends on:** Phase 3
`;

describe("computeLayout — repository reviews", () => {
  it("stacks review terminals without overlap", () => {
    const layout = computeLayout(buildGraph(parsePlan(PLAN), null));
    const api = layout.positions.get("pr-review:acme/api")!;
    const web = layout.positions.get("pr-review:acme/web")!;
    expect(Math.abs(api.y - web.y)).toBeGreaterThanOrEqual(NODE_H);
    expect(api.x).toBe(web.x);
  });

  it("places explicit review phases at their dependency layer", () => {
    const positions = computeLayout(buildGraph(parsePlan(REVIEW_ROUNDS), null)).positions;
    expect(positions.get("1")!.x).toBeLessThan(positions.get("2")!.x);
    expect(positions.get("2")!.x).toBeLessThan(positions.get("3")!.x);
    expect(positions.get("3")!.x).toBeLessThan(positions.get("4")!.x);
  });
});

const MILESTONES = `# Ship twice — Multi-Phase Plan
## Milestone 1 — Ship the core
### Phase 1 — Build the core [lane: A]
- **Depends on:** none
### Phase 2 — Review the core [kind: pr-review] [gate: merge]
- **Depends on:** Phase 1
## Milestone 2 — Ship the API
- **Integration branch:** \`feat/second\`
### Phase 3 — Build the API [lane: A]
- **Depends on:** Phase 2
### Phase 4 — Review the API [kind: pr-review]
- **Depends on:** Phase 3
`;

describe("computeLayout — milestones", () => {
  it("groups multi-milestone plans into ordered, non-overlapping column bands", () => {
    const layout = computeLayout(buildGraph(parsePlan(MILESTONES), null));
    expect(layout.milestones.map((m) => m.milestone)).toEqual([1, 2]);
    const [m1, m2] = layout.milestones;
    expect(m1.label).toContain("M1");
    expect(m1.label).toContain("Ship the core");
    expect(m1.x + m1.width).toBeLessThanOrEqual(m2.x);
    for (const id of ["1", "2"]) {
      const p = layout.positions.get(id)!;
      expect(p.x).toBeGreaterThanOrEqual(m1.x);
      expect(p.x + NODE_W).toBeLessThanOrEqual(m1.x + m1.width);
    }
    expect(layout.positions.get("3")!.x).toBeGreaterThanOrEqual(m2.x);
  });

  it("leaves single-milestone layouts unchanged", () => {
    const layout = computeLayout(buildGraph(parsePlan(REVIEW_ROUNDS), null));
    expect(layout.milestones).toEqual([]);
    const legacy = parsePlan(REVIEW_ROUNDS.replace("## Phases", "## Milestone 1 — Only"));
    const single = computeLayout(buildGraph(legacy, null));
    expect(single.milestones).toEqual([]);
    expect(single.positions).toEqual(layout.positions);
    expect(single.height).toBe(layout.height);
  });
});
