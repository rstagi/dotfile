// Shared scene parts: inline code, chips, the entry motion and the fit of a text to a box.
import { interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { COLORS as C, FONTS, TONES } from "./theme.mjs";

// Plain text with `code` spans.
export function Inline({ text }) {
  return String(text).split("`").map((part, i) => (i % 2
    ? <code key={i} style={{ fontFamily: FONTS.mono, fontSize: "0.86em", background: C["code-bg"], border: `1px solid ${C.line}`, borderRadius: 6, padding: "0 0.25em" }}>{part}</code>
    : <span key={i}>{part}</span>));
}

// A rounded chip in the tone of a status, a severity or a component status.
export function Chip({ tone, children, size = 22, mono = false }) {
  const [ink, bg] = TONES[tone] ?? ["ink-2", "surface-2"];
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 8, fontSize: size, fontWeight: 600, lineHeight: 1.2, whiteSpace: "nowrap",
      color: C[ink], background: C[bg], border: `1px solid ${C[ink]}55`, borderRadius: 999, padding: "4px 14px",
      fontFamily: mono ? FONTS.mono : FONTS.sans,
    }}>{children}</span>
  );
}

// Text clamped to `lines` lines with an ellipsis.
export function Clamp({ lines, style, children }) {
  return <div style={{ display: "-webkit-box", WebkitBoxOrient: "vertical", WebkitLineClamp: lines, overflow: "hidden", ...style }}>{children}</div>;
}

// "+N more" under a list.
export function More({ n, what = "more" }) {
  if (!n) return null;
  return <div style={{ color: C.muted, fontSize: 24, marginTop: 10 }}>+{n} {what}</div>;
}

// Entry motion: rise and fade in after `delay` frames of the current sequence.
export function useRise(delay = 0) {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t = spring({ frame, fps, delay, config: { damping: 200 }, durationInFrames: Math.round(0.6 * fps) });
  return { opacity: t, transform: `translateY(${(1 - t) * 24}px)` };
}

// Progress from 0 to 1 over `frames` frames after `delay`.
export function useProgress(delay, frames) {
  const frame = useCurrentFrame();
  return interpolate(frame, [delay, delay + frames], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
}

// fitFont(text, box, { min, max, lineHeight, charWidth }) -> the largest font size in [min, max]
// whose estimated wrapped height fits the box. `charWidth` is the mean glyph width per em.
export function fitFont(text, box, { min = 26, max = 48, lineHeight = 1.3, charWidth = 0.5 } = {}) {
  const chars = String(text).length;
  for (let size = max; size > min; size -= 2) {
    const perLine = Math.max(1, Math.floor(box.w / (size * charWidth)));
    if (Math.ceil(chars / perLine) * size * lineHeight <= box.h) return size;
  }
  return min;
}
