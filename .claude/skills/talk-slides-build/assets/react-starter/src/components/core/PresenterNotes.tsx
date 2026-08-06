import { motion, AnimatePresence } from 'framer-motion'

export interface SlideNotes {
  /** trigger-word cue lines, lifted verbatim from 03-delivery-plan.md */
  cues: string[]
  /** a verbatim hand-written line to deliver (opening / closing) */
  line?: string
  memorable?: boolean
}

interface Props {
  notes?: SlideNotes
  title: string
  index: number
  total: number
  visible: boolean
  /** counter label — rename per the talk's world (e.g. "Folio") */
  label?: string
}

/* Toggleable presenter overlay (press N or S). Ships the delivery-plan cue
   cards inside the deck. It is part of the same tab, so toggle it OFF before
   recording / screen-sharing the take. */
export function PresenterNotes({ notes, title, index, total, visible, label = 'Slide' }: Props) {
  return (
    <AnimatePresence>
      {visible && (
        <motion.div
          initial={{ opacity: 0, y: 30 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 30 }}
          transition={{ duration: 0.25 }}
          style={{
            position: 'absolute',
            left: '4cqw',
            right: '4cqw',
            bottom: '3cqh',
            zIndex: 20,
            background: 'rgba(8,6,4,0.93)',
            border: '1px solid var(--talk-rule)',
            borderRadius: 6,
            padding: '2cqw 2.4cqw',
            backdropFilter: 'blur(6px)',
            maxHeight: '42cqh',
            overflow: 'auto',
          }}
        >
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', marginBottom: '1cqw' }}>
            <span
              style={{
                fontFamily: 'var(--talk-font-mono)',
                fontSize: '1.2cqw',
                letterSpacing: '0.18em',
                textTransform: 'uppercase',
                color: notes?.memorable ? 'var(--talk-accent)' : 'var(--talk-muted)',
              }}
            >
              {notes?.memorable ? '★ Memorable moment · ' : 'Notes · '}
              {label} {String(index + 1).padStart(2, '0')} / {total}
            </span>
            <span style={{ fontFamily: 'var(--talk-font-mono)', fontSize: '1.1cqw', color: 'var(--talk-rule)' }}>
              N / S to hide
            </span>
          </div>

          <div style={{ fontFamily: 'var(--talk-font-display)', fontSize: '1.7cqw', color: 'var(--talk-ink-bright)', marginBottom: '1cqw' }}>
            {title}
          </div>

          {notes?.line && (
            <div
              style={{
                fontFamily: 'var(--talk-font-display)',
                fontStyle: 'italic',
                fontSize: '1.7cqw',
                lineHeight: 1.4,
                color: 'var(--talk-accent)',
                borderLeft: '2px solid var(--talk-accent)',
                paddingLeft: '1.2cqw',
                marginBottom: '1.2cqw',
              }}
            >
              {notes.line}
            </div>
          )}

          <ul style={{ listStyle: 'none', display: 'flex', flexDirection: 'column', gap: '0.6cqw' }}>
            {notes?.cues.map((c, i) => (
              <li
                key={i}
                style={{ fontFamily: 'var(--talk-font-body)', fontSize: '1.5cqw', lineHeight: 1.35, color: 'var(--talk-ink)', paddingLeft: '1.4cqw', position: 'relative' }}
              >
                <span style={{ position: 'absolute', left: 0, color: 'var(--talk-muted)' }}>·</span>
                {c}
              </li>
            ))}
          </ul>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
