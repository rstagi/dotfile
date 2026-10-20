// Two compositions, one per mode. The input props come from render-video.mjs: { mode, props,
// scenes, shots } (the props JSON, the narration scenes and the screenshots of the report page),
// plus `voiceover` ({ <scene id>: { src, seconds } }) and `fps`. The narration sets the length.
import { Composition } from "remotion";
import { Explainer } from "./Explainer.jsx";
import { FPS, HEIGHT, WIDTH, storyboard } from "./storyboard.mjs";

const calculateMetadata = ({ props }) => {
  const board = storyboard(props, props.voiceover, { fps: props.fps ?? FPS });
  return { durationInFrames: board.durationInFrames, fps: board.fps };
};

// Default props for the Remotion Studio preview. A real render replaces every key, so no sample
// value can leak into a video.
const meta = { title: "Sample", subtitle: "Studio preview props", date: "2026-10-21" };
const SAMPLE_PLAN = {
  mode: "plan", shots: {}, voiceover: {},
  scenes: [{ id: "intro", text: "A sample plan." }, { id: "phases", text: "One phase." }],
  props: { mode: "plan", meta, goal: "Preview", architecture: { summary: "-", layers: [{ name: "L", components: [{ name: "C", role: "-" }] }] },
    milestones: [{ number: 1, title: "M", outcome: "-", phases: [{ number: 1, title: "Phase", kind: "work" }] }], decisions: [], risks: [] },
};
const SAMPLE_SHIP = {
  mode: "ship", shots: {}, voiceover: {},
  scenes: [{ id: "intro", text: "A sample review." }],
  props: { mode: "ship", meta, summary: "Preview", delivered: [], deviations: [], reviews: [], handTest: [], rollback: [] },
};

export function Root() {
  return (
    <>
      <Composition id="plan" component={Explainer} width={WIDTH} height={HEIGHT} fps={FPS} durationInFrames={60 * FPS}
        defaultProps={SAMPLE_PLAN} calculateMetadata={calculateMetadata} />
      <Composition id="ship" component={Explainer} width={WIDTH} height={HEIGHT} fps={FPS} durationInFrames={60 * FPS}
        defaultProps={SAMPLE_SHIP} calculateMetadata={calculateMetadata} />
    </>
  );
}
