// Default props for the Remotion Studio preview: a two-node graph and a two-stop tour. Remotion
// merges input props over these defaults, one top-level key at a time. A real render replaces every
// key, so no sample value can leak into a video.
const KINDS = {
  goal: { label: "Goal", color: "accent" }, phase: { label: "Phases", color: "accent" },
  summary: { label: "Outcome", color: "accent" }, step: { label: "Hand test", color: "accent" },
};

const sample = (mode, root, other) => ({
  mode,
  meta: { title: `Sample ${mode}`, subtitle: "Studio preview props", date: "2026-10-18" },
  kinds: KINDS,
  graph: {
    mode, title: `Sample ${mode}`, width: 760, height: 400,
    groups: [{ id: "group-a", name: "Group", label: "Group", kinds: [other], x: 400, y: 120, w: 296, h: 174 }],
    nodes: [
      { id: root, kind: root, group: null, kicker: mode, title: `Sample ${mode}`, summary: "Studio preview", badges: [], tone: null, lines: [`Sample ${mode}`], sub: ["Studio preview"], facts: [{ label: "Goal", text: "Studio preview", key: true }], x: 48, y: 120, w: 340, h: 170 },
      { id: `${other}-1`, kind: other, group: "group-a", kicker: other, title: "Item", summary: "Item", badges: [], tone: null, lines: ["Item"], facts: [{ label: "Fact", text: "A fact", key: true }], x: 428, y: 182, w: 240, h: 84 },
    ],
    edges: [{ from: root, to: `${other}-1`, type: "contains", d: "M388 205 L428 224" }],
  },
  stops: [
    { id: "stop-1", title: "Overview", text: "", nodes: [root, `${other}-1`], overview: true },
    { id: "stop-2", title: "Item", text: "", nodes: [`${other}-1`], overview: false },
  ],
});

export const SAMPLE_PLAN = sample("plan", "goal", "phase");
export const SAMPLE_SHIP = sample("ship", "summary", "step");
