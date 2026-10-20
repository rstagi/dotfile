// snapcn UI Core · snapcn — https://snapcn.dev/docs/components
import { useCurrentFrame, useVideoConfig } from "remotion";

export function framesFor(d, fps) {
  return typeof d === "number" ? d : Math.round(d.seconds * fps);
}

export function revealCount(localFrame, fps, len, cps) {
  const over = (len / cps) * fps;
  if (over <= 0) return len;
  return Math.max(0, Math.min(len, Math.floor((localFrame / over) * len)));
}

export function clamp01(t) {
  return Math.max(0, Math.min(1, t));
}

export function revealedText(full, count) {
  const c = Math.max(0, Math.min(full.length, Math.floor(count)));
  return full.slice(0, c);
}

export function useTypewriter(full, options = {}) {
  const { cps = 20, speed = 1, startFrame = 0 } = options;
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const local = frame * speed - startFrame;
  const count = local <= 0 ? 0 : revealCount(local, fps, full.length, cps);
  return {
    text: revealedText(full, count),
    count,
    done: count >= full.length,
    typing: count > 0 && count < full.length,
  };
}

export function useCurrentState(steps, defaultState, speed = 1) {
  const effectiveFrame = useCurrentFrame() * speed;
  let current = defaultState;
  let bestAt = -Infinity;
  steps.forEach((step) => {
    if (step.at <= effectiveFrame && step.at >= bestAt) {
      bestAt = step.at;
      current = step.state;
    }
  });
  return current;
}

export function useStateTransition(steps, defaultState, speed = 1, defaultDuration = 8) {
  const effectiveFrame = useCurrentFrame() * speed;
  const started = steps
    .map((step, index) => ({ step, index }))
    .sort((a, b) => a.step.at - b.step.at || a.index - b.index)
    .filter((e) => e.step.at <= effectiveFrame)
    // Same-`at` ties: the later array entry wins and the earlier tied
    // entries never display, so they can never act as a `from` state.
    .filter(
      (e, i, arr) => i === arr.length - 1 || arr[i + 1].step.at !== e.step.at,
    );
  if (started.length === 0)
    return { from: defaultState, to: defaultState, progress: 1 };
  const to = started[started.length - 1].step;
  const from = started.length >= 2 ? started[started.length - 2].step : null;
  const dur = to.duration ?? defaultDuration;
  const progress = dur > 0 ? clamp01((effectiveFrame - to.at) / dur) : 1;
  return { from: from ? from.state : defaultState, to: to.state, progress };
}
