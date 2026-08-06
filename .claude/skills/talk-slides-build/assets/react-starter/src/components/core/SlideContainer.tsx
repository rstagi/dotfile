import { ReactNode } from 'react'
import { motion, AnimatePresence } from 'framer-motion'

interface SlideContainerProps {
  children: ReactNode
  slideKey: string
}

/* Default transition is a soft cross-fade with a faint drift — like turning a
   page. No hard slide-ins. A per-talk style spec can tweak easing/duration. */
export function SlideContainer({ children, slideKey }: SlideContainerProps) {
  return (
    <AnimatePresence mode="wait">
      <motion.div
        key={slideKey}
        className="stage-layer"
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -8 }}
        transition={{ duration: 0.38, ease: [0.22, 0.61, 0.36, 1] }}
      >
        {children}
      </motion.div>
    </AnimatePresence>
  )
}
