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

describe("computeLayout — repository reviews", () => {
  it("stacks review terminals without overlap", () => {
    const layout = computeLayout(buildGraph(parsePlan(PLAN), null));
    const api = layout.positions.get("pr-review:acme/api")!;
    const web = layout.positions.get("pr-review:acme/web")!;
    expect(Math.abs(api.y - web.y)).toBeGreaterThanOrEqual(NODE_H);
    expect(api.x).toBe(web.x);
  });
});
