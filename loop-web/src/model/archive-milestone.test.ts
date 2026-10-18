import { readFileSync } from "node:fs";
import { describe, it, expect } from "vitest";
import { archiveMilestone, attachRetro } from "./archive-milestone.ts";
import { parsePlan } from "./parse-plan.ts";

const fixture = (name: string) => readFileSync(new URL(`../../../tests/fixtures/milestones/${name}`, import.meta.url), "utf8");
const PLAN = fixture("plan.md");

describe("archiveMilestone", () => {
  it("moves a shipped milestone's phase blocks and log entries to its archive file and leaves a stub", () => {
    expect(archiveMilestone(PLAN, 1)).toEqual({
      status: "archived",
      plan: fixture("archived-m1/plan.md"),
      archive: fixture("archived-m1/milestones/m1.md"),
      archiveFile: "milestones/m1.md",
      prs: { "acme/api": "https://github.com/acme/api/pull/1", "acme/web": "https://github.com/acme/web/pull/2" },
    });
  });

  it("writes a custom one-line outcome into the stub", () => {
    const result = archiveMilestone(PLAN, 1, { outcome: "The API and the shell are live" });
    expect(result.status === "archived" && result.plan).toContain("- **Outcome:** The API and the shell are live\n");
  });

  it("is a no-op on a milestone that is already archived", () => {
    expect(archiveMilestone(fixture("archived-m1/plan.md"), 1)).toEqual({ status: "already", archiveFile: "milestones/m1.md" });
  });

  it("refuses while a phase of the milestone is not done", () => {
    expect(archiveMilestone(PLAN, 2)).toEqual({
      status: "refused",
      reason: "milestone 2 has phases that are not done: Phase 4 (in-progress), Phase 10 (todo), Phase 5 (todo)",
    });
  });

  it("refuses a milestone without a merge gate, an unknown milestone, and a plan without milestones", () => {
    const allDone = PLAN.replace(/\[status: (todo|in-progress)\]/g, "[status: done]");
    expect(archiveMilestone(allDone, 3)).toEqual({ status: "refused", reason: "milestone 3 has no [gate: merge] phase" });
    expect(archiveMilestone(PLAN, 4)).toEqual({ status: "refused", reason: "plan has no milestone 4" });
    expect(archiveMilestone("# P\n\n## Phases\n\n### Phase 1 — W `[lane: A]` `[status: done]`\n", 1))
      .toEqual({ status: "refused", reason: "plan has no milestone 1" });
  });

  it("archives milestones in sequence; readers still see every phase as before", () => {
    const shipped = PLAN
      .replace(/\[status: (todo|in-progress)\](.*\n- \*\*Repository:\*\* `acme\/web`)/g, "[status: done]$2")
      .replace("`[status: todo]` `[kind: pr-review]` `[gate: merge]`", "`[status: done]` `[kind: pr-review]` `[gate: merge]`")
      .replace("- **PR (acme/web):** _none yet_", "- **PR (acme/web):** https://github.com/acme/web/pull/3");
    const first = archiveMilestone(shipped, 1);
    const second = first.status === "archived" ? archiveMilestone(first.plan, 2) : first;
    if (second.status !== "archived") throw new Error(JSON.stringify(second));
    const shape = (text: string) => parsePlan(text).phases.map((phase) => [phase.phase, phase.milestone, phase.status, phase.kind]);
    expect(shape(second.plan)).toEqual(shape(shipped));
    expect(parsePlan(second.plan).warnings).toEqual([]);
    expect(second.plan).toContain("- **Phases (all done):** 4, 10, 5\n- **Gate:** Phase 5\n- **Archive:** [milestones/m2.md](milestones/m2.md)");
    expect(second.archive).toContain("- 2026-10-04 — Phase 4 claimed (lane A, codex)\n- 2026-10-04 — Phase 10 added to milestone 2");
    expect(second.plan).toContain("## Progress log\n- 2026-10-01 — Plan drafted (Claude Code, main).\n- 2026-10-04 — Phases 4 and 6 revised");
  });

  it("puts a pending retro first, then the raw log; the stub takes the retro Outcome", () => {
    expect(archiveMilestone(PLAN, 1, { retro: fixture("m1.retro.md") })).toEqual({
      status: "archived",
      plan: fixture("archived-m1-retro/plan.md"),
      archive: fixture("archived-m1-retro/milestones/m1.md"),
      archiveFile: "milestones/m1.md",
      prs: { "acme/api": "https://github.com/acme/api/pull/1", "acme/web": "https://github.com/acme/web/pull/2" },
    });
  });

  it("prefers --outcome over the retro Outcome, and the title when the retro has no Outcome", () => {
    const stubOutcome = (result: ReturnType<typeof archiveMilestone>) =>
      result.status === "archived" ? result.plan.match(/^- \*\*Outcome:\*\* (.*)$/m)?.[1] : result.status;
    expect(stubOutcome(archiveMilestone(PLAN, 1, { retro: fixture("m1.retro.md"), outcome: "Shipped" }))).toBe("Shipped");
    expect(stubOutcome(archiveMilestone(PLAN, 1, { retro: "### Lessons\n1. Keep it small.\n" }))).toBe("Core API and web shell ship");
  });

  it("gives a retro without its heading the `## Analysis and recap` heading", () => {
    const result = archiveMilestone(PLAN, 1, { retro: "- **Outcome:** Shipped.\n" });
    expect(result.status === "archived" && result.archive).toContain(
      "without changes.\n\n## Analysis and recap\n- **Outcome:** Shipped.\n\n## Raw log\n\n## Milestone 1 — Core API and web shell ship\n");
  });

  it("attaches a pending retro to an archived milestone and updates the stub Outcome", () => {
    const attached = attachRetro(fixture("archived-m1/plan.md"), fixture("archived-m1/milestones/m1.md"), 1, fixture("m1.retro.md"));
    expect(attached).toEqual({
      status: "attached",
      plan: fixture("archived-m1-retro/plan.md"),
      archive: fixture("archived-m1-retro/milestones/m1.md"),
      archiveFile: "milestones/m1.md",
    });
  });

  it("replaces an attached retro with a newer one and keeps the raw log", () => {
    const newer = fixture("m1.retro.md").replace("1. Run the web build", "1. Fail the web build on warnings. 2. Run the web build");
    const attached = attachRetro(fixture("archived-m1-retro/plan.md"), fixture("archived-m1-retro/milestones/m1.md"), 1, newer);
    if (attached.status !== "attached") throw new Error(JSON.stringify(attached));
    expect(attached.archive).toBe(fixture("archived-m1-retro/milestones/m1.md").replace("1. Run the web build", "1. Fail the web build on warnings. 2. Run the web build"));
    expect(attached.plan).toBe(fixture("archived-m1-retro/plan.md"));
  });

  it("refuses to attach a retro to a milestone that is not archived", () => {
    expect(attachRetro(PLAN, "", 1, fixture("m1.retro.md"))).toEqual({ status: "refused", reason: "milestone 1 is not archived" });
    expect(attachRetro(PLAN, "", 4, fixture("m1.retro.md"))).toEqual({ status: "refused", reason: "plan has no milestone 4" });
  });

  it("refuses when a participant has no gate PR link", () => {
    const noPr = PLAN.replace("- **PR (acme/web):** https://github.com/acme/web/pull/2", "- **PR (acme/web):** _none yet_");
    expect(archiveMilestone(noPr, 1)).toEqual({ status: "refused", reason: "milestone 1 has no PR link for acme/web" });
  });
});
