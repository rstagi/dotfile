import { expect, test, vi } from "vitest";
import type { CockpitEvent, RunState } from "../core/events.ts";
import { reduceEvent } from "../core/state.ts";
import { processIntents } from "./processor.ts";

const timestamp = "2026-11-01T20:00:00.000Z";
const phase: CockpitEvent = {
  id: "start", runId: "run", timestamp, type: "phase.start",
  phase: { id: "p1", title: "Repair session replay" },
};
function action(id: string, summary: string, sessionId = "s1"): CockpitEvent {
  return { id: `event-${id}`, runId: "run", timestamp, sessionId, phaseId: "p1", type: "action",
    action: { id, summary, tool: "Read", sessionId, phaseId: "p1" } };
}
function replay(events: CockpitEvent[], previous?: RunState): RunState {
  return events.reduce<RunState | undefined>(reduceEvent, previous)!;
}

test("names new actions with goals and reasons, then skips unchanged sessions after replay", async () => {
  const incoming = [phase, action("a1", "Read the replay reducer to find why phases disappear")];
  const model = vi.fn().mockResolvedValue(JSON.stringify({ intents: [
    { text: "Find why replay loses phases", reason: "Restore the complete session history", actionIds: ["a1"] },
  ] }));
  const events = await processIntents(undefined, incoming, model);
  expect(events).toEqual([expect.objectContaining({ type: "intent.upsert", intent: expect.objectContaining({
    text: "Find why replay loses phases", reason: "Restore the complete session history",
    actionIds: ["a1"], phaseId: "p1", status: "live",
  }) })]);
  expect(model).toHaveBeenCalledTimes(1);
  expect(model.mock.calls[0][0]).toBe("haiku");
  expect(model.mock.calls[0][1]).toContain("Read the replay reducer to find why phases disappear");
  const restored = replay([...incoming, ...events]);
  expect(await processIntents(restored, [], model)).toEqual([]);
  expect(await processIntents(restored, [incoming[1]], model)).toEqual([]);
  expect(model).toHaveBeenCalledTimes(1);
});

test("keeps two live intents and merges older goals when the window advances or the phase ends", async () => {
  const incoming = [phase, action("a1", "Inspect the replay reducer"), action("a2", "Repair media access"), action("a3", "Test server ports")];
  const model = vi.fn().mockResolvedValueOnce(JSON.stringify({ intents: [
    { text: "Inspect replay to find missing phases", reason: "Restore history", actionIds: ["a1"] },
    { text: "Restrict media paths to protect local files", reason: "Protect private files", actionIds: ["a2"] },
    { text: "Check port fallback to preserve startup", reason: "Start when the default port is busy", actionIds: ["a3"] },
  ] })).mockResolvedValueOnce(JSON.stringify({ text: "Restore replay history from recorded phases" }))
    .mockResolvedValueOnce(JSON.stringify({ text: "Restore safe and reliable session playback" }));
  const events = await processIntents(undefined, incoming, model);
  const state = replay([...incoming, ...events]);
  expect(state.intents.filter((intent) => intent.status === "live").map((intent) => intent.actionIds)).toEqual([["a2"], ["a3"]]);
  expect(state.groups).toEqual([expect.objectContaining({
    text: "Restore replay history from recorded phases", intentIds: [state.intents[0].id], phaseId: "p1",
  })]);
  expect(model.mock.calls.map(([model]) => model)).toEqual(["haiku", "sonnet"]);
  expect(model.mock.calls[1][1]).toContain(state.intents[0].text);
  const end: CockpitEvent = { ...phase, id: "end", type: "phase.end", phase: { id: "p1" } };
  const closed = await processIntents(state, [end], model);
  const finished = replay([end, ...closed], state);
  expect(finished.intents.every((intent) => intent.status === "earlier")).toBe(true);
  expect(finished.groups).toEqual([expect.objectContaining({
    id: state.groups[0].id, text: "Restore safe and reliable session playback", intentIds: state.intents.map((intent) => intent.id),
  })]);
  expect(model.mock.calls.map(([model]) => model)).toEqual(["haiku", "sonnet", "sonnet"]);
  expect(await processIntents(finished, [end], model)).toEqual([]);
  expect(model).toHaveBeenCalledTimes(3);
});

test("infers a plain chat phase from its named goal and ends it when the session becomes idle", async () => {
  const plain = action("plain1", "Read failing replay tests");
  if (plain.type !== "action") throw new Error("The fixture must be an action.");
  delete plain.phaseId;
  delete plain.action.phaseId;
  const model = vi.fn().mockResolvedValueOnce(JSON.stringify({ intents: [
    { text: "Repair replay so earlier phases remain visible", reason: "Keep a complete history", actionIds: ["plain1"] },
  ] })).mockResolvedValueOnce(JSON.stringify({ text: "Preserve replay history for session review" }));
  const events = await processIntents(undefined, [plain], model);
  const start = events.find((event) => event.type === "phase.start");
  expect(start).toMatchObject({ type: "phase.start", sessionId: "s1", phase: {
    inferred: true, title: "Repair replay so earlier phases remain visible",
  } });
  const state = replay([plain, ...events]);
  expect(state.intents[0].phaseId).toBe(state.phases[0].id);
  const idle: CockpitEvent = { id: "idle", runId: "run", timestamp, sessionId: "s1", type: "run.meta", meta: { status: "idle" } };
  const ended = await processIntents(state, [idle], model);
  expect(ended).toContainEqual(expect.objectContaining({ type: "phase.end", phase: expect.objectContaining({ id: state.phases[0].id, inferred: true }) }));
  const finished = replay([idle, ...ended], state);
  expect(finished.phases[0].status).toBe("done");
  expect(finished.intents[0].status).toBe("earlier");
  expect(model.mock.calls.map(([model]) => model)).toEqual(["haiku", "sonnet"]);
  expect(await processIntents(finished, [idle], model)).toEqual([]);
  const next = structuredClone(plain);
  next.id = "event-plain2";
  next.action.id = "plain2";
  const fallback = vi.fn().mockRejectedValue(new Error("Unavailable"));
  const resumed = replay([next, ...await processIntents(finished, [next], fallback)], finished);
  expect(resumed.phases).toHaveLength(2);
  expect(resumed.phases[1]).toMatchObject({ inferred: true, status: "running" });
});

test("updates a live goal without losing its actions or renaming another session", async () => {
  const first = [phase, action("a1", "Read replay tests")];
  const initial = await processIntents(undefined, first, async () => JSON.stringify({ intents: [
    { text: "Inspect replay to locate the defect", reason: "Restore history", actionIds: ["a1"] },
  ] }));
  const state = replay([...first, ...initial]);
  const model = vi.fn().mockResolvedValue(JSON.stringify({ intents: [
    { id: state.intents[0].id, text: "Repair replay so phases remain visible", reason: "Restore history", actionIds: ["a2"] },
  ] }));
  const input = [action("a2", "Repair replay tests"), { id: "other-meta", runId: "run", timestamp,
    sessionId: "s2", type: "run.meta", meta: { title: "Another session" } } as CockpitEvent];
  const result = replay([...input, ...await processIntents(state, input, model)], state);
  expect(result.intents).toEqual([expect.objectContaining({
    id: state.intents[0].id, text: "Repair replay so phases remain visible", actionIds: ["a1", "a2"], status: "live",
  })]);
  expect(model.mock.calls.map(([model]) => model)).toEqual(["haiku"]);
});

test.each([
  ["a rejected call", undefined],
  ["invalid JSON", "no JSON"],
  ["an unknown action", JSON.stringify({ intents: [{ text: "Invent a goal", reason: "Unknown", actionIds: ["unknown"] }] })],
  ["an omitted action", JSON.stringify({ intents: [{ text: "Partial goal", reason: "Partial", actionIds: ["a1"] }] })],
  ["a duplicate action", JSON.stringify({ intents: [{ text: "Duplicate goal", reason: "Duplicate", actionIds: ["a1", "a1", "a2"] }] })],
  ["an unknown intent", JSON.stringify({ intents: [{ id: "invented", text: "Wrong goal", reason: "Wrong", actionIds: ["a1", "a2"] }] })],
])("uses copied rule names after %s and groups all members when the phase ends", async (_label, response) => {
  const model = vi.fn(async () => { if (response === undefined) throw new Error("Unavailable"); return response; });
  const input = [phase, action("a1", "Inspect replay tests to find missing phases"), action("a2", "Inspect replay tests to find missing phases")];
  const events = await processIntents(undefined, input, model);
  const state = replay([...input, ...events]);
  expect(state.intents).toHaveLength(1);
  expect(state.intents[0]).toMatchObject({ actionIds: ["a1", "a2"], reason: "Repair session replay", status: "live" });
  expect(state.intents[0].text).toContain("Inspect replay tests");
  const end: CockpitEvent = { ...phase, id: "end", type: "phase.end", phase: { id: "p1" } };
  const derived = await processIntents(state, [end], async () => { throw new Error("Unavailable"); });
  const result = replay([end, ...derived], state);
  expect(result.groups[0].intentIds).toEqual([state.intents[0].id]);
  expect(result.groups[0].text).toContain("replay");
  expect(result.intents[0].status).toBe("earlier");
});

test("uses a running planned phase when the action has no phase ID", async () => {
  const event = action("planned", "Inspect the reducer");
  if (event.type !== "action") throw new Error("The fixture must be an action.");
  delete event.phaseId;
  delete event.action.phaseId;
  const result = await processIntents(undefined, [phase, event], async () => JSON.stringify({ intents: [
    { text: "Inspect replay to find the defect", reason: "Restore history", actionIds: ["planned"] },
  ] }));
  expect(result).toEqual([expect.objectContaining({ type: "intent.upsert", intent: expect.objectContaining({ phaseId: "p1" }) })]);
});

test("orders the live window by action order when the model returns names in reverse order", async () => {
  const input = [phase, action("a1", "Inspect replay"), action("a2", "Repair media"), action("a3", "Check ports")];
  const model = vi.fn().mockResolvedValueOnce(JSON.stringify({ intents: [
    { text: "Check ports to keep startup available", reason: "Keep startup available", actionIds: ["a3"] },
    { text: "Repair media to protect files", reason: "Protect files", actionIds: ["a2"] },
    { text: "Inspect replay to restore history", reason: "Restore history", actionIds: ["a1"] },
  ] })).mockResolvedValueOnce(JSON.stringify({ text: "Restore recorded history" }));
  const result = replay([...input, ...await processIntents(undefined, input, model)]);
  expect(result.intents.filter((intent) => intent.status === "live").map((intent) => intent.actionIds)).toEqual([["a2"], ["a3"]]);
  expect(result.groups[0].intentIds).toEqual([result.intents.find((intent) => intent.actionIds.includes("a1"))!.id]);
});
