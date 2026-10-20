// The composition: one sequence for each scene (its title, its body and its captions), one voice
// clip for each narrated scene, and the header with the section progress bar.
import { useMemo } from "react";
import { AbsoluteFill, Html5Audio, interpolate, Sequence, staticFile, useCurrentFrame, useVideoConfig } from "remotion";
import { TextReveal } from "@/components/snap-cn/text-reveal";
import { WordCaptions } from "@/components/snap-cn/word-captions";
import { SnapCnUIProvider } from "@/lib/snap-cn-ui";
import { SCENE_VIEWS } from "./scenes.jsx";
import { modeLabel, sceneIndexAt, storyboard } from "./storyboard.mjs";
import { COLORS as C, FONTS, SNAP_THEME } from "./theme.mjs";

const FADE_FRAMES = 8;
const CAPTION_SIZE = 38;

export function Explainer(props) {
  const { fps } = useVideoConfig();
  const board = useMemo(() => storyboard(props, props.voiceover, { fps }), [props, fps]);
  return (
    <SnapCnUIProvider theme={SNAP_THEME} mode="dark">
      <AbsoluteFill style={{ background: C.bg, color: C.ink, fontFamily: FONTS.sans }}>
        {board.scenes.map((scene) => (
          <Sequence key={scene.id} from={scene.from} durationInFrames={scene.durationInFrames} name={`scene ${scene.id}`}>
            <Scene scene={scene} mode={props.mode} />
          </Sequence>
        ))}
        {board.scenes.filter((s) => s.audio).map(({ id, audio }) => (
          <Sequence key={`voice-${id}`} from={audio.from} durationInFrames={audio.durationInFrames} name={`voice ${id}`}>
            <Html5Audio src={staticFile(audio.src)} />
          </Sequence>
        ))}
        <Header label={modeLabel(props)} title={props.props.meta.title} board={board} />
      </AbsoluteFill>
    </SnapCnUIProvider>
  );
}

// One scene: a fade in and out, the numbered title, the body, and the burned-in captions.
function Scene({ scene, mode }) {
  const frame = useCurrentFrame();
  const opacity = interpolate(frame, [0, FADE_FRAMES, scene.durationInFrames - FADE_FRAMES, scene.durationInFrames], [0, 1, 1, 0],
    { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const View = SCENE_VIEWS[scene.id];
  return (
    <AbsoluteFill style={{ opacity }}>
      <View data={scene.data} mode={mode} frames={scene.durationInFrames} />
      {scene.title ? (
        <div style={{ position: "absolute", left: 120, right: 120, top: 112, height: 100 }}>
          <TextReveal text={`${scene.number} ${scene.title}`} fontSize={54} initialScale={1.6} holdDuration={6} />
        </div>
      ) : null}
      <WordCaptions words={scene.captions} preset="clean" activeStyle="color" accentColor={C.accent} fontSize={CAPTION_SIZE}
        maxChars={64} maxWords={12} maxWidth={1560} pillColor="rgba(5, 8, 12, 0.82)" textColor={C.ink} />
    </AbsoluteFill>
  );
}

// The top bar: one progress segment for each scene, the mode, the title and the scene counter.
function Header({ label, title, board }) {
  const frame = useCurrentFrame();
  const current = sceneIndexAt(board, frame);
  const scene = board.scenes[current];
  return (
    <>
      <div style={{ position: "absolute", left: 0, right: 0, top: 0, height: 8, display: "flex", gap: 6, padding: "0 6px" }}>
        {board.scenes.map((s, i) => {
          const fill = i < current ? 1 : i > current ? 0 : (frame - s.from) / s.durationInFrames;
          return (
            <div key={s.id} style={{ flex: s.durationInFrames, background: C["surface-2"], borderRadius: 4, overflow: "hidden" }}>
              <div style={{ width: `${Math.min(1, fill) * 100}%`, height: "100%", background: C.accent }} />
            </div>
          );
        })}
      </div>
      <div style={{ position: "absolute", left: 120, right: 120, top: 36, opacity: scene.id === "outro" ? 0 : 1, display: "flex", alignItems: "center", gap: 18, fontSize: 26, color: C.muted }}>
        <span style={{ color: C.accent, fontWeight: 700, letterSpacing: "0.08em", textTransform: "uppercase", fontSize: 22 }}>{label}</span>
        <span style={{ whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis", maxWidth: 1200 }}>{title}</span>
        <span style={{ marginLeft: "auto", fontFamily: FONTS.mono, fontSize: 24 }}>{scene.label} · {current + 1}/{board.scenes.length}</span>
      </div>
    </>
  );
}
