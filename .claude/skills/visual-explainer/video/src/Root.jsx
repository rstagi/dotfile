// Two compositions, one per mode. The input props come from render-video.mjs: { mode, meta, kinds,
// graph, stops } (the graph of the page and the resolved tour stops), plus `voiceover`
// ({ <stop id>: { src, seconds } }) and `fps`. The narration sets the length.
import { Composition } from "remotion";
import { Explainer } from "./Explainer.jsx";
import { SAMPLE_PLAN, SAMPLE_SHIP } from "./samples.mjs";
import { FPS, HEIGHT, WIDTH, storyboard } from "./storyboard.mjs";

const calculateMetadata = ({ props }) => {
  const board = storyboard(props, props.voiceover, { fps: props.fps ?? FPS });
  return { durationInFrames: board.durationInFrames, fps: board.fps };
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
