// Concept graph: validated props -> nodes, edges, groups and one fixed layout. Zero dependencies.
// The graph page draws this graph with these positions.
//
// Node ids are stable: the same props always give the same ids and the same positions.
//   plan: goal, milestone-<n>, phase-<n>, layer-<i>, component-<slug>, decision-<i>, risk-<i>
//   ship: summary, process, change-<i>, phase-<n> (delivered rows of phase n), delivered-<i> (a row
//         without a phase), deviation-<i>, setback-<i>, review-<i>, finding-<slug>, step-<i>, rollback-<i>
// <n> is the number in the props, <i> is the 1-based position in its list, <slug> is the name in
// lowercase with each run of other characters as "-". Group ids are group-<name> and "all".

export const NODE = { w: 240, h: 84 };
export const ROOT = { w: 340, h: 170 };
// Characters for one line of a node title (15 px), of the root title (22 px) and of the root
// summary (13.5 px). A kicker (11.5 px capitals) has one line.
export const LINE_CHARS = { node: 26, root: 24, sub: 40 };
const KICKER_CHARS = 26;
const SUMMARY_CHARS = 220;
const GAP = 28;
const HEAD_GAP = 20;
const SECTION_GAP = 36;
const GROUP_PAD = 28;
const GROUP_LABEL = 34;
const GROUP_GAP = 72;
const MARGIN = 48;
// Group placement: rays from the root, a step along each ray, and the screen shape to aim for.
const RAYS = 48;
const RAY_STEP = 12;
const SCREEN = 16 / 9;
const ELLIPSIS = "…";

// The kinds of each mode, in legend order. `color` names a color token of the page.
export const KINDS = {
  goal: { label: "Goal", color: "accent" },
  milestone: { label: "Milestones", color: "ink" },
  phase: { label: "Phases", color: "accent" },
  layer: { label: "Layers", color: "muted" },
  component: { label: "Components", color: "info" },
  decision: { label: "Decisions", color: "ok" },
  risk: { label: "Risks", color: "bad" },
  summary: { label: "Outcome", color: "accent" },
  process: { label: "Process", color: "muted" },
  change: { label: "Changes", color: "info" },
  delivered: { label: "Delivered", color: "ok" },
  deviation: { label: "Deviations", color: "warn" },
  setback: { label: "Setbacks", color: "warn" },
  review: { label: "Reviews", color: "accent" },
  finding: { label: "Findings", color: "bad" },
  step: { label: "Hand test", color: "accent" },
  rollback: { label: "Rollback", color: "warn" },
};
export const MODE_KINDS = {
  plan: ["goal", "milestone", "phase", "layer", "component", "decision", "risk"],
  ship: ["summary", "process", "change", "delivered", "deviation", "setback", "review", "finding", "step", "rollback"],
};
// The groups of each mode, in reading order. The root node belongs to no group.
const GROUPS = {
  plan: [["timeline", "Timeline"], ["architecture", "Architecture"], ["decisions", "Decisions"], ["risks", "Risks"]],
  ship: [["changes", "Changes"], ["delivered", "Delivered"], ["deviations", "Deviations"], ["setbacks", "Setbacks and hiccups"],
    ["review", "Review"], ["hand-test", "Hand test"], ["rollback", "Rollback"]],
};
// The label of an edge type, seen from its source and from its target.
export const EDGE_LABELS = {
  contains: ["Contains", "Part of"],
  depends: ["Unblocks", "Depends on"],
  flow: ["Flows to", "Flows from"],
  touches: ["Touches", "Touched by"],
  affects: ["Affects", "Affected by"],
  mitigates: ["Mitigates", "Mitigated by"],
  delivers: ["Delivers", "Delivered by"],
  concerns: ["Concerns", "Deviations"],
  during: ["During", "Setbacks"],
  about: ["About", "Findings"],
  checks: ["Checks", "Checked by"],
  then: ["Next", "Previous"],
};

const IMPACT_ORDER = { high: 0, medium: 1, low: 2 };
const SEVERITIES = ["blocker", "major", "minor", "nit", "style"];
const STATUS_RANK = { met: 0, pending: 1, partial: 2, missed: 3 };
const TONE = {
  status: { new: "ok", changed: "warn", existing: "muted", removed: "bad" },
  level: { high: "bad", medium: "warn", low: "ok" },
  delivered: { met: "ok", pending: "muted", partial: "warn", missed: "bad" },
  verdict: { APPROVE: "ok", COMMENT: "warn", REQUEST_CHANGES: "bad" },
  severity: { blocker: "bad", major: "warn", minor: "info", nit: "muted", style: "muted" },
};
const SETBACK_TONE = {
  "verify-fail": "bad", timeout: "bad", "merge-conflict": "bad", retry: "warn", "rate-limit": "warn",
  "model-escalation": "warn", environment: "warn", question: "info", hil: "info", "review-round": "info",
  "review-dispute": "info", replan: "info", other: "muted",
};
const VERDICT_LABEL = { APPROVE: "Approved", COMMENT: "Comment", REQUEST_CHANGES: "Changes requested" };
const PROCESS_STATS = [
  ["phases", "phases"], ["attempts", "attempts"], ["retries", "retries"], ["escalations", "escalations"],
  ["questions", "questions"], ["hil", "HIL"], ["mergeConflicts", "merge conflicts"], ["reviewRounds", "review rounds"],
  ["fixRounds", "fix rounds"], ["wallTime", "wall time"],
];

// buildGraph(props) -> { mode, title, nodes, edges, groups, width, height } for props that passed
// the schema and relationErrors. Each node: { id, kind, group, code, kicker, title, summary, tone,
// badges, facts, lines, x, y, w, h }. Each edge: { from, to, type, label?, d }. Each group:
// { id, label, kinds, x, y, w, h }.
export function buildGraph(props) {
  const model = props.mode === "plan" ? planModel(props) : shipModel(props);
  const graph = { mode: props.mode, title: props.meta.title, subtitle: props.meta.subtitle, nodes: model.nodes, edges: model.edges, groups: [] };
  layout(graph, model.blocks);
  return graph;
}

// relationErrors(props) -> [{ path, message }] for props that passed the schema: each relation
// field id that names no item, each ambiguous name, and each repeated phase or milestone number.
export function relationErrors(props) {
  const errors = [];
  const add = (path, message) => errors.push({ path, message });
  if (props.mode === "plan") {
    const phases = new Set();
    const milestones = new Set();
    const components = countBy(props.architecture.layers.flatMap((l) => l.components.map((c) => c.name)));
    const phaseNumbers = new Set(props.milestones.flatMap((m) => m.phases.map((p) => p.number)));
    const component = (at, name) => {
      if (!components.has(name)) add(at, `no component ${JSON.stringify(name)} in the architecture`);
      else if (components.get(name) > 1) add(at, `${JSON.stringify(name)} names more than one component; give each component a different name`);
    };
    const phase = (at, n) => phaseNumbers.has(n) || add(at, `no phase ${n} in the milestones`);
    props.milestones.forEach((m, mi) => {
      if (milestones.has(m.number)) add(`$.milestones[${mi}].number`, `milestone ${m.number} appears more than once`);
      milestones.add(m.number);
      m.phases.forEach((p, pi) => {
        const at = `$.milestones[${mi}].phases[${pi}]`;
        if (phases.has(p.number)) add(`${at}.number`, `phase ${p.number} appears more than once`);
        phases.add(p.number);
        (p.components ?? []).forEach((name, i) => component(`${at}.components[${i}]`, name));
      });
    });
    props.decisions.forEach((d, di) => {
      (d.phases ?? []).forEach((n, i) => phase(`$.decisions[${di}].phases[${i}]`, n));
      (d.components ?? []).forEach((name, i) => component(`$.decisions[${di}].components[${i}]`, name));
    });
    props.risks.forEach((r, ri) => {
      (r.mitigatedBy ?? []).forEach((n, i) => phase(`$.risks[${ri}].mitigatedBy[${i}]`, n));
      (r.components ?? []).forEach((name, i) => component(`$.risks[${ri}].components[${i}]`, name));
    });
    return errors;
  }
  const delivered = new Set(props.delivered.map((d) => d.phase).filter((n) => n !== undefined));
  const areas = countBy((props.changes ?? []).map((c) => c.area));
  const phase = (at, n) => delivered.has(n) || add(at, `no delivered row has phase ${n}`);
  const change = (at, area) => {
    if (!areas.has(area)) add(at, `no change with the area ${JSON.stringify(area)}`);
    else if (areas.get(area) > 1) add(at, `${JSON.stringify(area)} is the area of more than one change; give each change a different area`);
  };
  (props.changes ?? []).forEach((c, ci) => (c.phases ?? []).forEach((n, i) => phase(`$.changes[${ci}].phases[${i}]`, n)));
  props.deviations.forEach((d, di) => (d.phases ?? []).forEach((n, i) => phase(`$.deviations[${di}].phases[${i}]`, n)));
  props.reviews.forEach((r, ri) => r.findings.forEach((f, fi) => {
    const at = `$.reviews[${ri}].findings[${fi}]`;
    (f.phases ?? []).forEach((n, i) => phase(`${at}.phases[${i}]`, n));
    (f.changes ?? []).forEach((area, i) => change(`${at}.changes[${i}]`, area));
  }));
  props.handTest.forEach((s, si) => (s.phases ?? []).forEach((n, i) => phase(`$.handTest[${si}].phases[${i}]`, n)));
  return errors;
}

// groupIds(graph) -> [{ id, label }]: the focus ids that name a whole group, "all" first.
export function groupIds(graph) {
  return [{ id: "all", label: "The whole graph" }, ...graph.groups.map((g) => ({ id: g.id, label: g.label }))];
}

// resolveFocus(graph, focus) -> { nodes, overview }: the node ids that a list of focus ids names.
// "all" is the overview: every node. A group id names the nodes of the group. Unknown ids are skipped.
export function resolveFocus(graph, focus) {
  if (focus.includes("all")) return { nodes: graph.nodes.map((n) => n.id), overview: true };
  const ids = new Set();
  for (const id of focus) {
    const group = graph.groups.find((g) => g.id === id);
    if (group) graph.nodes.filter((n) => n.group === id).forEach((n) => ids.add(n.id));
    else if (graph.nodes.some((n) => n.id === id)) ids.add(id);
  }
  return { nodes: graph.nodes.filter((n) => ids.has(n.id)).map((n) => n.id), overview: false };
}

// slug(text) -> lowercase letters and digits, each other run as one "-".
export function slug(text) {
  return String(text).toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "item";
}

// wrap(text, perLine, maxLines) -> plain lines at word boundaries. Backticks go. A cut ends in "…".
export function wrap(text, perLine, maxLines) {
  const words = String(text).replaceAll("`", "").replace(/\s+/g, " ").trim().split(" ").filter(Boolean)
    .flatMap((word) => (word.length <= perLine ? [word] : word.match(new RegExp(`.{1,${perLine}}`, "gu"))));
  const lines = [];
  for (const word of words) {
    const last = lines.at(-1);
    if (last !== undefined && last.length + 1 + word.length <= perLine) lines[lines.length - 1] = `${last} ${word}`;
    else lines.push(word);
  }
  if (lines.length <= maxLines) return lines.length ? lines : [""];
  const kept = lines.slice(0, maxLines);
  kept[maxLines - 1] = clipText(`${kept[maxLines - 1]} ${lines[maxLines]}`, perLine);
  return kept;
}

// clipText(text, max) -> one line of at most `max` characters, cut at a word boundary with "…".
export function clipText(text, max) {
  const flat = String(text).replace(/\s+/g, " ").trim();
  if (flat.length <= max) return flat;
  const hard = flat.slice(0, max - 1);
  const space = hard.lastIndexOf(" ");
  return `${(space > max / 2 ? hard.slice(0, space) : hard).trimEnd()}${ELLIPSIS}`;
}

// ---- plan ----

function planModel(p) {
  const g = graphBuilder();
  const phaseIds = new Map();
  // componentIds: name -> id, for relations. nodeIds: component object -> id, unique also for a repeated name.
  const componentIds = new Map();
  const nodeIds = new Map();
  const phases = p.milestones.flatMap((m) => m.phases);
  phases.forEach((ph) => phaseIds.set(ph.number, `phase-${ph.number}`));
  const used = new Set();
  p.architecture.layers.forEach((l) => l.components.forEach((c) => {
    const id = unique(`component-${slug(c.name)}`, used);
    if (!componentIds.has(c.name)) componentIds.set(c.name, id);
    nodeIds.set(c, id);
  }));
  const flows = p.architecture.flows ?? [];

  g.node({
    id: "goal", kind: "goal", group: null, kicker: `Plan${p.meta.milestone ? ` · milestone ${p.meta.milestone}` : ""}`,
    title: p.meta.title, summary: p.goal, root: true,
    facts: [
      fact("Goal", p.goal, true), fact("Subtitle", p.meta.subtitle), fact("Architecture", p.architecture.summary),
      flows.length && { label: "Flows", list: flows.map((f) => `${f.from} → ${f.to}: ${f.label}`) },
      { label: "Size", text: `${p.milestones.length} ${plural(p.milestones.length, "milestone")}, ${phases.length} ${plural(phases.length, "phase")}, ${p.decisions.length} ${plural(p.decisions.length, "decision")}, ${p.risks.length} ${plural(p.risks.length, "risk")}` },
      ...metaFacts(p.meta),
    ],
  });

  for (const m of p.milestones) {
    const id = `milestone-${m.number}`;
    const lanes = [...new Set(m.phases.map((ph) => ph.lane).filter(Boolean))];
    g.node({
      id, kind: "milestone", group: "group-timeline", code: `M${m.number}`, kicker: `Milestone ${m.number} · ${m.phases.length} ${plural(m.phases.length, "phase")}`,
      title: m.title, summary: m.outcome ?? `${m.phases.length} ${plural(m.phases.length, "phase")}`,
      facts: [fact("Title", m.title), fact("Outcome", m.outcome, true), { label: "Phases", list: m.phases.map((ph) => `P${ph.number} ${ph.title}`) },
        lanes.length && fact("Lanes", lanes.join(", "))],
    });
    g.edge("goal", id, "contains");
    for (const ph of m.phases) {
      const pid = phaseIds.get(ph.number);
      const deps = (ph.dependsOn ?? []).map((n) => (phaseIds.has(n) ? `P${n}` : `P${n} (not in these props)`));
      g.node({
        id: pid, kind: "phase", group: "group-timeline", code: `P${ph.number}`,
        kicker: [`P${ph.number}`, ph.kind === "pr-review" && "review", ph.gate && "merge gate", ph.lane && `lane ${ph.lane}`].filter(Boolean).join(" · "),
        title: ph.title, summary: ph.doneWhen ?? (ph.kind === "pr-review" ? "Review phase" : "Work phase"),
        tone: ph.kind === "pr-review" ? "warn" : null,
        badges: [ph.kind === "pr-review" && "review", ph.gate && "merge gate", ph.lane && `lane ${ph.lane}`, ph.repository, ph.estimate].filter(Boolean),
        facts: [fact("Title", ph.title), fact("Done when", ph.doneWhen, true), deps.length && fact("Depends on", deps.join(", "), true),
          fact("Kind", ph.kind), fact("Lane", ph.lane), fact("Repository", ph.repository), fact("Estimate", ph.estimate),
          ph.components?.length && fact("Touches", ph.components.join(", "), true)],
      });
      g.edge(id, pid, "contains");
      for (const n of ph.dependsOn ?? []) if (phaseIds.has(n)) g.edge(phaseIds.get(n), pid, "depends");
      for (const name of ph.components ?? []) g.edge(pid, componentIds.get(name), "touches");
    }
  }

  p.architecture.layers.forEach((l, li) => {
    const id = `layer-${li + 1}`;
    g.node({
      id, kind: "layer", group: "group-architecture", code: `L${li + 1}`, kicker: `Layer ${li + 1} · ${l.components.length} ${plural(l.components.length, "component")}`,
      title: l.name, summary: l.components.map((c) => c.name).join(", "),
      facts: [fact("Layer", l.name), { label: "Components", list: l.components.map((c) => `${c.name}: ${c.role}`), key: true }],
    });
    for (const c of l.components) {
      const out = flows.filter((f) => f.from === c.name).map((f) => `→ ${f.to}${componentIds.has(f.to) ? "" : " (not in the architecture)"}: ${f.label}`);
      const into = flows.filter((f) => f.to === c.name).map((f) => `← ${f.from}${componentIds.has(f.from) ? "" : " (not in the architecture)"}: ${f.label}`);
      g.node({
        id: nodeIds.get(c), kind: "component", group: "group-architecture", kicker: ["Component", c.status].filter(Boolean).join(" · "),
        title: c.name, summary: c.role, tone: c.status ? TONE.status[c.status] : null, badges: [c.status].filter(Boolean),
        facts: [fact("Role", c.role, true), fact("Status", c.status, true), fact("Layer", l.name), (out.length || into.length) && { label: "Flows", list: [...out, ...into] }],
      });
      g.edge(id, nodeIds.get(c), "contains");
    }
  });
  for (const f of flows) {
    if (componentIds.has(f.from) && componentIds.has(f.to)) g.edge(componentIds.get(f.from), componentIds.get(f.to), "flow", f.label);
  }

  p.decisions.forEach((d, i) => {
    const id = `decision-${i + 1}`;
    g.node({
      id, kind: "decision", group: "group-decisions", code: `D${i + 1}`, kicker: `Decision ${i + 1}`, title: d.decision, summary: d.why,
      badges: [`${d.rejected.length} rejected`],
      facts: [fact("Decision", d.decision), fact("Why", d.why, true), { label: "Rejected", list: d.rejected.map((r) => `${r.option}: ${r.reason}`), key: true }],
    });
    for (const n of d.phases ?? []) g.edge(id, phaseIds.get(n), "affects");
    for (const name of d.components ?? []) g.edge(id, componentIds.get(name), "affects");
  });

  const risks = p.risks.map((r, i) => ({ r, i })).sort((a, b) => IMPACT_ORDER[a.r.impact] - IMPACT_ORDER[b.r.impact] || a.i - b.i);
  for (const { r, i } of risks) {
    const id = `risk-${i + 1}`;
    g.node({
      id, kind: "risk", group: "group-risks", code: `R${i + 1}`, kicker: [`Risk ${i + 1}`, `impact ${r.impact}`].join(" · "),
      title: r.risk, summary: r.mitigation, tone: TONE.level[r.impact],
      badges: [`impact ${r.impact}`, r.likelihood && `likelihood ${r.likelihood}`].filter(Boolean),
      facts: [fact("Risk", r.risk), fact("Impact", r.impact, true), fact("Likelihood", r.likelihood), fact("Mitigation", r.mitigation, true),
        r.mitigatedBy?.length && fact("Mitigated by", r.mitigatedBy.map((n) => `P${n}`).join(", "))],
    });
    for (const n of r.mitigatedBy ?? []) g.edge(phaseIds.get(n), id, "mitigates");
    for (const name of r.components ?? []) g.edge(id, componentIds.get(name), "affects");
  }

  const maxPhases = Math.max(...p.milestones.map((m) => m.phases.length));
  const maxComponents = Math.max(...p.architecture.layers.map((l) => l.components.length));
  return {
    ...g.result(),
    blocks: {
      "group-timeline": headed(p.milestones.map((m) => ({ head: `milestone-${m.number}`, items: m.phases.map((ph) => phaseIds.get(ph.number)) })), gridCols(maxPhases, 7)),
      "group-architecture": headed(p.architecture.layers.map((l, li) => ({ head: `layer-${li + 1}`, items: l.components.map((c) => nodeIds.get(c)) })), Math.min(maxComponents, 4)),
      "group-decisions": grid(p.decisions.map((_, i) => `decision-${i + 1}`)),
      "group-risks": grid(risks.map(({ i }) => `risk-${i + 1}`)),
    },
  };
}

// ---- ship ----

function shipModel(p) {
  const g = graphBuilder();
  const met = p.delivered.filter((d) => d.status === "met").length;
  const findings = p.reviews.flatMap((r) => r.findings);
  const unresolved = findings.filter((f) => f.status !== "fixed").length;
  g.node({
    id: "summary", kind: "summary", group: null, kicker: `Ship${p.meta.milestone ? ` · milestone ${p.meta.milestone}` : ""}`,
    title: p.meta.title, summary: p.summary, root: true,
    badges: [`${met}/${p.delivered.length} met`, ...p.reviews.map((r) => VERDICT_LABEL[r.verdict]), `${unresolved} unresolved`],
    facts: [
      fact("Summary", p.summary, true), fact("Subtitle", p.meta.subtitle),
      { label: "Done when met", text: `${met} of ${p.delivered.length}` },
      { label: "Review verdicts", text: p.reviews.map((r) => VERDICT_LABEL[r.verdict]).join(", ") || "No review yet" },
      { label: "Unresolved findings", text: String(unresolved) },
      !p.deviations.length && { label: "Deviations", text: "No deviations from the plan." },
      ...metaFacts(p.meta),
    ],
  });

  const phaseIds = new Map();
  const deliveredOrder = [];
  p.delivered.forEach((d, i) => {
    if (d.phase === undefined) {
      deliveredOrder.push({ id: `delivered-${i + 1}`, rows: [d] });
    } else if (phaseIds.has(d.phase)) {
      deliveredOrder.find((b) => b.id === phaseIds.get(d.phase)).rows.push(d);
    } else {
      phaseIds.set(d.phase, `phase-${d.phase}`);
      deliveredOrder.push({ id: `phase-${d.phase}`, phase: d.phase, rows: [d] });
    }
  });

  const areaIds = new Map();
  (p.changes ?? []).forEach((c, i) => {
    const id = `change-${i + 1}`;
    if (!areaIds.has(c.area)) areaIds.set(c.area, id);
    g.node({
      id, kind: "change", group: "group-changes", code: `C${i + 1}`, kicker: `Change ${i + 1}`, title: c.area, summary: c.how,
      facts: [fact("Area", c.area), fact("How", c.how, true), fact("Why", c.why, true), c.phases?.length && fact("Phases", c.phases.map((n) => `P${n}`).join(", "))],
    });
    for (const n of c.phases ?? []) g.edge(phaseIds.get(n), id, "delivers");
  });

  for (const block of deliveredOrder) {
    const worst = block.rows.reduce((w, d) => (STATUS_RANK[d.status] > STATUS_RANK[w] ? d.status : w), "met");
    const [first] = block.rows;
    const multi = block.rows.length > 1;
    g.node({
      id: block.id, kind: "delivered", group: "group-delivered", code: block.phase !== undefined ? `P${block.phase}` : null,
      kicker: [block.phase !== undefined ? `P${block.phase}` : "Done when", worst].join(" · "),
      title: first.criterion, summary: multi ? `${block.rows.length} criteria: ${block.rows.map((d) => d.status).join(", ")}` : first.result,
      tone: TONE.delivered[worst], badges: [worst],
      facts: block.rows.flatMap((d, i) => [
        { label: multi ? `Criterion ${i + 1}` : "Criterion", text: d.criterion },
        { label: "Result", text: d.result, key: i === 0 },
        { label: "Status", text: d.status, key: i === 0 },
      ]),
    });
  }

  p.deviations.forEach((d, i) => {
    const id = `deviation-${i + 1}`;
    g.node({
      id, kind: "deviation", group: "group-deviations", code: `V${i + 1}`, kicker: `Deviation ${i + 1}`, title: d.planned, summary: d.actual, tone: "warn",
      facts: [fact("Planned", d.planned, true), fact("Actual", d.actual, true), fact("Why", d.reason, true)],
    });
    for (const n of d.phases ?? []) g.edge(id, phaseIds.get(n), "concerns");
  });

  const setbackIds = [];
  if (p.process) {
    const stats = PROCESS_STATS.filter(([key]) => p.process[key] !== undefined);
    setbackIds.push("process");
    g.node({
      id: "process", kind: "process", group: "group-setbacks", kicker: "Process numbers", title: "Process numbers",
      summary: stats.map(([key, label]) => `${p.process[key]} ${label}`).join(", ") || "No numbers",
      facts: [{ label: "Numbers", list: stats.map(([key, label]) => `${p.process[key]} ${label}`), key: true }],
    });
  }
  (p.setbacks ?? []).forEach((s, i) => {
    const id = `setback-${i + 1}`;
    setbackIds.push(id);
    g.node({
      id, kind: "setback", group: "group-setbacks", code: `S${i + 1}`,
      kicker: [s.kind, s.phase !== undefined && `P${s.phase}`].filter(Boolean).join(" · "),
      title: s.title, summary: s.what, tone: SETBACK_TONE[s.kind], badges: [s.kind, s.cost].filter(Boolean),
      facts: [fact("Kind", s.kind), s.phase !== undefined && fact("Phase", `P${s.phase}`), fact("What", s.what, true), fact("Cost", s.cost, true),
        fact("Resolution", s.resolution, true), fact("Lesson", s.lesson)],
    });
    if (s.phase !== undefined && phaseIds.has(s.phase)) g.edge(id, phaseIds.get(s.phase), "during");
  });

  const findingIds = [];
  const usedFindings = new Set();
  p.reviews.forEach((r, ri) => {
    const id = `review-${ri + 1}`;
    g.node({
      id, kind: "review", group: "group-review", kicker: ["Review", r.repository, r.head?.slice(0, 7)].filter(Boolean).join(" · "),
      title: r.repository ? `Review of ${r.repository}` : `Review ${ri + 1}`, summary: r.summary, tone: TONE.verdict[r.verdict],
      badges: [VERDICT_LABEL[r.verdict], `${r.findings.length} ${plural(r.findings.length, "finding")}`],
      facts: [
        { label: "Verdict", text: VERDICT_LABEL[r.verdict], key: true }, fact("Summary", r.summary, true),
        { label: "Validation", list: r.validation.map((v) => `${v.result}: \`${v.command.replaceAll("`", "'")}\`${v.note ? ` (${v.note})` : ""}`) },
        { label: "Findings by severity", text: SEVERITIES.map((s) => `${r.findings.filter((f) => f.severity === s).length} ${s}`).join(", ") },
        r.head && { label: "Head", code: r.head }, r.split && fact("Stacked-PR split", r.split),
        (r.pr || r.url) && { label: "Links", links: [r.pr && { label: "Pull request", url: r.pr }, r.url && { label: "Posted review", url: r.url }].filter(Boolean) },
      ],
    });
    g.edge("summary", id, "contains");
    const sorted = r.findings.map((f, fi) => ({ f, fi }))
      .sort((a, b) => SEVERITIES.indexOf(a.f.severity) - SEVERITIES.indexOf(b.f.severity) || a.fi - b.fi);
    const ids = [];
    for (const { f, fi } of sorted) {
      const fid = unique(`finding-${p.reviews.length > 1 ? `${ri + 1}-` : ""}${f.id ? slug(f.id) : fi + 1}`, usedFindings);
      ids.push(fid);
      g.node({
        id: fid, kind: "finding", group: "group-review", code: f.id ?? null,
        kicker: [f.id, f.severity, f.status].filter(Boolean).join(" · "), title: f.title, summary: f.detail ?? f.fix ?? `${f.severity}, ${f.status}`,
        tone: TONE.severity[f.severity], badges: [f.severity, f.status],
        facts: [fact("Finding", f.title), { label: "Severity", text: `${f.severity}, ${f.status}`, key: true }, f.location && { label: "Location", code: f.location },
          fact("Detail", f.detail, true), fact("Fix", f.fix, true)],
      });
      g.edge(id, fid, "contains");
      for (const n of f.phases ?? []) g.edge(fid, phaseIds.get(n), "about");
      for (const area of f.changes ?? []) g.edge(fid, areaIds.get(area), "about");
    }
    findingIds.push({ head: id, items: ids });
  });

  p.handTest.forEach((s, i) => {
    const id = `step-${i + 1}`;
    g.node({
      id, kind: "step", group: "group-hand-test", code: `${i + 1}`, kicker: `Hand test · step ${i + 1}`,
      title: s.title ?? s.command.split("\n")[0], summary: s.why,
      facts: [fact("Step", s.title), { label: "Command", command: s.command, key: true }, { label: "Code", code: s.ref, key: true }, fact("Why", s.why, true), fact("Expect", s.expect),
        s.phases?.length && fact("Checks", s.phases.map((n) => `P${n}`).join(", "))],
    });
    if (i) g.edge(`step-${i}`, id, "then");
    for (const n of s.phases ?? []) g.edge(id, phaseIds.get(n), "checks");
  });

  p.rollback.forEach((s, i) => {
    const id = `rollback-${i + 1}`;
    g.node({
      id, kind: "rollback", group: "group-rollback", code: `${i + 1}`, kicker: `Rollback · step ${i + 1}`, title: s.step, summary: s.command ?? s.step,
      facts: [fact("Step", s.step, true), s.command && { label: "Command", command: s.command, key: true }, s.ref && { label: "Code", code: s.ref }],
    });
    if (i) g.edge(`rollback-${i}`, id, "then");
  });

  return {
    ...g.result(),
    blocks: {
      "group-changes": grid((p.changes ?? []).map((_, i) => `change-${i + 1}`)),
      "group-delivered": grid(deliveredOrder.map((b) => b.id)),
      "group-deviations": grid(p.deviations.map((_, i) => `deviation-${i + 1}`)),
      "group-setbacks": grid(setbackIds),
      "group-review": headed(findingIds, gridCols(Math.max(1, ...findingIds.map((s) => s.items.length)), 4)),
      "group-hand-test": grid(p.handTest.map((_, i) => `step-${i + 1}`)),
      "group-rollback": grid(p.rollback.map((_, i) => `rollback-${i + 1}`)),
    },
  };
}

// ---- model helpers ----

function graphBuilder() {
  const nodes = [];
  const edges = [];
  const seen = new Set();
  return {
    node({ id, kind, group, code = null, kicker, title, summary, tone = null, badges = [], facts, root = false }) {
      const lines = root ? wrap(title, LINE_CHARS.root, 2) : wrap(title, LINE_CHARS.node, 2);
      nodes.push({
        id, kind, group, code, kicker: clipText(kicker, KICKER_CHARS), title, summary: clipText(summary, SUMMARY_CHARS),
        tone, badges, facts: facts.filter(Boolean), lines,
        // The root box also shows the start of its summary under the title.
        ...(root ? { sub: wrap(summary, LINE_CHARS.sub, 4 - lines.length) } : {}),
      });
    },
    edge(from, to, type, label) {
      const key = `${from}>${to}>${type}`;
      if (!from || !to || seen.has(key) || from === to) return;
      seen.add(key);
      edges.push(label === undefined ? { from, to, type } : { from, to, type, label });
    },
    result: () => ({ nodes, edges }),
  };
}

// A fact with text, or null when the value is absent. `key` facts are the main facts of a node.
function fact(label, value, key = false) {
  if (value === undefined || value === null || value === "") return null;
  return key ? { label, text: String(value), key } : { label, text: String(value) };
}

function metaFacts(meta) {
  return [
    { label: "Date", text: meta.date }, fact("Plan", meta.planId), meta.milestone && { label: "Milestone", text: String(meta.milestone) },
    meta.links?.length && { label: "Links", links: meta.links },
  ];
}

function unique(base, used) {
  let id = base;
  for (let n = 2; used.has(id); n += 1) id = `${base}-${n}`;
  used.add(id);
  return id;
}

function countBy(values) {
  const counts = new Map();
  for (const v of values) counts.set(v, (counts.get(v) ?? 0) + 1);
  return counts;
}

function plural(n, word) {
  return n === 1 ? word : `${word}s`;
}

// ---- layout ----

// Columns for a block of n items: near a 4:3 block, from 1 to `max`.
function gridCols(n, max = 6) {
  return Math.max(1, Math.min(max, n, Math.round(Math.sqrt(n * 1.3))));
}

// A block of items in reading order. Positions are relative to the block.
function grid(ids, cols = gridCols(ids.length)) {
  return ids.map((id, i) => ({ id, x: (i % cols) * (NODE.w + GAP), y: Math.floor(i / cols) * (NODE.h + GAP) }));
}

// Sections with a head node in the first column and their items in reading order to its right.
function headed(sections, cols) {
  const cells = [];
  let y = 0;
  const itemsX = NODE.w + GAP + HEAD_GAP;
  for (const { head, items } of sections) {
    cells.push({ id: head, x: 0, y });
    items.forEach((id, i) => cells.push({ id, x: itemsX + (i % cols) * (NODE.w + GAP), y: y + Math.floor(i / cols) * (NODE.h + GAP) }));
    y += Math.max(1, Math.ceil(items.length / cols)) * (NODE.h + GAP) - GAP + SECTION_GAP;
  }
  return cells;
}

// Each group is a framed block. The root sits at the origin. Each group, the largest first, takes
// the free position that keeps the whole drawing smallest in a 16:9 frame, near the root and near
// the groups it has edges with. Then all positions move so that the drawing starts at MARGIN.
function layout(graph, blocks) {
  const byId = new Map(graph.nodes.map((n) => [n.id, n]));
  const order = GROUPS[graph.mode].map(([name, label]) => ({ id: `group-${name}`, label }));
  const groups = order.map(({ id, label }, index) => {
    const cells = (blocks[id] ?? []).filter((c) => byId.has(c.id));
    if (!cells.length) return null;
    const w = Math.max(...cells.map((c) => c.x)) + NODE.w + GROUP_PAD * 2;
    const h = Math.max(...cells.map((c) => c.y)) + NODE.h + GROUP_PAD * 2 + GROUP_LABEL;
    const kinds = [...new Set(cells.map((c) => byId.get(c.id).kind))];
    return { id, name: label, label: groupLabel(label, cells.length, kinds), kinds, index, cells, w, h };
  }).filter(Boolean);

  const root = graph.nodes.find((n) => n.group === null);
  const rootRect = { x: -ROOT.w / 2, y: -ROOT.h / 2, w: ROOT.w, h: ROOT.h };
  const weights = groupWeights(graph, byId);
  const placed = [{ id: null, ...rootRect }];
  for (const group of [...groups].sort((a, b) => b.w * b.h - a.w * a.h || a.index - b.index)) {
    const spot = bestSpot(group, placed, weights);
    group.x = spot.x;
    group.y = spot.y;
    placed.push({ id: group.id, x: group.x, y: group.y, w: group.w, h: group.h });
  }

  const minX = Math.min(...placed.map((r) => r.x));
  const minY = Math.min(...placed.map((r) => r.y));
  const dx = MARGIN - minX;
  const dy = MARGIN - minY;
  Object.assign(root, { x: rootRect.x + dx, y: rootRect.y + dy, w: ROOT.w, h: ROOT.h });
  graph.groups = groups.map((group) => {
    const x = group.x + dx;
    const y = group.y + dy;
    for (const cell of group.cells) Object.assign(byId.get(cell.id), { x: x + GROUP_PAD + cell.x, y: y + GROUP_PAD + GROUP_LABEL + cell.y, w: NODE.w, h: NODE.h });
    return { id: group.id, name: group.name, label: group.label, kinds: group.kinds, x, y, w: group.w, h: group.h };
  });
  graph.width = Math.max(...placed.map((r) => r.x + r.w)) + dx + MARGIN;
  graph.height = Math.max(...placed.map((r) => r.y + r.h)) + dy + MARGIN;
  for (const edge of graph.edges) edge.d = edgePath(byId.get(edge.from), byId.get(edge.to));
}

// A group of one kind shows its count. A mixed group (a timeline, an architecture) does not.
function groupLabel(label, count, kinds) {
  return kinds.length === 1 ? `${label} · ${count}` : label;
}

// Edge counts between groups, by pair key "a|b" with a < b.
function groupWeights(graph, byId) {
  const weights = new Map();
  for (const edge of graph.edges) {
    const a = byId.get(edge.from).group ?? "root";
    const b = byId.get(edge.to).group ?? "root";
    if (a === b) continue;
    const key = [a, b].sort().join("|");
    weights.set(key, (weights.get(key) ?? 0) + 1);
  }
  return weights;
}

function bestSpot(group, placed, weights) {
  let best = null;
  for (let ray = 0; ray < RAYS; ray += 1) {
    const angle = (ray / RAYS) * 2 * Math.PI;
    const ux = Math.cos(angle) * SCREEN;
    const uy = Math.sin(angle);
    for (let r = 0; ; r += RAY_STEP) {
      const rect = { x: Math.round(ux * r - group.w / 2), y: Math.round(uy * r - group.h / 2), w: group.w, h: group.h };
      if (placed.some((p) => overlaps(rect, p, GROUP_GAP))) continue;
      const cost = spotCost(rect, group, placed, weights);
      if (!best || cost < best.cost - 1e-6) best = { ...rect, cost };
      break;
    }
  }
  return best;
}

// The width of the smallest 16:9 frame around everything, plus the pull to the root and to the
// groups with edges to this group.
function spotCost(rect, group, placed, weights) {
  const all = [...placed, rect];
  const w = Math.max(...all.map((r) => r.x + r.w)) - Math.min(...all.map((r) => r.x));
  const h = Math.max(...all.map((r) => r.y + r.h)) - Math.min(...all.map((r) => r.y));
  const frame = Math.max(w, h * SCREEN);
  const cx = rect.x + rect.w / 2;
  const cy = rect.y + rect.h / 2;
  let pull = 0.25 * (cx * cx + cy * cy);
  for (const p of placed) {
    const weight = weights.get([p.id ?? "root", group.id].sort().join("|")) ?? 0;
    if (!weight) continue;
    const px = p.x + p.w / 2;
    const py = p.y + p.h / 2;
    pull += 0.2 * Math.min(weight, 8) * ((cx - px) ** 2 + (cy - py) ** 2);
  }
  return frame * frame + pull;
}

function overlaps(a, b, gap) {
  return a.x < b.x + b.w + gap && b.x < a.x + a.w + gap && a.y < b.y + b.h + gap && b.y < a.y + a.h + gap;
}

// A straight line between two boxes in one row or one column, else a gentle curve. It starts and
// ends on the box borders.
function edgePath(a, b) {
  const ca = { x: a.x + a.w / 2, y: a.y + a.h / 2 };
  const cb = { x: b.x + b.w / 2, y: b.y + b.h / 2 };
  const p1 = border(a, cb);
  const p2 = border(b, ca);
  const straight = Math.abs(ca.y - cb.y) < 1 || Math.abs(ca.x - cb.x) < 1;
  const len = Math.hypot(p2.x - p1.x, p2.y - p1.y);
  if (straight && len < NODE.w * 1.5) return `M${n(p1.x)} ${n(p1.y)} L${n(p2.x)} ${n(p2.y)}`;
  const bend = Math.min(80, len * 0.18);
  const cx = (p1.x + p2.x) / 2 + ((p2.y - p1.y) / (len || 1)) * bend;
  const cy = (p1.y + p2.y) / 2 - ((p2.x - p1.x) / (len || 1)) * bend;
  return `M${n(p1.x)} ${n(p1.y)} Q${n(cx)} ${n(cy)} ${n(p2.x)} ${n(p2.y)}`;
}

// The point where the line from the box center to `target` leaves the box.
function border(box, target) {
  const cx = box.x + box.w / 2;
  const cy = box.y + box.h / 2;
  const dx = target.x - cx;
  const dy = target.y - cy;
  if (!dx && !dy) return { x: cx, y: cy };
  const t = Math.min(dx ? box.w / 2 / Math.abs(dx) : Infinity, dy ? box.h / 2 / Math.abs(dy) : Infinity);
  return { x: cx + dx * t, y: cy + dy * t };
}

function n(value) {
  return Math.round(value * 10) / 10;
}
