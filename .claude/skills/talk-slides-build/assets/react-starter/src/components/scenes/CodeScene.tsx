import { Fragment } from 'react'
import type { CodeSlide } from '../../data/slides'

/* Code walkthrough for technical talks. Keep code MONOCHROME (ink on surface);
   reserve the accent for ONE thing — the token(s) in `highlight`, revealed one
   per subStep as it travels through the snippet (set steps = highlight.length).
   Don't read the code aloud; narrate the one idea it proves. */
function escapeRe(s: string) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

export function CodeScene({ content, subStep }: { content: CodeSlide['content']; subStep: number }) {
  const active = (content.highlight ?? []).slice(0, subStep)
  const re = active.length ? new RegExp(`(${active.map(escapeRe).join('|')})`, 'g') : null

  const rendered = re
    ? content.code.split(re).map((part, i) =>
        active.includes(part) ? (
          <span
            key={i}
            style={{ color: 'var(--talk-accent)', background: 'rgba(216,100,46,0.16)', fontWeight: 700, borderRadius: 3, padding: '0 0.2em' }}
          >
            {part}
          </span>
        ) : (
          <Fragment key={i}>{part}</Fragment>
        )
      )
    : content.code

  return (
    <div className="scene scene--flow" style={{ paddingTop: '12cqh' }}>
      <h2 className="flow-head" style={{ marginBottom: '2cqh', maxWidth: '40ch' }}>{content.assertion}</h2>
      <pre
        style={{
          fontFamily: 'var(--talk-font-mono)',
          fontSize: '1.34cqw',
          lineHeight: 1.4,
          color: 'var(--talk-ink)',
          background: 'var(--talk-surface)',
          border: '1px solid var(--talk-rule)',
          borderLeft: '3px solid var(--talk-accent)',
          borderRadius: 6,
          padding: '1.8cqw 2.2cqw',
          maxWidth: '62cqw',
          overflow: 'hidden',
          whiteSpace: 'pre',
        }}
      >
        {rendered}
      </pre>
    </div>
  )
}
