import { describe, it, expect } from "vitest";
import { parsePlan } from "./parse-plan.ts";

const MULTI_LANE = `<!-- loop-plan
planId: loop-widget-revamp-2026-08-01
daemon: http://localhost:7717
-->

# Widget revamp — Multi-Phase Plan

**Status:** in progress
**Last updated:** 2026-08-01 by Codex (feat/widget-core)

## Loop config

- **Integration branch:** \`feat/widget-revamp\`
- **Verify:** \`npm test\`
- **PR:** [#42](https://github.com/acme/widgets/pull/42)
- **Concurrency:** 3

## Goal
Rebuild widgets.

## Phases

### Phase 1 — Add the widget core \`[lane: A]\` \`[status: done]\`
- **Depends on:** none
- **Suggested branch:** \`feat/widget-core\`
- **Touches:** src/core
- **Done when:** core compiles
- **Verify:** \`npm run test:core\`

### Phase 2 — Migrate retries to a state machine \`[lane: B]\` \`[status: in-progress]\`
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

const MULTI_REPO = `# Multi repo — Multi-Phase Plan

## Loop config
- **Integration branch:** \`feat/shared\`
- **Concurrency:** 2

## Repositories

### \`acme/api\`
- **Verify:** \`npm test\`
- **Integration branch:** \`feat/api\`
- **PR:** [#10](https://github.com/acme/api/pull/10)

### \`acme/web\`
- **Verify:** \`npm run check\`
- **PR:** _none yet_

## Phases

### Phase 1 — API \`[lane: A]\` \`[status: todo]\`
- **Repository:** \`acme/api\`
- **Depends on:** none

### Phase 2 — Web \`[lane: A]\` \`[status: todo]\`
- **Repository:** \`acme/web\`
- **Depends on:** Phase 1
`;

const EDITABLE_WITH_REVIEW = `# Editable — Multi-Phase Plan

## Phases

### Phase 1 — Build the first slice \`[lane: A]\` \`[status: done]\`
- **Depends on:** none

### Phase 3 — Add a late slice before review \`[lane: A]\` \`[status: todo]\`
- **Depends on:** Phase 1

### Phase 2 — Review pull requests \`[lane: review]\` \`[status: todo]\` \`[kind: pr-review]\`
- **Depends on:** Phase 3
- **Done when:** every repository PR has a non-blocking review verdict
`;

const TWO_MILESTONES = `# Ship twice — Multi-Phase Plan

**Status:** awaiting merge

## Loop config
- **Integration branch:** \`feat/first\`

## Milestone 1 — Ship the core

### Phase 1 — Build the core [lane: A]
- **Depends on:** none

### Phase 2 — Review the core [kind: pr-review] \`[gate: merge]\`
- **Depends on:** Phase 1

## Milestone 2 — Ship the API
- **Integration branch:** \`feat/second\`

### Phase 4 — Build the API [lane: A]
- **Depends on:** Phase 2

### Phase 3 — Review the API [kind: pr-review]
- **Depends on:** Phase 4

## Progress log
### Phase 99 — A historical heading, not executable
`;

const MULTI_REPO_MILESTONES = MULTI_REPO.slice(0, MULTI_REPO.indexOf("## Phases")) + `
## Milestone 1 — Ship both services

### Phase 1 — Build API
- **Repository:** \`acme/api\`
- **Depends on:** none

### Phase 2 — Build web
- **Repository:** \`acme/web\`
- **Depends on:** Phase 1

### Phase 3 — Review services [kind: pr-review] [gate: merge]
- **Depends on:** Phase 2

## Milestone 2 — Extend API
- **Integration branch (acme/api):** \`feat/api-next\`

### Phase 4 — Extend API
- **Repository:** \`acme/api\`
- **Depends on:** Phase 3

### Phase 5 — Review API [kind: pr-review]
- **Depends on:** Phase 4
`;

describe("parsePlan — milestones", () => {
  it("collects both milestone sections in document order and parses the merge gate", () => {
    const plan = parsePlan(TWO_MILESTONES);
    expect(plan.status).toBe("awaiting merge");
    expect(plan.phases.map((phase) => [phase.phase, phase.milestone])).toEqual([
      ["1", 1], ["2", 1], ["4", 2], ["3", 2],
    ]);
    expect(plan.phases[1].title).toBe("Review the core");
    expect(plan.phases[1].gate).toBe("merge");
    expect(plan.phases.at(-1)).toMatchObject({ phase: "3", milestone: 2, kind: "pr-review", gate: null });
    expect(plan.milestones).toEqual([
      {
        number: 1, title: "Ship the core", gatePhase: "2",
        integrationBranches: { primary: "feat/first" }, prs: { primary: null }, participants: ["primary"],
      },
      {
        number: 2, title: "Ship the API", gatePhase: null,
        integrationBranches: { primary: "feat/second" }, prs: { primary: null }, participants: ["primary"],
      },
    ]);
    expect(plan.warnings).toEqual([]);
  });

  it("treats legacy Phases as milestone 1 with existing repository branches and PRs", () => {
    const plan = parsePlan(MULTI_REPO);
    expect(plan.phases.map((phase) => phase.milestone)).toEqual([1, 1]);
    expect(plan.milestones).toEqual([{
      number: 1, title: "Phases", gatePhase: null,
      integrationBranches: { "acme/api": "feat/api", "acme/web": "feat/shared" },
      prs: { "acme/api": "https://github.com/acme/api/pull/10", "acme/web": null },
      participants: ["acme/api", "acme/web"],
    }]);
    expect(parsePlan(MULTI_LANE).milestones[0].prs).toEqual({ primary: "https://github.com/acme/widgets/pull/42" });
  });

  it.each([
    ["", { "acme/api": "feat/api", "acme/web": "feat/shared" }],
    ["- **Integration branch:** `feat/milestone`", { "acme/api": "feat/milestone", "acme/web": "feat/milestone" }],
    ["- **Integration branch:** `feat/milestone`\n- **Integration branch (acme/api):** `feat/api-milestone`", { "acme/api": "feat/api-milestone", "acme/web": "feat/milestone" }],
  ])("resolves branch precedence for milestone 1: %s", (header, expected) => {
    const plan = parsePlan(MULTI_REPO_MILESTONES.replace("## Milestone 1 — Ship both services", `## Milestone 1 — Ship both services\n${header}`));
    expect(plan.milestones[0].integrationBranches).toEqual(expected);
    expect(plan.milestones[1]).toMatchObject({
      participants: ["acme/api"], integrationBranches: { "acme/api": "feat/api-next" },
    });
    expect(plan.warnings).toEqual([]);
  });

  it("reads PRs from milestone headers and scopes unqualified PRs to a single participant", () => {
    expect(parsePlan(MULTI_REPO_MILESTONES).milestones[0].prs).toEqual({ "acme/api": null, "acme/web": null });
    const plan = parsePlan(MULTI_REPO_MILESTONES
      .replace("## Milestone 1 — Ship both services", `## Milestone 1 — Ship both services
- **PR:** https://github.com/acme/api/pull/999
- **PR (acme/api):** [#20](https://github.com/acme/api/pull/20)
- **PR (acme/web):** _none yet_`)
      .replace("## Milestone 2 — Extend API", `## Milestone 2 — Extend API
- **PR:** https://github.com/acme/api/pull/21`));
    expect(plan.milestones.map((milestone) => milestone.prs)).toEqual([
      { "acme/api": "https://github.com/acme/api/pull/20", "acme/web": null },
      { "acme/api": "https://github.com/acme/api/pull/21" },
    ]);
    const cleared = parsePlan(TWO_MILESTONES.replace("## Milestone 2 — Ship the API", `## Milestone 2 — Ship the API
- **PR:** https://github.com/acme/api/pull/21
- **PR (primary):** _none yet_`));
    expect(cleared.milestones[1].prs).toEqual({ primary: null });
  });

  it("requires a declared branch for every later milestone participant without global fallback", () => {
    const plan = parsePlan(MULTI_REPO_MILESTONES.replace("- **Integration branch (acme/api):** `feat/api-next`\n", ""));
    expect(plan.milestones[1].integrationBranches).toEqual({ "acme/api": null });
    expect(plan.warnings.join("\n")).toMatch(/Milestone 2.*acme\/api.*[Bb]ranch/);
  });

  it("warns when a repository reuses a branch, allowing the same branch for different repositories", () => {
    const repeated = parsePlan(MULTI_REPO_MILESTONES.replace("`feat/api-next`", "`feat/api`"));
    expect(repeated.warnings.join("\n")).toMatch(/Milestone 2.*acme\/api.*feat\/api.*(?:repeat|reuse)/i);
    const distinct = parsePlan(MULTI_REPO_MILESTONES.replace("`feat/api-next`", "`feat/shared`"));
    expect(distinct.warnings).toEqual([]);
  });

  it.each([
    TWO_MILESTONES.replace("`[gate: merge]`", ""),
    TWO_MILESTONES.replace("## Milestone 2 — Ship the API", `### Phase 6 — Work after the gate
- **Depends on:** Phase 2

## Milestone 2 — Ship the API`),
  ])("warns unless a non-final milestone ends with a gated PR-review", (md) => {
    expect(parsePlan(md).warnings.join("\n")).toMatch(/Milestone 1.*end.*PR-review.*gate.*merge/i);
  });

  it("warns on gate tags on work phases without treating them as milestone gates", () => {
    const plan = parsePlan(TWO_MILESTONES.replace("### Phase 2 — Review the core [kind: pr-review]", "### Phase 2 — Review the core"));
    expect(plan.phases[1].title).toBe("Review the core");
    expect(plan.milestones[0].gatePhase).toBeNull();
    expect(plan.warnings.join("\n")).toMatch(/Phase 2.*gate.*PR-review/i);
  });

  it.each(["none", "Phase 1"])("requires milestone roots to explicitly depend on the previous gate: %s", (dependency) => {
    const plan = parsePlan(TWO_MILESTONES.replace("### Phase 4 — Build the API [lane: A]\n- **Depends on:** Phase 2", `### Phase 4 — Build the API [lane: A]\n- **Depends on:** ${dependency}`));
    expect(plan.phases[2].dependsOn).toEqual(dependency === "none" ? [] : ["1"]);
    expect(plan.warnings.join("\n")).toMatch(/Milestone 2.*(?:root )?Phase 4.*explicit.*gate Phase 2/i);
    expect(plan.warnings.join("\n")).not.toMatch(/Phase 3.*gate Phase 2/i);
  });

  it("walks an implicit first milestone followed by an explicit later milestone", () => {
    const plan = parsePlan(TWO_MILESTONES.replace("## Milestone 1 — Ship the core", "## Phases"));
    expect(plan.milestones.map((milestone) => [milestone.number, milestone.title])).toEqual([[1, "Phases"], [2, "Ship the API"]]);
    expect(plan.phases.map((phase) => phase.phase)).toEqual(["1", "2", "4", "3"]);
    expect(plan.warnings).toEqual([]);
  });

  it("ignores phase-local branch and PR bullets when reading milestone metadata", () => {
    const plan = parsePlan(TWO_MILESTONES.replace("### Phase 1 — Build the core [lane: A]", `### Phase 1 — Build the core [lane: A]
- **Integration branch:** \`feat/phase-only\`
- **PR:** https://github.com/acme/api/pull/999`));
    expect(plan.milestones[0]).toMatchObject({ integrationBranches: { primary: "feat/first" }, prs: { primary: null } });
  });

  it("keeps phase identities unique across milestone sections", () => {
    const plan = parsePlan(TWO_MILESTONES.replace("### Phase 3 — Review the API", "### Phase 1 — Review the API"));
    expect(plan.phases.map((phase) => phase.phase)).toEqual(["1", "2", "4"]);
    expect(plan.warnings.join("\n")).toMatch(/Duplicate Phase 1/);
  });
});

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

describe("parsePlan — legacy Kestral markers", () => {
  it("ignores kestral* header keys, Kestral header lines and Task lines", () => {
    const legacy = MULTI_LANE.replace(
      "daemon: http://localhost:7717\n",
      "daemon: http://localhost:7717\nkestralProject: Widgets\nkestralWorkContextId: doc_9\n",
    )
      .replace("**Status:** in progress\n", "**Status:** in progress\n**Kestral project:** [Widgets](https://k.example/p)\n")
      .replace("- **Depends on:** none\n", "- **Task:** [core - Add core](https://k.example/t1)\n- **Depends on:** none\n");
    const plan = parsePlan(legacy);
    expect(plan.phases.map((p) => p.phase)).toEqual(parsePlan(MULTI_LANE).phases.map((p) => p.phase));
    expect(plan.phases[0].dependsOn).toEqual([]);
    expect(plan.status).toBe("in progress");
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

describe("parsePlan — repositories", () => {
  it("parses repository blocks and shared branch defaults", () => {
    const plan = parsePlan(MULTI_REPO);
    expect(plan.repositories).toEqual([
      {
        slug: "acme/api",
        verify: "npm test",
        integrationBranch: "feat/api",
        pr: "https://github.com/acme/api/pull/10",
      },
      {
        slug: "acme/web",
        verify: "npm run check",
        integrationBranch: "feat/shared",
        pr: null,
      },
    ]);
    expect(plan.phases.map((phase) => phase.repository)).toEqual(["acme/api", "acme/web"]);
  });

  it("normalizes legacy scalar config into an implicit primary repository", () => {
    const plan = parsePlan(MULTI_LANE);
    expect(plan.repositories).toEqual([{
      slug: "primary",
      verify: "npm test",
      integrationBranch: "feat/widget-revamp",
      pr: "https://github.com/acme/widgets/pull/42",
    }]);
    expect(plan.phases.every((phase) => phase.repository === "primary")).toBe(true);
  });

  it("rejects duplicate/unknown repositories and missing verify or phase assignment", () => {
    const plan = parsePlan(MULTI_REPO.replace(
      "## Phases",
      "### `acme/api`\n- **Verify:** `true`\n\n## Phases",
    ));
    expect(plan.warnings.join("\n")).toMatch(/Duplicate repository acme\/api/);

    const invalid = parsePlan(MULTI_REPO
      .replace("- **Verify:** `npm run check`", "- **PR:** _none yet_")
      .replace("- **Repository:** `acme/web`", "- **Repository:** `acme/missing`"));
    expect(invalid.warnings.join("\n")).toMatch(/acme\/web.*Verify/i);
    expect(invalid.warnings.join("\n")).toMatch(/unknown repository.*acme\/missing/i);

    const missing = parsePlan(MULTI_REPO.replace("- **Repository:** `acme/web`\n", ""));
    expect(missing.warnings.join("\n")).toMatch(/Phase 2.*repository/i);
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
  it("captures per-phase fields (branch, verify, doneWhen, notes)", () => {
    const p1 = parsePlan(MULTI_LANE).phases[0];
    expect(p1.suggestedBranch).toBe("feat/widget-core");
    expect(p1.verify).toBe("npm run test:core");
    expect(p1.doneWhen).toBe("core compiles");
    const p3 = parsePlan(MULTI_LANE).phases[2];
    expect(p3.notes).toBe("same PR as Phase 1");
    expect(p3.verify).toBeNull();
  });

  it("recognizes an explicit PR-review phase and preserves document order", () => {
    const phases = parsePlan(EDITABLE_WITH_REVIEW).phases;
    expect(phases.map((phase) => [phase.phase, phase.kind])).toEqual([
      ["1", "work"],
      ["3", "work"],
      ["2", "pr-review"],
    ]);
    expect(phases[2].repository).toBe("all");
  });

  it("accepts a terminal PR-review phase without Repository as canonical", () => {
    const plan = parsePlan(`${MULTI_REPO}\n### Phase 3 — Review pull requests \`[lane: review]\` \`[status: todo]\` \`[kind: pr-review]\`\n- **Depends on:** Phase 2\n`);
    const review = plan.phases.at(-1)!;
    expect(review.kind).toBe("pr-review");
    expect(review.repository).toBe("all");
    expect(plan.warnings.join("\n")).not.toMatch(/Phase 3.*Repository/i);
  });

  it("parses review tier + rounds tags on a PR-review phase (defaults: medium, tier rounds)", () => {
    const tagged = parsePlan(`${MULTI_REPO}\n### Phase 3 — Review pull requests \`[lane: review]\` \`[status: todo]\` \`[kind: pr-review]\` \`[review: max]\` \`[rounds: 2]\`\n- **Depends on:** Phase 2\n`);
    const review = tagged.phases.at(-1)!;
    expect(review.title).toBe("Review pull requests");
    expect(review.review).toEqual({ tier: "max", rounds: 2 });

    const plain = parsePlan(`${MULTI_REPO}\n### Phase 3 — Review pull requests \`[lane: review]\` \`[status: todo]\` \`[kind: pr-review]\`\n- **Depends on:** Phase 2\n`);
    expect(plain.phases.at(-1)!.review).toEqual({ tier: "medium", rounds: null });
    expect(plain.phases[0].review).toBeNull();
  });

  it("warns on an unknown review tier or out-of-range rounds and falls back to defaults", () => {
    const bad = parsePlan(`${MULTI_REPO}\n### Phase 3 — Review \`[lane: review]\` \`[kind: pr-review]\` \`[review: huge]\` \`[rounds: 9]\`\n- **Depends on:** Phase 2\n`);
    expect(bad.phases.at(-1)!.review).toEqual({ tier: "medium", rounds: null });
    expect(bad.warnings.join("\n")).toMatch(/Phase 3.*review tier/i);
    expect(bad.warnings.join("\n")).toMatch(/Phase 3.*rounds/i);
  });

  it("warns when work appears after the last explicit PR-review phase", () => {
    const invalid = EDITABLE_WITH_REVIEW.replace(
      /### Phase 3[\s\S]*?(?=### Phase 2)/,
      "",
    ) + `\n### Phase 4 — Too late \`[lane: A]\` \`[status: todo]\`\n- **Depends on:** Phase 2\n`;
    expect(parsePlan(invalid).warnings.join("\n")).toMatch(/must end with.*PR-review/i);
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

describe("parsePlan — malformed milestone headings", () => {
  const plan = (headings: string[]) => `# Plan

${headings.map((heading, i) => `## ${heading}

### Phase ${i + 1} — Work [lane: A]
- **Depends on:** none
`).join("\n")}`;

  it.each(["Milestone 2: foo", "Milestone 2", "milestone 2 — foo"])("warns on near-miss heading %s", (heading) => {
    const parsed = parsePlan(plan(["Milestone 1 — Core", heading]));
    expect(parsed.warnings.join("\n")).toMatch(new RegExp(`heading "${heading}".*Milestone N — <outcome>`));
  });

  it("warns on duplicate and non-sequential milestone numbers", () => {
    expect(parsePlan(plan(["Milestone 1 — Core", "Milestone 1 — Again"])).warnings.join("\n")).toMatch(/Milestone 1.*duplicate/i);
    expect(parsePlan(plan(["Milestone 1 — Core", "Milestone 3 — Skip"])).warnings.join("\n")).toMatch(/Milestone 3.*expected Milestone 2/i);
  });
});
