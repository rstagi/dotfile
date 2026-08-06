import { motion } from 'framer-motion'
import type { TitleSlide } from '../../data/slides'
import { renderRich } from '../core/rich'

/* Title / closing card. Left-aligned, hangs off the margin — not a centered
   default title slide. The per-talk world can add a decorative column to the
   right (see the GoT deck's throne); the generic version is title-block only. */
export function TitleScene({ content }: { content: TitleSlide['content'] }) {
  return (
    <div className="scene">
      <motion.div initial={{ opacity: 0, y: 16 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.6 }}>
        <div className="kicker">{content.kicker}</div>
        <div className="flourish" style={{ margin: '1.6cqw 0' }} />
        <h1 className="display">{renderRich(content.title)}</h1>
        {content.subtitle && (
          <p className="lead" style={{ marginTop: '1.6cqw' }}>{renderRich(content.subtitle)}</p>
        )}
        <p className="byline" style={{ marginTop: '2.6cqw' }}>{content.byline}</p>
      </motion.div>
    </div>
  )
}
