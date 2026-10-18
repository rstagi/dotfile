// The dark color tokens of template/page.html. tests/visual-explainer-video.test.mjs fails when the two differ.
export const COLORS = {
  bg: "#111215",
  surface: "#1a1c21",
  "surface-2": "#23262d",
  ink: "#ecebe6",
  "ink-2": "#aeaca4",
  line: "#31343c",
  accent: "#7ea2ff",
  ok: "#4cc58a",
  warn: "#f0b155",
  bad: "#ff7466",
  info: "#ad98ff",
  muted: "#8d8b84",
  "code-bg": "#15171b",
  "on-color": "#111215",
};

export const FONTS = {
  sans: 'ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif',
  mono: 'ui-monospace, "SF Mono", Menlo, Consolas, monospace',
};

// Sizes in px at 1920x1080. The smallest text stays above 16 px on a 6-inch phone in landscape.
export const TYPE = {
  hero: 112,
  title: 76,
  lead: 54,
  body: 40,
  small: 34,
};
