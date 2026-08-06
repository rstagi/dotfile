import type { ReactNode } from 'react'
import type { Section } from '../../data/sections'

interface Props {
  section: Section
  index: number // 0-based
  total: number
  visible: boolean
  /** counter label — rename per the talk's world (e.g. "Folio") */
  label?: string
  /** optional top-right slot — the per-talk world fills this (a sigil, a status, …) */
  badge?: ReactNode
}

/* Persistent on-stage chrome: the current section (top-left), an optional
   per-talk badge (top-right), the slide counter + a progress rule (bottom).
   Engine-level and world-agnostic — the per-talk world dresses it via `badge`
   and by re-skinning the CSS tokens. */
export function DeckChrome({ section, index, total, visible, label = 'Slide', badge }: Props) {
  const progress = (index + 1) / total

  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        zIndex: 5,
        pointerEvents: 'none',
        opacity: visible ? 1 : 0,
        transition: 'opacity 0.4s ease',
      }}
    >
      {/* top-left — current section */}
      <div style={{ position: 'absolute', top: '5cqh', left: 'var(--margin)' }}>
        <div className="eyebrow-row">
          <span style={{ width: '2.6cqw', height: 1, background: 'var(--talk-rule)' }} />
          <span style={{ fontFamily: 'var(--talk-font-display)', fontSize: '1.9cqw', fontStyle: 'italic', color: 'var(--talk-ink)' }}>
            {section.label}
          </span>
        </div>
      </div>

      {/* top-right — optional per-talk badge */}
      {badge && <div style={{ position: 'absolute', top: '5cqh', right: 'var(--margin)' }}>{badge}</div>}

      {/* bottom-left — counter */}
      <div
        style={{
          position: 'absolute',
          bottom: '5cqh',
          left: 'var(--margin)',
          fontFamily: 'var(--talk-font-mono)',
          fontSize: 'var(--fs-kicker)',
          letterSpacing: '0.18em',
          textTransform: 'uppercase',
          color: 'var(--talk-muted)',
        }}
      >
        {label} {String(index + 1).padStart(2, '0')} / {total}
      </div>

      {/* bottom-right — nav hint */}
      <div
        style={{
          position: 'absolute',
          bottom: '5cqh',
          right: 'var(--margin)',
          fontFamily: 'var(--talk-font-mono)',
          fontSize: 'var(--fs-kicker)',
          letterSpacing: '0.14em',
          color: 'var(--talk-rule)',
        }}
      >
        ← → · N for notes
      </div>

      {/* progress rule */}
      <div style={{ position: 'absolute', left: 0, right: 0, bottom: 0, height: 2, background: 'var(--talk-rule)' }}>
        <div
          style={{
            height: '100%',
            width: `${progress * 100}%`,
            background: 'var(--talk-accent)',
            opacity: 0.8,
            transition: 'width 0.5s ease',
          }}
        />
      </div>
    </div>
  )
}
