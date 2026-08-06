import { motion } from 'framer-motion'
import type { DataSlide } from '../../data/slides'
import { renderRich } from '../core/rich'
import { ExampleChart } from '../charts/ExampleChart'

/* The records. Clean editorial: assertion headline, one decluttered chart, a
   sourced caption. Add charts as components and switch on `content.chart` here.
   `subStep` is available to stage chart layers if a chart wants it. */
export function DataScene({ content }: { content: DataSlide['content']; subStep: number }) {
  return (
    <div className="scene scene--flow">
      <h2 className="flow-head" style={{ marginBottom: '3cqh', maxWidth: '40ch' }}>{renderRich(content.assertion)}</h2>

      <motion.div style={{ width: '54cqw' }} initial={{ opacity: 0, y: 14 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.5 }}>
        {/* add: content.chart === 'yourChart' && <YourChart subStep={subStep} /> */}
        {content.chart === 'example' && <ExampleChart />}
      </motion.div>

      <div style={{ display: 'flex', gap: '2cqw', marginTop: '2.4cqh', alignItems: 'baseline' }}>
        <span
          style={{
            fontFamily: 'var(--talk-font-mono)',
            fontSize: 'var(--fs-kicker)',
            letterSpacing: '0.1em',
            textTransform: 'uppercase',
            color: 'var(--talk-muted)',
          }}
        >
          Source · {content.source}
        </span>
      </div>
      {content.footnote && (
        <p className="num muted" style={{ fontSize: 'var(--fs-micro)', marginTop: '0.8cqh', fontStyle: 'italic' }}>
          {content.footnote}
        </p>
      )}
    </div>
  )
}
