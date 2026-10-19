// The composition: the graph with its camera, one card and one voice clip for each stop, a header
// and a progress bar.
import { useMemo } from "react";
import { AbsoluteFill, Html5Audio, Sequence, staticFile, useCurrentFrame, useVideoConfig } from "remotion";
import { StopCard } from "./card.jsx";
import { GraphCanvas } from "./graph.jsx";
import { stopIndexAt, storyboard } from "./storyboard.mjs";
import { COLORS as C, FONTS } from "./theme.mjs";

export function Explainer(props) {
  const { fps } = useVideoConfig();
  const board = useMemo(() => storyboard(props, props.voiceover, { fps }), [props, fps]);
  return (
    <AbsoluteFill style={{ background: C.bg, color: C.ink, fontFamily: FONTS.sans }}>
      <GraphCanvas graph={props.graph} kinds={props.kinds} board={board} />
      {board.stops.map((stop) => (
        <Sequence key={stop.id} from={stop.from} durationInFrames={stop.durationInFrames} name={`stop ${stop.id}`}>
          <StopCard stop={stop} kinds={props.kinds} />
        </Sequence>
      ))}
      {board.stops.filter((s) => s.audio).map(({ id, audio }) => (
        <Sequence key={`voice-${id}`} from={audio.from} durationInFrames={audio.durationInFrames} name={`voice ${id}`}>
          <Html5Audio src={staticFile(audio.src)} />
        </Sequence>
      ))}
      <Header mode={props.mode} title={props.meta.title} board={board} />
      <Footer mode={props.mode} date={props.meta.date} total={board.durationInFrames} />
    </AbsoluteFill>
  );
}

function Header({ mode, title, board }) {
  const frame = useCurrentFrame();
  const i = stopIndexAt(board, frame);
  return (
    <div style={{ position: "absolute", left: 56, right: 56, top: 40, display: "flex", alignItems: "baseline", gap: 20 }}>
      <span style={{ color: C.accent, fontWeight: 700, letterSpacing: "0.08em", textTransform: "uppercase", fontSize: 26 }}>{mode === "plan" ? "Plan" : "Ship"}</span>
      <span style={{ fontSize: 34, fontWeight: 700, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{title}</span>
      <span style={{ marginLeft: "auto", fontFamily: FONTS.mono, color: C["ink-2"], fontSize: 26 }}>{i + 1} / {board.stops.length}</span>
    </div>
  );
}

function Footer({ mode, date, total }) {
  const frame = useCurrentFrame();
  return (
    <>
      <div style={{ position: "absolute", left: 56, bottom: 30, fontSize: 24, color: C.muted }}>visual-explainer · {mode} · {date}</div>
      <div style={{ position: "absolute", left: 0, right: 0, bottom: 0, height: 8, background: C["surface-2"] }}>
        <div style={{ height: "100%", width: `${(frame / Math.max(1, total - 1)) * 100}%`, background: C.accent }} />
      </div>
    </>
  );
}
