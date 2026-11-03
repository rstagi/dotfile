import { expect, test } from "vitest";
import { reduceEvent } from "./state.ts";
import type { CockpitEvent } from "./events.ts";

test("reduction preserves its inputs and tracks the earliest and latest event times", () => {
  const event: CockpitEvent = {
    id: "e1", runId: "r1", timestamp: "2026-11-01T12:00:00Z",
    type: "run.meta", meta: { title: "Build the store", status: "active" },
  };
  const first = reduceEvent(undefined, event);
  const second = reduceEvent(first, {
    ...event, id: "e2", timestamp: "2026-11-01T11:00:00Z", meta: { branch: "feature/store" },
  });
  expect(first.meta).toEqual({ title: "Build the store", status: "active" });
  expect(second.meta).toEqual({ title: "Build the store", status: "active", branch: "feature/store" });
  expect(second.createdAt).toBe("2026-11-01T11:00:00Z");
  expect(second.updatedAt).toBe("2026-11-01T12:00:00Z");
  expect(event.meta).toEqual({ title: "Build the store", status: "active" });
});
