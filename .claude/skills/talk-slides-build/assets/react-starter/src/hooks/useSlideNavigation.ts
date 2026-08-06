import { useState, useCallback } from 'react'

export type Direction = 'forward' | 'backward'

interface UseSlideNavigationOptions {
  totalSlides: number
  getStepCount: (slideIndex: number) => number
}

/**
 * Two-axis navigation. currentIndex = which slide; subStep = which fragment
 * within it. subStep 0 is the slide's resting state; subStep N reveals
 * fragment N. next() walks the fragments, then advances the slide.
 */
export function useSlideNavigation({ totalSlides, getStepCount }: UseSlideNavigationOptions) {
  const [currentIndex, setCurrentIndex] = useState(0)
  const [subStep, setSubStep] = useState(0)
  const [direction, setDirection] = useState<Direction>('forward')

  const next = useCallback(() => {
    const steps = getStepCount(currentIndex)
    if (subStep < steps) {
      setSubStep(s => s + 1)
    } else if (currentIndex < totalSlides - 1) {
      setDirection('forward')
      setCurrentIndex(i => i + 1)
      setSubStep(0)
    }
  }, [currentIndex, subStep, totalSlides, getStepCount])

  const prev = useCallback(() => {
    if (subStep > 0) {
      setSubStep(s => s - 1)
    } else if (currentIndex > 0) {
      setDirection('backward')
      const prevIndex = currentIndex - 1
      setCurrentIndex(prevIndex)
      setSubStep(getStepCount(prevIndex)) // land on the prev slide fully revealed
    }
  }, [currentIndex, subStep, getStepCount])

  const goTo = useCallback((index: number) => {
    if (index >= 0 && index < totalSlides) {
      setDirection(index > currentIndex ? 'forward' : 'backward')
      setCurrentIndex(index)
      setSubStep(0)
    }
  }, [currentIndex, totalSlides])

  return {
    currentIndex,
    subStep,
    direction,
    next,
    prev,
    goTo,
    isFirst: currentIndex === 0 && subStep === 0,
    isLast: currentIndex === totalSlides - 1,
  }
}
