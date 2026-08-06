import { motion } from 'framer-motion'
import type { TwoColSlide } from '../../data/slides'
import { renderRich } from '../core/rich'

/* Two-column compare. Use when two things GENUINELY differ. Color follows the
   concept via accent-1 / accent-2 (decide the meaning-mapping in the style spec). */
export function TwoColScene({ content }: { content: TwoColSlide['content'] }) {
  const cols = [content.left, content.right]
  return (
    <div className="scene scene--flow">
      <h2 className="flow-head" style={{ marginBottom: '5cqh', maxWidth: '38ch' }}>{renderRich(content.assertion)}</h2>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1px 1fr', gap: '3cqw', alignItems: 'start' }}>
        {[cols[0], 'divider', cols[1]].map((c, i) => {
          if (c === 'divider') {
            return <div key="div" style={{ width: 1, height: '24cqh', background: 'var(--talk-rule)', justifySelf: 'center' }} />
          }
          const col = c as TwoColSlide['content']['left']
          const color = col.accent === 'accent-1' ? 'var(--talk-accent)' : 'var(--talk-accent-2)'
          return (
            <motion.div
              key={col.label}
              initial={{ opacity: 0, y: 14 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.5, delay: i === 0 ? 0.05 : 0.18 }}
            >
              <div className="lead" style={{ color, marginBottom: '2cqw' }}>{col.label}</div>
              <ul style={{ listStyle: 'none', display: 'flex', flexDirection: 'column', gap: '1.2cqw' }}>
                {col.items.map(item => (
                  <li key={item} className="body" style={{ fontSize: 'var(--fs-lead)', display: 'flex', alignItems: 'center', gap: '1.2cqw' }}>
                    <span style={{ width: '2cqw', height: 2, background: color, flexShrink: 0 }} />
                    {item}
                  </li>
                ))}
              </ul>
            </motion.div>
          )
        })}
      </div>
    </div>
  )
}
