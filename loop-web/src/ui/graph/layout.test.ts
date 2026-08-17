import { describe, expect, it } from "vitest";
import { buildGraph } from "../../model/build-graph.ts";
import { parsePlan } from "../../model/parse-plan.ts";
import { NODE_H, computeLayout } from "./layout.ts";

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
