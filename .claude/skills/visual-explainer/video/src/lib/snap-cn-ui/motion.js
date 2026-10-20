// snapcn UI Core · snapcn — https://snapcn.dev/docs/components
export const easings = {
  linear: t => t,
  out: t => 1 - (1 - t) ** 3,
  in: t => t * t * t,
  inOut: t => t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2
};

export const springs = {
  snappy: { damping: 18, stiffness: 220, mass: 0.7 },
  soft: { damping: 14, stiffness: 120, mass: 0.9 },
  bouncy: { damping: 10, stiffness: 180, mass: 0.8 }
};
