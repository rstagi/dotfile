// Plan scenes: goal, architecture, flows, timeline, decisions, risks.
import { COLORS as C, FONTS, TYPE } from "./theme.mjs";
import { Badge, Card, Inline, Label, LEVEL_COLOR, More, Pill, SceneFrame, STATUS_COLOR, useRise } from "./ui.jsx";

export const PLAN_SCENES = {
  goal: GoalScene,
  architecture: ArchitectureScene,
  flows: FlowsScene,
  timeline: TimelineScene,
  decisions: DecisionsScene,
  risks: RisksScene,
};

function GoalScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      <p style={{ margin: "auto 0", fontSize: TYPE.lead, lineHeight: 1.35, maxWidth: 1560, ...useRise(8) }}><Inline text={scene.text} /></p>
    </SceneFrame>
  );
}

function ArchitectureScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      <p style={{ margin: 0, fontSize: TYPE.body, color: C["ink-2"], ...useRise(4) }}><Inline text={scene.summary} /></p>
      {scene.layers.map((layer, i) => (
        <Card key={i} index={i} style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <Label>{layer.name}</Label>
          <div style={{ display: "grid", gap: 18, gridTemplateColumns: `repeat(${Math.max(2, layer.components.length)}, minmax(0, 1fr))` }}>
            {layer.components.map((c, j) => (
              <div key={j} style={{ background: C.bg, border: `2px solid ${C.line}`, borderTop: `8px solid ${c.status ? STATUS_COLOR[c.status] : C.accent}`, borderRadius: 16, padding: "14px 20px" }}>
                <div style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>
                  <b style={{ fontSize: 36 }}>{c.name}</b>{c.status && <Badge>{c.status}</Badge>}
                </div>
                <p style={{ margin: "8px 0 0", fontSize: 30, lineHeight: 1.3, color: C["ink-2"] }}><Inline text={c.role} /></p>
              </div>
            ))}
          </div>
          {layer.more > 0 && <More n={layer.more} inline />}
        </Card>
      ))}
    </SceneFrame>
  );
}

function FlowsScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      <Label>Flows</Label>
      {scene.flows.map((f, i) => (
        <Card key={i} index={i} style={{ display: "flex", alignItems: "center", gap: 24, flexWrap: "wrap", fontSize: TYPE.body }}>
          <Node>{f.from}</Node>
          <svg width="72" height="24" viewBox="0 0 72 24" aria-hidden="true">
            <path d="M4 12h58" stroke={C.accent} strokeWidth="4" strokeDasharray="8 8" fill="none" />
            <path d="M54 4l10 8-10 8" stroke={C.accent} strokeWidth="4" fill="none" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          <Node>{f.to}</Node>
          <span style={{ color: C["ink-2"] }}><Inline text={f.label} /></span>
        </Card>
      ))}
    </SceneFrame>
  );
}

function Node({ children }) {
  return <span style={{ fontWeight: 700, padding: "4px 18px", border: `2px solid ${C.line}`, borderRadius: 12, background: C.bg }}>{children}</span>;
}

function TimelineScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      <div style={{ display: "flex", alignItems: "baseline", gap: 20, ...useRise(4) }}>
        <span style={{ fontFamily: FONTS.mono, fontWeight: 700, color: C.bg, background: C.ink, borderRadius: 10, padding: "2px 14px", fontSize: TYPE.small }}>M{scene.milestone.number}</span>
        <h3 style={{ margin: 0, fontSize: 48 }}><Inline text={scene.milestone.title} /></h3>
      </div>
      <div style={{ position: "relative", display: "flex", flexDirection: "column", gap: 16 }}>
        <div style={{ position: "absolute", left: 15, top: 20, bottom: 20, width: 4, background: `linear-gradient(${C.accent}, ${C.info})` }} />
        {scene.phases.map((ph, i) => <PhaseRow key={ph.number} phase={ph} index={i} />)}
      </div>
    </SceneFrame>
  );
}

function PhaseRow({ phase, index }) {
  const review = phase.kind === "pr-review";
  return (
    <div style={{ display: "grid", gridTemplateColumns: "34px minmax(0, 1fr)", gap: 22, alignItems: "center", ...useRise(8 + index * 6) }}>
      <span style={{ width: 34, height: 34, background: C.surface, border: `5px solid ${review ? C.warn : C.accent}`, borderRadius: review ? 6 : "50%",
        transform: review ? "rotate(45deg) scale(0.85)" : "none", zIndex: 1 }} />
      <div style={{ background: C.surface, border: `2px solid ${C.line}`, borderRadius: 18, padding: "14px 24px", display: "flex", alignItems: "center", gap: 18, minWidth: 0 }}>
        <span style={{ fontFamily: FONTS.mono, fontWeight: 700, color: C.accent, fontSize: 34 }}>P{phase.number}</span>
        <span style={{ fontSize: TYPE.body, fontWeight: 600, flex: 1, minWidth: 0 }}><Inline text={phase.title} /></span>
        {review && <Badge>review</Badge>}
        {phase.gate && <Badge background={C.warn} color={C.bg}>merge gate</Badge>}
        {phase.lane && <Badge>lane {phase.lane}</Badge>}
        {phase.estimate && <Badge>{phase.estimate}</Badge>}
      </div>
    </div>
  );
}

function DecisionsScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      <div style={{ display: "grid", gridTemplateColumns: `repeat(${scene.decisions.length}, minmax(0, 1fr))`, gap: 28, alignItems: "start" }}>
        {scene.decisions.map((d, i) => (
          <Card key={i} index={i} style={{ display: "flex", flexDirection: "column", gap: 18 }}>
            <h3 style={{ margin: 0, fontSize: 44, lineHeight: 1.2 }}><Inline text={d.decision} /></h3>
            <p style={{ margin: 0, fontSize: 34, lineHeight: 1.35, color: C["ink-2"] }}><Inline text={d.why} /></p>
            <Label>Rejected</Label>
            <div style={{ display: "grid", gridTemplateColumns: "28px minmax(0, 1fr)", gap: 14, fontSize: 32, lineHeight: 1.35, color: C["ink-2"] }}>
              <svg width="28" height="28" viewBox="0 0 16 16" style={{ marginTop: 6 }} aria-hidden="true">
                <path d="M4 4l8 8M12 4l-8 8" stroke={C.bad} strokeWidth="2.4" strokeLinecap="round" />
              </svg>
              <span><b style={{ color: C.ink }}><Inline text={d.rejected.option} /></b>: <Inline text={d.rejected.reason} /></span>
            </div>
            {d.moreRejected > 0 && <More n={d.moreRejected} inline />}
          </Card>
        ))}
      </div>
    </SceneFrame>
  );
}

function RisksScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      {scene.risks.map((r, i) => (
        <Card key={i} index={i} accent={LEVEL_COLOR[r.impact]} style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          <div style={{ display: "flex", gap: 12 }}>
            <Pill color={LEVEL_COLOR[r.impact]}>impact {r.impact}</Pill>
            {r.likelihood && <Pill color={LEVEL_COLOR[r.likelihood]}>likelihood {r.likelihood}</Pill>}
          </div>
          <p style={{ margin: 0, fontSize: TYPE.body, fontWeight: 600, lineHeight: 1.3 }}><Inline text={r.risk} /></p>
          <p style={{ margin: 0, fontSize: 32, lineHeight: 1.35, color: C["ink-2"] }}><b style={{ color: C.ink }}>Mitigation</b> <Inline text={r.mitigation} /></p>
        </Card>
      ))}
    </SceneFrame>
  );
}
