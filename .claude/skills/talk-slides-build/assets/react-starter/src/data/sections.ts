export interface Section {
  label: string
  /** inclusive 1-based slide range */
  range: [number, number]
}

/* The narrative arc — the chrome shows the current section's label.
   Rename these to your talk's acts/movements (the GoT deck called them
   "Books"; a journey talk might use "Act I / II / III"). */
export const SECTIONS: Section[] = [
  { label: 'Opening', range: [1, 2] },
  { label: 'The case', range: [3, 5] },
  { label: 'Close', range: [6, 6] },
]

export function sectionForSlide(oneBased: number): Section {
  return SECTIONS.find(s => oneBased >= s.range[0] && oneBased <= s.range[1]) ?? SECTIONS[0]
}
