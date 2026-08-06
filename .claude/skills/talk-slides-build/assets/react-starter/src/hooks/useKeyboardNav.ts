import { useEffect } from 'react'

interface UseKeyboardNavOptions {
  onNext: () => void
  onPrev: () => void
  onToggleNotes: () => void
  onGoTo: (index: number) => void
}

export function useKeyboardNav({ onNext, onPrev, onToggleNotes, onGoTo }: UseKeyboardNavOptions) {
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      switch (e.key) {
        case 'ArrowRight':
        case 'ArrowDown':
        case ' ':
        case 'Enter':
        case 'PageDown':
          e.preventDefault()
          onNext()
          break
        case 'ArrowLeft':
        case 'ArrowUp':
        case 'PageUp':
          e.preventDefault()
          onPrev()
          break
        case 'n':
        case 'N':
        case 's':
        case 'S':
          e.preventDefault()
          onToggleNotes()
          break
        case 'Home':
          e.preventDefault()
          onGoTo(0)
          break
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [onNext, onPrev, onToggleNotes, onGoTo])
}
