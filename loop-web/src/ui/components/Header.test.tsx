import { describe, it, expect } from "vitest";
import { formatOccupancy } from "./Header.tsx";

describe("formatOccupancy", () => {
  it("passes small counts through", () => { expect(formatOccupancy(0)).toBe("0"); expect(formatOccupancy(999)).toBe("999"); });
  it("rounds thousands to k", () => { expect(formatOccupancy(1000)).toBe("1k"); expect(formatOccupancy(152341)).toBe("152k"); });
});
