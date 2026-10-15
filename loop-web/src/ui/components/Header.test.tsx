import { describe, it, expect } from "vitest";
import { formatOccupancy } from "./Header.tsx";
import { renderToStaticMarkup } from "react-dom/server";
import { Header } from "./Header.tsx";
import type { Snapshot } from "../../model/types.ts";

describe("formatOccupancy", () => {
  it("passes small counts through", () => { expect(formatOccupancy(0)).toBe("0"); expect(formatOccupancy(999)).toBe("999"); });
  it("rounds thousands to k", () => { expect(formatOccupancy(1000)).toBe("1k"); expect(formatOccupancy(152341)).toBe("152k"); });
});

describe("Header repositories", () => {
  it("renders one PR link per repository", () => {
    const snapshot = {
      effort: {
        name: "Split", status: "integrating", integrationBranch: null, updatedAt: null,
        pr: null, runId: "r", repositories: [
          { slug: "acme/api", integrationBranch: "feat/api", verify: "test", pr: { url: "https://github.com/acme/api/pull/1" } },
          { slug: "acme/web", integrationBranch: "feat/web", verify: "test", pr: { url: "https://github.com/acme/web/pull/2" } },
        ],
      },
      subOrch: null,
      loopActive: false,
    } as unknown as Snapshot;
    const html = renderToStaticMarkup(<Header snapshot={snapshot} conn="live" />);
    expect(html).toContain("acme/api #1");
    expect(html).toContain("acme/web #2");
    expect(html).not.toContain("Jev");
  });
});

describe("Header Jev status", () => {
  it("shows mode, count, and fallback count", () => {
    const snapshot = {
      effort: { name: "Jev", status: null, integrationBranch: null, updatedAt: null, pr: null, runId: "r", repositories: [] },
      subOrch: null, loopActive: false,
      jev: { mode: "shadow", count: 3, fallbackCount: 1 },
    } as unknown as Snapshot;
    const html = renderToStaticMarkup(<Header snapshot={snapshot} conn="live" />);
    expect(html).toContain("Jev shadow · 3 decisions · 1 fallback");
  });
});

describe("Header milestones", () => {
  const snap = (effort: object) => ({
    effort: { name: "M", status: "awaiting merge", integrationBranch: null, updatedAt: null, pr: null, runId: "r", repositories: [], ...effort },
    subOrch: null, loopActive: false,
  }) as unknown as Snapshot;

  it("shows M<n>/<total> and status for multi-milestone plans", () => {
    const html = renderToStaticMarkup(<Header snapshot={snap({ milestone: 2, milestoneTotal: 3 })} conn="live" />);
    expect(html).toContain("M2/3");
    expect(html).toContain("awaiting merge");
  });

  it("omits the milestone pill for single-milestone plans", () => {
    const html = renderToStaticMarkup(<Header snapshot={snap({ milestone: 1, milestoneTotal: 1 })} conn="live" />);
    expect(html).not.toContain("M1/1");
  });

  it("flags awaiting merge from effort state even if the plan status lags", () => {
    const html = renderToStaticMarkup(<Header snapshot={snap({ status: "integrating", milestone: 1, milestoneTotal: 2, awaitingMerge: { milestone: 1, prs: {} } })} conn="live" />);
    expect(html).toContain("awaiting merge");
  });
});
