// Shared scene parts: the scene frame, inline code, chips, pills and the entry motion.
import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { COLORS as C, FONTS, TYPE } from "./theme.mjs";

const PAD_X = 104;
const STAGGER_FRAMES = 6;

export const STATUS_COLOR = { met: C.ok, partial: C.warn, missed: C.bad, new: C.ok, changed: C.warn, existing: C.muted, removed: C.bad };
export const LEVEL_COLOR = { high: C.bad, medium: C.warn, low: C.ok, pass: C.ok, fail: C.bad, skipped: C.warn, open: C.bad, deferred: C.warn, fixed: C.ok };
export const SEVERITY_COLOR = { blocker: C.bad, major: C.warn, minor: C.info, nit: C.muted, style: C.line };
export const VERDICT = { APPROVE: ["Approved", C.ok], COMMENT: ["Comment", C.warn], REQUEST_CHANGES: ["Changes requested", C.bad] };

// One scene: the section kicker and title, the body, the "+N more on the page" chip, a fade out.
export function SceneFrame({ scene, aside, children }) {
  const frame = useCurrentFrame();
  const fadeOut = interpolate(frame, [scene.durationInFrames - 10, scene.durationInFrames], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const part = scene.part?.count > 1 ? `${scene.part.index} / ${scene.part.count}` : "";
  return (
    <AbsoluteFill style={{ padding: `72px ${PAD_X}px 96px`, opacity: fadeOut, display: "flex", flexDirection: "column", gap: 36 }}>
      <div style={{ display: "flex", alignItems: "baseline", gap: 28, ...useRise(0) }}>
        <span style={{ fontFamily: FONTS.mono, color: C.accent, fontSize: TYPE.small }}>{String(scene.section.number).padStart(2, "0")}</span>
        <h2 style={{ margin: 0, fontSize: TYPE.title, letterSpacing: "-0.01em", lineHeight: 1.1 }}>{scene.section.title}</h2>
        {part && <span style={{ color: C.muted, fontSize: TYPE.small }}>{part}</span>}
        <span style={{ marginLeft: "auto" }}>{aside}</span>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "hidden", display: "flex", flexDirection: "column", gap: 28 }}>{children}</div>
      {scene.more > 0 && <More n={scene.more} />}
    </AbsoluteFill>
  );
}

export function More({ n, inline = false }) {
  const style = useRise(inline ? 12 : 20);
  return (
    <span style={{ alignSelf: inline ? "flex-start" : "flex-end", color: C.accent, border: `2px solid ${C.accent}`, borderRadius: 999,
      padding: "4px 22px", fontSize: inline ? 30 : TYPE.small, fontWeight: 600, ...style }}>+{n} more on the page</span>
  );
}

// Plain text with `code` spans.
export function Inline({ text }) {
  return String(text).split("`").map((part, i) => (i % 2
    ? <code key={i} style={{ fontFamily: FONTS.mono, fontSize: "0.86em", background: C["code-bg"], border: `1px solid ${C.line}`, borderRadius: 8, padding: "0 0.25em" }}>{part}</code>
    : <span key={i}>{part}</span>));
}

export function Chip({ children, mono = false, color = C["ink-2"] }) {
  return (
    <span style={{ display: "inline-block", padding: "4px 18px", borderRadius: 999, border: `2px solid ${C.line}`, background: C.surface,
      color, fontSize: 30, fontFamily: mono ? FONTS.mono : FONTS.sans, whiteSpace: "nowrap" }}>{children}</span>
  );
}

export function Pill({ children, color }) {
  return (
    <span style={{ display: "inline-block", fontSize: 26, fontWeight: 700, textTransform: "uppercase", letterSpacing: "0.04em",
      padding: "2px 12px", borderRadius: 8, border: `2px solid ${color}`, color, whiteSpace: "nowrap" }}>{children}</span>
  );
}

export function Badge({ children, background = C["surface-2"], color = C["ink-2"] }) {
  return <span style={{ fontSize: 26, padding: "3px 14px", borderRadius: 999, background, color, fontWeight: 600, whiteSpace: "nowrap" }}>{children}</span>;
}

export function Verdict({ verdict }) {
  const [label, color] = VERDICT[verdict];
  return <span style={{ display: "inline-block", fontWeight: 700, padding: "6px 24px", borderRadius: 999, background: color, color: C["on-color"], fontSize: TYPE.small }}>{label}</span>;
}

export function Card({ children, index = 0, accent, style }) {
  return (
    <div style={{ background: C.surface, border: `2px solid ${C.line}`, borderLeft: accent ? `8px solid ${accent}` : `2px solid ${C.line}`,
      borderRadius: 20, padding: "24px 30px", minWidth: 0, ...useRise(8 + index * STAGGER_FRAMES), ...style }}>{children}</div>
  );
}

export function Label({ children }) {
  return <p style={{ margin: 0, fontSize: 26, textTransform: "uppercase", letterSpacing: "0.06em", color: C.muted, fontWeight: 700 }}>{children}</p>;
}

// A numbered step with its command lines.
export function Step({ n, color, index, children }) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "64px minmax(0, 1fr)", gap: 24, ...useRise(8 + index * STAGGER_FRAMES) }}>
      <span style={{ width: 64, height: 64, borderRadius: "50%", background: color, color: C.bg, display: "grid", placeContent: "center", fontWeight: 700, fontSize: 32 }}>{n}</span>
      <div style={{ background: C.surface, border: `2px solid ${C.line}`, borderRadius: 20, padding: "18px 26px", minWidth: 0 }}>{children}</div>
    </div>
  );
}

export function Command({ command }) {
  return (
    <div style={{ background: C["code-bg"], border: `2px solid ${C.line}`, borderRadius: 14, padding: "12px 20px", margin: "10px 0", fontFamily: FONTS.mono, fontSize: 30, lineHeight: 1.4 }}>
      {command.lines.map((line, i) => <div key={i} style={{ whiteSpace: "pre" }}>{line}</div>)}
      {command.more > 0 && <div style={{ color: C.muted, fontFamily: FONTS.sans, fontSize: 26 }}>+{command.more} more {command.more === 1 ? "line" : "lines"} on the page</div>}
    </div>
  );
}

// Entry motion: rise and fade in after `delay` frames.
export function useRise(delay) {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t = spring({ frame, fps, delay, config: { damping: 200 } });
  return { opacity: t, transform: `translateY(${(1 - t) * 28}px)` };
}

export function stagger(index) {
  return 8 + index * STAGGER_FRAMES;
}
