import { motion, AnimatePresence } from 'framer-motion'
import type { StatementSlide } from '../../data/slides'
import { renderRich } from '../core/rich'

/* The spine beat. Left-aligned. The canonical subStep-reveal demo: subStep 1
   reveals the trailing clause; an optional muted anchor lands after. */
export function StatementScene({ content, subStep }: { content: StatementSlide['content']; subStep: number }) {
  return (
    <div className="scene">
      {content.kicker && <div className="kicker" style={{ marginBottom: '1.4cqw' }}>{content.kicker}</div>}

      <h2 className="statement">
        {renderRich(content.main)}
        {content.reveal && (
          <AnimatePresence>
            {subStep >= 1 && (
              <motion.span
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.45 }}
                style={{ display: 'inline' }}
              >
                {' '}
                {renderRich(content.reveal)}
              </motion.span>
            )}
          </AnimatePresence>
        )}
      </h2>

      {content.anchor && (
        <AnimatePresence>
          {subStep >= 1 && (
            <motion.p
              className="num muted"
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.45, delay: 0.1 }}
              style={{ fontSize: 'var(--fs-num)', marginTop: '3cqw', letterSpacing: '0.02em' }}
            >
              {content.anchor}
            </motion.p>
          )}
        </AnimatePresence>
      )}
    </div>
  )
}
