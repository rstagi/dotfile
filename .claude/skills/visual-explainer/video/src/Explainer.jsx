// The composition component: one Sequence per storyboard scene, one per narration clip, a footer
// and a progress bar.
import { useMemo } from "react";
import { AbsoluteFill, Html5Audio, interpolate, Sequence, staticFile, useCurrentFrame, useVideoConfig } from "remotion";
import { PLAN_SCENES } from "./plan-scenes.jsx";
import { SHIP_SCENES } from "./ship-scenes.jsx";
import { storyboard } from "./storyboard.mjs";
import { COLORS as C, FONTS, TYPE } from "./theme.mjs";
import { Chip, Inline, useRise, Verdict } from "./ui.jsx";

const SCENES = { title: TitleScene, end: EndScene, ...PLAN_SCENES, ...SHIP_SCENES };

export function Explainer(props) {
  const { fps } = useVideoConfig();
  const board = useMemo(() => storyboard(props, props.voiceover, { fps }), [props, fps]);
  return (
    <AbsoluteFill style={{ background: C.bg, color: C.ink, fontFamily: FONTS.sans }}>
      {board.scenes.map((scene, i) => {
        const Scene = SCENES[scene.kind];
        return (
          <Sequence key={i} from={scene.from} durationInFrames={scene.durationInFrames} name={`${scene.kind} ${i + 1}`}>
            <Scene scene={scene} />
          </Sequence>
        );
      })}
      {board.sections.filter((s) => s.audio).map(({ id, audio }) => (
        <Sequence key={id} from={audio.from} durationInFrames={audio.durationInFrames} name={`voice ${id}`}>
          <Html5Audio src={staticFile(audio.src)} />
        </Sequence>
      ))}
      <Footer mode={props.mode} date={props.meta.date} total={board.durationInFrames} />
    </AbsoluteFill>
  );
}

function Footer({ mode, date, total }) {
  const frame = useCurrentFrame();
  return (
    <>
      <div style={{ position: "absolute", left: 104, bottom: 34, fontSize: 26, color: C.muted }}>visual-explainer · {mode} · {date}</div>
      <div style={{ position: "absolute", left: 0, right: 0, bottom: 0, height: 8, background: C["surface-2"] }}>
        <div style={{ height: "100%", width: `${(frame / Math.max(1, total - 1)) * 100}%`, background: C.accent }} />
      </div>
    </>
  );
}

function TitleScene({ scene }) {
  const frame = useCurrentFrame();
  const fadeOut = interpolate(frame, [scene.durationInFrames - 10, scene.durationInFrames], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <AbsoluteFill style={{ padding: "0 104px", display: "grid", gridTemplateColumns: "minmax(0, 1fr) auto", alignItems: "center", gap: 80, opacity: fadeOut }}>
      <div>
        <p style={{ margin: "0 0 16px", color: C.accent, fontWeight: 700, letterSpacing: "0.08em", textTransform: "uppercase", fontSize: TYPE.small, ...useRise(0) }}>{scene.eyebrow}</p>
        <h1 style={{ margin: 0, fontSize: TYPE.hero, lineHeight: 1.05, letterSpacing: "-0.02em", ...useRise(4) }}>{scene.title}</h1>
        <p style={{ margin: "28px 0 0", fontSize: 44, lineHeight: 1.3, color: C["ink-2"], maxWidth: 1100, ...useRise(10) }}><Inline text={scene.subtitle} /></p>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 14, marginTop: 32, ...useRise(16) }}>
          {scene.chips.map((chip) => <Chip key={chip}>{chip}</Chip>)}
        </div>
      </div>
      <div style={useRise(20)}>{scene.eyebrow === "Plan" ? <PlanStats stats={scene.stats} /> : <ShipStats stats={scene.stats} />}</div>
    </AbsoluteFill>
  );
}

function PlanStats({ stats }) {
  const frame = useCurrentFrame();
  const line = (n, word) => <p style={{ margin: 0, fontSize: 36, color: C["ink-2"] }}><b style={{ color: C.ink, fontSize: 64 }}>{n}</b> {n === 1 ? word : `${word}s`}</p>;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 18, maxWidth: 480 }}>
      {line(stats.milestones, "milestone")}
      {line(stats.phases, "phase")}
      {stats.lanes > 0 && line(stats.lanes, "lane")}
      <div style={{ display: "flex", flexWrap: "wrap", gap: 8, marginTop: 8 }}>
        {stats.strip.map((kind, i) => {
          const t = interpolate(frame, [24 + i * 1.5, 34 + i * 1.5], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
          if (kind === "sep") return <i key={i} style={{ width: 4, height: 28, background: C.line }} />;
          return <i key={i} style={{ width: 28, height: 28, borderRadius: 5, opacity: t, background: kind === "gate" ? C.warn : C.accent,
            transform: `${kind === "gate" ? "rotate(45deg) " : ""}scale(${(kind === "gate" ? 0.8 : 1) * t})` }} />;
        })}
      </div>
    </div>
  );
}

function ShipStats({ stats }) {
  const frame = useCurrentFrame();
  const pct = stats.total ? stats.met / stats.total : 0;
  const shown = pct * interpolate(frame, [20, 60], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 40 }}>
      <div style={{ position: "relative", width: 280, height: 280 }}>
        <svg width="280" height="280" viewBox="0 0 120 120" style={{ transform: "rotate(-90deg)" }} aria-hidden="true">
          <circle cx="60" cy="60" r="52" fill="none" stroke={C["surface-2"]} strokeWidth="10" />
          <circle cx="60" cy="60" r="52" fill="none" stroke={C.ok} strokeWidth="10" strokeLinecap="round" pathLength="100" strokeDasharray="100" strokeDashoffset={100 - shown * 100} />
        </svg>
        <div style={{ position: "absolute", inset: 0, display: "grid", placeContent: "center", textAlign: "center" }}>
          <b style={{ fontSize: 60 }}>{stats.met}/{stats.total}</b>
          <span style={{ fontSize: 26, color: C["ink-2"] }}>Done when met</span>
        </div>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 16, alignItems: "flex-start" }}>
        {stats.verdicts.map((v, i) => <Verdict key={i} verdict={v} />)}
        <p style={{ margin: 0, fontSize: 34, color: C["ink-2"] }}><b style={{ color: C.ink, fontSize: 56 }}>{stats.unresolved}</b> unresolved {stats.unresolved === 1 ? "finding" : "findings"}</p>
      </div>
    </div>
  );
}

function EndScene({ scene }) {
  return (
    <AbsoluteFill style={{ padding: "0 104px", display: "flex", flexDirection: "column", justifyContent: "center", gap: 24 }}>
      <p style={{ margin: 0, color: C.accent, fontWeight: 700, letterSpacing: "0.08em", textTransform: "uppercase", fontSize: TYPE.small, ...useRise(0) }}>Open the page</p>
      <h1 style={{ margin: 0, fontSize: 96, lineHeight: 1.05, ...useRise(4) }}>{scene.title}</h1>
      <p style={{ margin: 0, fontSize: 44, color: C["ink-2"], ...useRise(10) }}>{scene.text}</p>
    </AbsoluteFill>
  );
}
