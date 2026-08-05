import { describe, it, expect } from "vitest";
import { parsePlan } from "./parse-plan.ts";

const MULTI_LANE = `<!-- kestral-plan
project: Widgets
projectId: proj_1
workContextId: doc_9
docUrl: https://kestral.example/doc_9
-->

# Widget revamp — Multi-Phase Plan

**Status:** in progress
**Last updated:** 2026-08-01 by Codex (feat/widget-core)
**Kestral project:** [Widgets](https://kestral.example/proj_1)

## Loop config

- **Integration branch:** \`feat/widget-revamp\`
- **Verify:** \`npm test\`
- **PR:** [#42](https://github.com/acme/widgets/pull/42)
- **Concurrency:** 3

## Goal
Rebuild widgets.

## Phases

### Phase 1 — Add the widget core \`[lane: A]\` \`[status: done]\`
- **Task:** [core - Add core](https://kestral.example/t1)
- **Depends on:** none
- **Suggested branch:** \`feat/widget-core\`
- **Touches:** src/core
- **Done when:** core compiles
- **Verify:** \`npm run test:core\`

### Phase 2 — Migrate retries to a state machine \`[lane: B]\` \`[status: in-progress]\`
- **Task:** [retry - Retry SM](https://kestral.example/t2)
- **Depends on:** none
- **Suggested branch:** \`refactor/retry-state\`
- **Done when:** retries deterministic

### Phase 3 — Wire the core into the API \`[lane: A]\` \`[status: todo]\`
- **Depends on:** Phase 1
- **Suggested branch:** \`feat/api-wire\`
- **Notes:** same PR as Phase 1

### Phase 4 — Integrate and run e2e \`[lane: integration]\` \`[status: todo]\`
- **Depends on:** Phase 3, Phase 2
- **Done when:** both lanes merged and green
`;

describe("parsePlan — header", () => {
  it("extracts effort name, stripping the '— Multi-Phase Plan' suffix", () => {
    expect(parsePlan(MULTI_LANE).name).toBe("Widget revamp");
  });
  it("reads the Status header value", () => {
    expect(parsePlan(MULTI_LANE).status).toBe("in progress");
  });
  it("reads the Last updated value", () => {
    expect(parsePlan(MULTI_LANE).updatedAt).toBe(
      "2026-08-01 by Codex (feat/widget-core)",
    );
  });
});

describe("parsePlan — loop config", () => {
  it("parses the loop config block", () => {
    const cfg = parsePlan(MULTI_LANE).loopConfig!;
    expect(cfg.integrationBranch).toBe("feat/widget-revamp");
    expect(cfg.verify).toBe("npm test");
    expect(cfg.pr).toBe("https://github.com/acme/widgets/pull/42");
    expect(cfg.concurrency).toBe(3);
  });
  it("returns null loopConfig when the section is absent", () => {
    const noLoop = MULTI_LANE.replace(
      /## Loop config[\s\S]*?(?=## Goal)/,
      "",
    );
    expect(parsePlan(noLoop).loopConfig).toBeNull();
  });
  it("treats a '_none yet_' PR as null", () => {
    const p = MULTI_LANE.replace(
      "- **PR:** [#42](https://github.com/acme/widgets/pull/42)",
      "- **PR:** _none yet_",
    );
    expect(parsePlan(p).loopConfig!.pr).toBeNull();
  });
});

describe("parsePlan — phases", () => {
  it("parses every phase in document order", () => {
    const phases = parsePlan(MULTI_LANE).phases;
    expect(phases.map((p) => p.phase)).toEqual(["1", "2", "3", "4"]);
  });
  it("extracts the descriptive title without the bracket markers", () => {
    const p1 = parsePlan(MULTI_LANE).phases[0];
    expect(p1.title).toBe("Add the widget core");
  });
  it("parses lane and status markers", () => {
    const phases = parsePlan(MULTI_LANE).phases;
    expect(phases.map((p) => p.lane)).toEqual(["A", "B", "A", "integration"]);
    expect(phases.map((p) => p.status)).toEqual([
      "done",
      "in-progress",
      "todo",
      "todo",
    ]);
  });
  it("parses Depends on: none / single / multiple", () => {
    const phases = parsePlan(MULTI_LANE).phases;
    expect(phases[0].dependsOn).toEqual([]);
    expect(phases[2].dependsOn).toEqual(["1"]);
    expect(phases[3].dependsOn).toEqual(["3", "2"]);
  });
  it("expands plural 'Phases N–M' ranges (en-dash, em-dash, hyphen)", () => {
    const dep = (line: string) =>
      parsePlan(
        `# X — Multi-Phase Plan\n\n## Phases\n\n### Phase 9 — T \`[lane: A]\` \`[status: todo]\`\n- **Depends on:** ${line}\n`,
      ).phases[0].dependsOn;
    expect(dep("Phases 1–4")).toEqual(["1", "2", "3", "4"]); // en-dash
    expect(dep("Phases 1—4")).toEqual(["1", "2", "3", "4"]); // em-dash
    expect(dep("Phase 1-3")).toEqual(["1", "2", "3"]); // hyphen, singular
    expect(dep("Phases 2, 3")).toEqual(["2", "3"]); // plural, enumerated
    expect(dep("Phase 1 (prose), Phase 2")).toEqual(["1", "2"]); // trailing prose, deduped in order
  });
  it("captures per-phase fields (branch, verify, doneWhen, task, notes)", () => {
    const p1 = parsePlan(MULTI_LANE).phases[0];
    expect(p1.suggestedBranch).toBe("feat/widget-core");
    expect(p1.verify).toBe("npm run test:core");
    expect(p1.doneWhen).toBe("core compiles");
    expect(p1.taskUrl).toBe("https://kestral.example/t1");
    const p3 = parsePlan(MULTI_LANE).phases[2];
    expect(p3.notes).toBe("same PR as Phase 1");
    expect(p3.verify).toBeNull();
  });
});

describe("parsePlan — prose sections", () => {
  const WITH_PROSE = `# Prosy — Multi-Phase Plan

## Goal
Ship the thing.
It matters.

## Approach & key decisions
Use a state machine.

## Parallelization guide
Lane A and B are independent.

## Phases

### Phase 1 — Do it \`[lane: A]\` \`[status: todo]\`
- **Depends on:** none

## Progress log
- 2026-08-01 kicked off
`;

  it("extracts the Goal prose (multi-line, joined and trimmed)", () => {
    expect(parsePlan(WITH_PROSE).prose.goal).toBe("Ship the thing.\nIt matters.");
  });
  it("extracts the Approach prose from the '& key decisions' heading", () => {
    expect(parsePlan(WITH_PROSE).prose.approach).toBe("Use a state machine.");
  });
  it("accepts a bare '## Approach' heading", () => {
    const md = "# X — Multi-Phase Plan\n\n## Approach\nKeep it simple.\n";
    expect(parsePlan(md).prose.approach).toBe("Keep it simple.");
  });
  it("extracts the Parallelization guide from any '## Parallel…' heading", () => {
    expect(parsePlan(WITH_PROSE).prose.parallelGuide).toBe("Lane A and B are independent.");
    const md = "# X — Multi-Phase Plan\n\n## Parallel work\nSplit by lane.\n";
    expect(parsePlan(md).prose.parallelGuide).toBe("Split by lane.");
  });
  it("extracts the Progress log", () => {
    expect(parsePlan(WITH_PROSE).prose.progressLog).toBe("- 2026-08-01 kicked off");
  });
  it("leaves every prose field null when no prose section is present", () => {
    const md = "# X — Multi-Phase Plan\n\n## Phases\n\n### Phase 1 — T \`[lane: A]\` \`[status: todo]\`\n- **Depends on:** none\n";
    expect(parsePlan(md).prose).toEqual({
      goal: null,
      approach: null,
      parallelGuide: null,
      progressLog: null,
    });
  });
});

describe("parsePlan — tolerance & warnings", () => {
  it("tolerates extra whitespace inside the markers", () => {
    const md = `# X — Multi-Phase Plan\n\n## Phases\n\n###   Phase 7   —   Loose spacing   \`[lane:  C ]\`  \`[status:  blocked ]\`\n- **Depends on:** none\n`;
    const p = parsePlan(md).phases[0];
    expect(p.phase).toBe("7");
    expect(p.lane).toBe("C");
    expect(p.status).toBe("blocked");
    expect(p.title).toBe("Loose spacing");
  });
  it("defaults an unknown status to todo and records a warning", () => {
    const md = `# X — Multi-Phase Plan\n\n## Phases\n\n### Phase 1 — T \`[lane: A]\` \`[status: frobnicated]\`\n`;
    const plan = parsePlan(md);
    expect(plan.phases[0].status).toBe("todo");
    expect(plan.warnings.join(" ")).toMatch(/frobnicated/);
  });
  it("falls back to the whole H1 when there is no em-dash suffix", () => {
    expect(parsePlan("# Just A Name\n").name).toBe("Just A Name");
  });
  it("de-duplicates a repeated Phase N heading (keeps first) and warns", () => {
    const md = `# X — Multi-Phase Plan\n\n## Phases\n\n### Phase 2 — First \`[lane: A]\` \`[status: todo]\`\n- **Depends on:** none\n\n### Phase 2 — Dup \`[lane: A]\` \`[status: done]\`\n- **Depends on:** none\n`;
    const plan = parsePlan(md);
    expect(plan.phases.map((p) => p.phase)).toEqual(["2"]);
    expect(plan.phases[0].title).toBe("First");
    expect(plan.warnings.join(" ")).toMatch(/Duplicate Phase 2/);
  });
});
