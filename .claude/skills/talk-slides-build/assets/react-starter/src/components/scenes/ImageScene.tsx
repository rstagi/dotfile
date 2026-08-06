import { motion } from 'framer-motion'
import type { ImageSlide } from '../../data/slides'

/* Full-stage media. Put real photography / a hand-built figure here — never an
   AI-generated image. Drop the file in /public and reference it as "/your.jpg".
   Caption + kicker sit in the lower third over a legibility scrim. */
export function ImageScene({ content }: { content: ImageSlide['content'] }) {
  return (
    <div className="scene" style={{ padding: 0 }}>
      <motion.img
        src={content.src}
        alt={content.caption ?? ''}
        initial={{ opacity: 0, scale: 1.03 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.7 }}
        style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'cover' }}
      />
      {(content.caption || content.kicker) && (
        <>
          <div style={{ position: 'absolute', inset: 0, background: 'linear-gradient(0deg, rgba(0,0,0,0.72), transparent 45%)' }} />
          <div style={{ position: 'absolute', left: 'var(--margin)', right: 'var(--margin)', bottom: '8cqh' }}>
            {content.kicker && <div className="kicker" style={{ marginBottom: '0.8cqw' }}>{content.kicker}</div>}
            {content.caption && <div className="h2">{content.caption}</div>}
          </div>
        </>
      )}
    </div>
  )
}
