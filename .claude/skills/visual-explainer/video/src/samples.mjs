// Default props for the Remotion Studio preview. Remotion merges input props over these defaults,
// one top-level key at a time. So the samples hold only required keys: a real render replaces
// every one of them, and no sample value can leak into a video.
export const SAMPLE_PLAN = {
  mode: "plan",
  meta: { title: "Sample plan", subtitle: "Studio preview props", date: "2026-10-18" },
  goal: "Show the plan scenes in the Studio.",
  architecture: { summary: "One layer.", layers: [{ name: "Layer", components: [{ name: "Component", role: "Role" }] }] },
  milestones: [{ number: 1, title: "Milestone", phases: [{ number: 1, title: "Phase", kind: "work" }] }],
  decisions: [{ decision: "Decision", why: "Why", rejected: [{ option: "Option", reason: "Reason" }] }],
  risks: [{ risk: "Risk", impact: "low", mitigation: "Mitigation" }],
};

export const SAMPLE_SHIP = {
  mode: "ship",
  meta: { title: "Sample ship", subtitle: "Studio preview props", date: "2026-10-18" },
  summary: "Show the ship scenes in the Studio.",
  delivered: [{ criterion: "Criterion", result: "Result", status: "met" }],
  deviations: [],
  reviews: [{ verdict: "APPROVE", summary: "Summary", validation: [{ command: "npm test", result: "pass" }], findings: [] }],
  handTest: [{ command: "npm start", ref: "src/index.js:1", why: "Why" }],
  rollback: [{ step: "Revert the merge commit." }],
};
