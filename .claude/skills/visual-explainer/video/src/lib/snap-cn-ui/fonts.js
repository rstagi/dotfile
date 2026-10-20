// snapcn UI Core · snapcn — https://snapcn.dev/docs/components
"use client";


/**
 * The type faces a scene's `fontFamily` may name.
 *
 * ## Why an allowlist and not a free string
 *
 * Every component in this registry loads its face through
 * `@remotion/google-fonts` rather than a CSS variable, for the reason spelled
 * out at the top of each one: a Remotion bundle has none of the app's CSS, so
 * `var(--font-…)` gets you the right face in the Player and a **fallback in the
 * mp4**. A bare family name handed to a component has exactly the same problem
 * — the browser preview finds it, the render does not, and the bug is invisible
 * until someone plays the file.
 *
 * So a name only counts as a font here if this module has loaded it. Values map
 * label → the real CSS family string `loadFont` returns, which is not always the
 * label ("Instrument Serif", not "InstrumentSerif").
 *
 * ## A raw stack still works
 *
 * `resolveFont` passes anything it does not recognise straight through, so a
 * caller who has loaded their own face — a brand font, a local `@font-face` —
 * names it directly and gets it. The allowlist is the safe path, not the only one.
 *
 * ponytail: all six load at module scope, so a render fetches six latin subsets
 * whether or not the video uses them. Move to a lazy per-family `loadFont()` if
 * bundle time ever shows up in a render trace.
 */
// visual-explainer patch: every name maps to a system stack. The upstream file loads six
// Google fonts at module scope, which makes each render fetch them over the network.
const SYSTEM_SANS = 'ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif';
export const FONTS = {
  Inter: SYSTEM_SANS,
  Geist: SYSTEM_SANS,
  "Space Grotesk": SYSTEM_SANS,
  Outfit: SYSTEM_SANS,
  Montserrat: SYSTEM_SANS,
  "Instrument Serif": 'ui-serif, Georgia, serif',
};

/**
 * What a select control offers. `DEFAULT_FONT` first: a scene's own face is a
 * design decision, so the knob has to have an "off", and it has to be the
 * value it ships at.
 */
export const DEFAULT_FONT = "Default";
export const FONT_NAMES = [DEFAULT_FONT, ...Object.keys(FONTS)];

/**
 * Label → loaded CSS family. Anything unrecognised passes through unchanged, so
 * a caller's own stack is never swallowed. `undefined` in, `undefined` out —
 * that is the signal for "keep the component's own face".
 */
export const resolveFont = name => !name || name === DEFAULT_FONT ? undefined : (FONTS[name] ?? name);
