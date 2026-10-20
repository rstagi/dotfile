// The dark color tokens of template/report.html (tests/visual-explainer-video.test.mjs fails when
// the two differ), and the snapcn theme that maps them onto the shadcn token
// names of the vendored components (lib/snap-cn-ui/theme.js).
export const COLORS = {
  bg: "#0d1117",
  surface: "#151b23",
  "surface-2": "#1c232c",
  ink: "#e6edf3",
  "ink-2": "#9ea7b3",
  muted: "#7d8590",
  line: "#3d444d",
  "line-2": "#2a313b",
  accent: "#4493f8",
  "accent-bg": "#142a45",
  "accent-line": "#1f4b80",
  "callout-bg": "#111f33",
  "code-bg": "#262c36",
  ok: "#3fb950",
  "ok-bg": "#12261e",
  "ok-line": "#2a6a3a",
  warn: "#d29922",
  "warn-bg": "#2a2111",
  "warn-line": "#6b5216",
  bad: "#f85149",
  "bad-bg": "#2d1316",
  "bad-line": "#7a2a2a",
  orange: "#f0883e",
  "orange-bg": "#2b1b0e",
  "orange-line": "#7a4318",
};

export const FONTS = {
  sans: 'ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif',
  mono: 'ui-monospace, "SF Mono", Menlo, Consolas, monospace',
};

export const SNAP_THEME = {
  background: COLORS.bg,
  foreground: COLORS.ink,
  card: COLORS.surface,
  cardForeground: COLORS.ink,
  primary: COLORS.accent,
  primaryForeground: COLORS.bg,
  secondary: COLORS["surface-2"],
  muted: COLORS["surface-2"],
  mutedForeground: COLORS.muted,
  border: COLORS.line,
  ring: COLORS.accent,
  fontFamily: FONTS.sans,
};

// The tone of each status, severity and component status: [ink, background].
export const TONES = {
  met: ["ok", "ok-bg"], partial: ["warn", "warn-bg"], missed: ["bad", "bad-bg"],
  blocker: ["bad", "bad-bg"], major: ["bad", "bad-bg"], minor: ["orange", "orange-bg"], nit: ["muted", "surface-2"], style: ["muted", "surface-2"],
  open: ["accent", "accent-bg"], pending: ["muted", "surface-2"], deferred: ["warn", "warn-bg"], fixed: ["ok", "ok-bg"],
  new: ["ok", "ok-bg"], changed: ["warn", "warn-bg"], existing: ["ink-2", "surface-2"],
  ok: ["ok", "ok-bg"], bad: ["bad", "bad-bg"],
};
