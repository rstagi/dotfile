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
    } as Snapshot;
    const html = renderToStaticMarkup(<Header snapshot={snapshot} conn="live" />);
    expect(html).toContain("acme/api #1");
    expect(html).toContain("acme/web #2");
  });
});
