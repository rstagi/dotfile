import { motion } from 'framer-motion'
import type { QuoteSlide } from '../../data/slides'

/* A pull-quote that earns a turn in the argument. World-agnostic typography. */
export function QuoteScene({ content }: { content: QuoteSlide['content'] }) {
  return (
    <div className="scene">
      <motion.div initial={{ opacity: 0, y: 14 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.6 }}>
        <div
          aria-hidden
          style={{
            fontFamily: 'var(--talk-font-display)',
            fontWeight: 300,
            fontSize: '7cqw',
            lineHeight: 0.7,
            color: 'var(--talk-accent)',
            marginBottom: '0.8cqw',
          }}
        >
          &ldquo;
        </div>
        <blockquote className="statement" style={{ fontStyle: 'italic', fontWeight: 300, maxWidth: '24ch' }}>
          {content.quote}
        </blockquote>
        <div className="byline" style={{ marginTop: '2.6cqw' }}>{content.attribution}</div>
        {content.note && (
          <p className="body muted" style={{ marginTop: '1.2cqw', fontStyle: 'italic' }}>{content.note}</p>
        )}
      </motion.div>
    </div>
  )
}
