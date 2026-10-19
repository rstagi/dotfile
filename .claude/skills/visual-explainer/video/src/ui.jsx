// Shared card parts: inline code and the entry motion.
import { spring, useCurrentFrame, useVideoConfig } from "remotion";
import { COLORS as C, FONTS } from "./theme.mjs";

// Plain text with `code` spans.
export function Inline({ text }) {
  return String(text).split("`").map((part, i) => (i % 2
    ? <code key={i} style={{ fontFamily: FONTS.mono, fontSize: "0.86em", background: C["code-bg"], border: `1px solid ${C.line}`, borderRadius: 8, padding: "0 0.25em" }}>{part}</code>
    : <span key={i}>{part}</span>));
}

// Entry motion: rise and fade in after `delay` frames of the current sequence.
export function useRise(delay) {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t = spring({ frame, fps, delay, config: { damping: 200 } });
  return { opacity: t, transform: `translateY(${(1 - t) * 24}px)` };
}
