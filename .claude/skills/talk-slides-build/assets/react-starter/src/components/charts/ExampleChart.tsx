/* A decluttered, hand-built example chart — the conventions to copy for every
   chart you add: NO gridlines, NO legend, NO border; direct labels; tabular
   mono numerals; ONE accent on the takeaway, everything else muted; sized via a
   fixed viewBox + width:100% so it scales with the stage. Add real charts as
   sibling components and switch on them in DataScene by the slide's `chart` id. */
const LBL = { fontFamily: 'var(--talk-font-mono)', fontSize: '22px' } as const

export function ExampleChart() {
  const yOf = (v: number) => 360 - (v / 100) * 290
  return (
    <svg viewBox="0 0 760 420" width="100%" style={{ display: 'block', overflow: 'visible' }}>
      <line x1="120" y1="360" x2="640" y2="360" stroke="var(--talk-rule)" strokeWidth="2" />

      {/* "before" — non-focal */}
      <rect x="190" y={yOf(38)} width="130" height={360 - yOf(38)} fill="var(--talk-muted)" opacity="0.55" />
      <text x="255" y={yOf(38) - 14} textAnchor="middle" fill="var(--talk-muted)" {...LBL}>38</text>
      <text x="255" y="392" textAnchor="middle" fill="var(--talk-muted)" {...LBL}>before</text>

      {/* "after" — the takeaway, in the accent */}
      <rect x="430" y={yOf(82)} width="130" height={360 - yOf(82)} fill="var(--talk-accent)" />
      <text x="495" y={yOf(82) - 14} textAnchor="middle" fill="var(--talk-accent)" {...LBL}>82</text>
      <text x="495" y="392" textAnchor="middle" fill="var(--talk-ink)" {...LBL}>after</text>
    </svg>
  )
}
