// Ship scenes: summary, changes, delivered, deviations, setbacks, review, hand test, rollback.
import { interpolate, useCurrentFrame } from "remotion";
import { COLORS as C, FONTS, TYPE } from "./theme.mjs";
import {
  Card, Chip, Command, Inline, Label, LEVEL_COLOR, More, Pill, SceneFrame, SEVERITY_COLOR, STATUS_COLOR, Step, Verdict, stagger, useRise,
} from "./ui.jsx";

export const SHIP_SCENES = {
  summary: SummaryScene,
  changes: ChangesScene,
  delivered: DeliveredScene,
  deviations: DeviationsScene,
  setbacks: SetbacksScene,
  review: ReviewScene,
  "hand-test": HandTestScene,
  rollback: RollbackScene,
};

const STATUS_PATH = { met: "M6 13l4 4 8-9", partial: "M6 12h12", missed: "M7 7l10 10M17 7L7 17" };
const STATUS_LABEL = { met: "Met", partial: "Partial", missed: "Missed" };

function SummaryScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      <p style={{ margin: "auto 0", fontSize: TYPE.lead, lineHeight: 1.35, maxWidth: 1560, ...useRise(8) }}><Inline text={scene.text} /></p>
    </SceneFrame>
  );
}

function ChangesScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 24 }}>
        {scene.changes.map((c, i) => (
          <Card key={i} index={i} style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            <h3 style={{ margin: 0, fontSize: 42 }}>{c.area}</h3>
            <p style={{ margin: 0, fontSize: 30, lineHeight: 1.35 }}><b>How</b> <Inline text={c.how} /></p>
            <p style={{ margin: 0, fontSize: 30, lineHeight: 1.35, color: C["ink-2"] }}><b style={{ color: C.ink }}>Why</b> <Inline text={c.why} /></p>
          </Card>
        ))}
      </div>
    </SceneFrame>
  );
}

function DeliveredScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      {scene.delivered.map((d, i) => (
        <Card key={i} index={i} style={{ display: "grid", gridTemplateColumns: "56px minmax(0, 1fr)", gap: 22, alignItems: "start", padding: "16px 26px" }}>
          <StatusIcon status={d.status} delay={stagger(i) + 8} />
          <div>
            <p style={{ margin: 0, fontSize: 36, fontWeight: 600, lineHeight: 1.3 }}>
              {d.phase !== undefined && <span style={{ fontFamily: FONTS.mono, fontSize: 28, color: C.accent, border: `2px solid ${C.line}`, borderRadius: 8, padding: "0 10px", marginRight: 14 }}>P{d.phase}</span>}
              <Inline text={d.criterion} />
            </p>
            <p style={{ margin: "4px 0 0", fontSize: 30, lineHeight: 1.3, color: C["ink-2"] }}>
              <b style={{ color: STATUS_COLOR[d.status] }}>{STATUS_LABEL[d.status]}</b> <Inline text={d.result} />
            </p>
          </div>
        </Card>
      ))}
    </SceneFrame>
  );
}

function StatusIcon({ status, delay }) {
  const frame = useCurrentFrame();
  const draw = interpolate(frame, [delay, delay + 14], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const color = STATUS_COLOR[status];
  return (
    <svg width="56" height="56" viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="11" fill="none" stroke={color} strokeWidth="1.5" />
      <path d={STATUS_PATH[status]} pathLength="1" strokeDasharray="1" strokeDashoffset={draw} fill="none" stroke={color} strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function DeviationsScene({ scene }) {
  if (!scene.deviations.length) {
    return (
      <SceneFrame scene={scene}>
        <p style={{ margin: "auto 0", fontSize: TYPE.lead, fontStyle: "italic", color: C.muted, ...useRise(8) }}>No deviations from the plan.</p>
      </SceneFrame>
    );
  }
  return (
    <SceneFrame scene={scene}>
      {scene.deviations.map((d, i) => (
        <Card key={i} index={i} style={{ display: "grid", gridTemplateColumns: "minmax(0, 1fr) 80px minmax(0, 1fr)", gap: 20, alignItems: "center" }}>
          <div><Label>Planned</Label><p style={{ margin: "6px 0 0", fontSize: 34, lineHeight: 1.3 }}><Inline text={d.planned} /></p></div>
          <svg width="80" height="32" viewBox="0 0 40 16" aria-hidden="true">
            <path d="M2 8h34M30 2l6 6-6 6" fill="none" stroke={C.warn} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          <div><Label>Actual</Label><p style={{ margin: "6px 0 0", fontSize: 34, lineHeight: 1.3 }}><Inline text={d.actual} /></p></div>
          <p style={{ gridColumn: "1 / -1", margin: 0, paddingTop: 12, borderTop: `2px dashed ${C.line}`, fontSize: 30, lineHeight: 1.35, color: C["ink-2"] }}>
            <b style={{ color: C.ink }}>Why</b> <Inline text={d.reason} />
          </p>
        </Card>
      ))}
    </SceneFrame>
  );
}

function SetbacksScene({ scene }) {
  if (!scene.process.length && !scene.setbacks.length) {
    return (
      <SceneFrame scene={scene}>
        <p style={{ margin: "auto 0", fontSize: TYPE.lead, fontStyle: "italic", color: C.muted, ...useRise(8) }}>No setbacks.</p>
      </SceneFrame>
    );
  }
  return (
    <SceneFrame scene={scene}>
      {scene.process.length > 0 && (
        <div style={{ display: "grid", gridTemplateColumns: `repeat(${Math.min(5, scene.process.length)}, minmax(0, 1fr))`, gap: 16 }}>
          {scene.process.map(([label, value], i) => <ProcessTile key={label} label={label} value={value} index={i} />)}
        </div>
      )}
      <div style={{ display: "grid", gridTemplateColumns: `repeat(${Math.max(1, scene.setbacks.length)}, minmax(0, 1fr))`, gap: 24 }}>
        {scene.setbacks.map((s, i) => (
          <Card key={i} index={i + scene.process.length} accent={C.warn} style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            <div style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "center" }}>
              <Pill color={C.warn}>{s.kind}</Pill>
              {s.phase !== undefined && <Chip mono>P{s.phase}</Chip>}
              {s.cost && <Chip>{s.cost}</Chip>}
            </div>
            <p style={{ margin: 0, fontSize: 38, fontWeight: 600, lineHeight: 1.25 }}><Inline text={s.title} /></p>
            <p style={{ margin: 0, fontSize: 30, lineHeight: 1.35 }}><Inline text={s.what} /></p>
            <p style={{ margin: 0, fontSize: 30, lineHeight: 1.35, color: C["ink-2"] }}><b style={{ color: C.ok }}>Resolution</b> <Inline text={s.resolution} /></p>
            {s.lesson && <p style={{ margin: 0, fontSize: 30, lineHeight: 1.35, color: C["ink-2"] }}><b style={{ color: C.info }}>Lesson</b> <Inline text={s.lesson} /></p>}
          </Card>
        ))}
      </div>
    </SceneFrame>
  );
}

function ProcessTile({ label, value, index }) {
  return (
    <div style={{ background: C.surface, border: `2px solid ${C.line}`, borderRadius: 16, padding: "12px 20px", ...useRise(stagger(index)) }}>
      <b style={{ display: "block", fontSize: 50, lineHeight: 1.1 }}>{value}</b>
      <span style={{ fontSize: 28, color: C["ink-2"] }}>{label}</span>
    </div>
  );
}

function ReviewScene({ scene }) {
  const max = Math.max(1, ...scene.severities.map((s) => s.count));
  return (
    <SceneFrame scene={scene}>
      <div style={{ display: "flex", gap: 16, alignItems: "center", flexWrap: "wrap", ...useRise(4) }}>
        <Verdict verdict={scene.verdict} />
        {scene.repository && <Chip>{scene.repository}</Chip>}
        {scene.head && <Chip mono>{scene.head}</Chip>}
      </div>
      <p style={{ margin: 0, fontSize: 36, lineHeight: 1.35, ...useRise(8) }}><Inline text={scene.summary} /></p>
      <div style={{ display: "grid", gridTemplateColumns: "minmax(0, 0.8fr) minmax(0, 1.2fr)", gap: 40, alignItems: "start" }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
          <Label>Findings by severity</Label>
          {scene.severities.map((s, i) => <SeverityBar key={s.severity} row={s} max={max} delay={stagger(i)} />)}
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
          {scene.findings.length === 0 && <p style={{ margin: 0, fontSize: 34, fontStyle: "italic", color: C.muted }}>No findings.</p>}
          {scene.findings.map((f, i) => (
            <Card key={i} index={i + 2} accent={SEVERITY_COLOR[f.severity]} style={{ padding: "14px 22px", opacity: f.status === "fixed" ? 0.75 : 1 }}>
              <div style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>
                <Pill color={SEVERITY_COLOR[f.severity]}>{f.severity}</Pill>
                {f.id && <b style={{ fontFamily: FONTS.mono, fontSize: 30 }}>{f.id}</b>}
                <Pill color={LEVEL_COLOR[f.status]}>{f.status}</Pill>
              </div>
              <p style={{ margin: "8px 0 0", fontSize: 32, fontWeight: 600, lineHeight: 1.3 }}><Inline text={f.title} /></p>
            </Card>
          ))}
          {scene.moreFindings > 0 && <More n={scene.moreFindings} inline />}
        </div>
      </div>
    </SceneFrame>
  );
}

function SeverityBar({ row, max, delay }) {
  const frame = useCurrentFrame();
  const grow = interpolate(frame, [delay, delay + 20], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <div style={{ display: "grid", gridTemplateColumns: "120px minmax(0, 1fr) 210px", gap: 16, alignItems: "center", fontSize: 30 }}>
      <span style={{ color: C["ink-2"] }}>{row.severity}</span>
      <span style={{ height: 18, background: C["surface-2"], borderRadius: 9, overflow: "hidden" }}>
        <span style={{ display: "block", height: "100%", width: `${(row.count / max) * 100 * grow}%`, background: SEVERITY_COLOR[row.severity], borderRadius: 9 }} />
      </span>
      <span>{row.count}{row.count > 0 && <small style={{ color: C.muted, fontSize: 26 }}> ({row.unresolved} open)</small>}</span>
    </div>
  );
}

function HandTestScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      {scene.steps.map((s, i) => (
        <Step key={s.n} n={s.n} color={C.accent} index={i}>
          {s.title && <h3 style={{ margin: 0, fontSize: 36 }}>{s.title}</h3>}
          <Command command={s.command} />
          <p style={{ margin: 0, fontSize: 30, lineHeight: 1.35, color: C["ink-2"] }}><Inline text={s.why} /></p>
          <p style={{ margin: "6px 0 0", fontSize: 26, color: C.muted }}>Code <span style={{ fontFamily: FONTS.mono }}>{s.ref}</span></p>
        </Step>
      ))}
    </SceneFrame>
  );
}

function RollbackScene({ scene }) {
  return (
    <SceneFrame scene={scene}>
      {scene.steps.map((s, i) => (
        <Step key={s.n} n={s.n} color={C.warn} index={i}>
          <p style={{ margin: 0, fontSize: 34, lineHeight: 1.35 }}><Inline text={s.step} /></p>
          {s.command && <Command command={s.command} />}
        </Step>
      ))}
    </SceneFrame>
  );
}
