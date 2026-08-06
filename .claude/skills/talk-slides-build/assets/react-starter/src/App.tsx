import { useState, useCallback } from 'react'
import { slides, type Slide } from './data/slides'
import { sectionForSlide } from './data/sections'
import { useSlideNavigation } from './hooks/useSlideNavigation'
import { useKeyboardNav } from './hooks/useKeyboardNav'
import { SlideContainer } from './components/core/SlideContainer'
import { DeckChrome } from './components/core/DeckChrome'
import { PresenterNotes } from './components/core/PresenterNotes'
import {
  TitleScene,
  StatementScene,
  DataScene,
  QuoteScene,
  TwoColScene,
  ImageScene,
  CodeScene,
} from './components/scenes'

// counter label shown in the chrome + presenter notes — rename per the world.
const SLIDE_LABEL = 'Slide'

function getStepCount(index: number): number {
  return slides[index].steps
}

function renderScene(slide: Slide, subStep: number) {
  switch (slide.type) {
    case 'title':
      return <TitleScene content={slide.content} />
    case 'statement':
      return <StatementScene content={slide.content} subStep={subStep} />
    case 'data':
      return <DataScene content={slide.content} subStep={subStep} />
    case 'quote':
      return <QuoteScene content={slide.content} />
    case 'twocol':
      return <TwoColScene content={slide.content} />
    case 'image':
      return <ImageScene content={slide.content} />
    case 'code':
      return <CodeScene content={slide.content} subStep={subStep} />
  }
}

function titleOf(slide: Slide): string {
  switch (slide.type) {
    case 'title':
      return slide.content.title
    case 'quote':
      return slide.content.quote
    case 'image':
      return slide.content.caption ?? 'Image'
    default:
      return (slide.content as { assertion: string }).assertion
  }
}

export default function App() {
  const [notesOpen, setNotesOpen] = useState(false)
  const { currentIndex, subStep, next, prev, goTo } = useSlideNavigation({
    totalSlides: slides.length,
    getStepCount,
  })
  const toggleNotes = useCallback(() => setNotesOpen(o => !o), [])
  useKeyboardNav({ onNext: next, onPrev: prev, onToggleNotes: toggleNotes, onGoTo: goTo })

  const slide = slides[currentIndex]
  const section = sectionForSlide(currentIndex + 1)
  const showChrome = slide.chrome !== false

  return (
    <div className="deck">
      <SlideContainer slideKey={slide.id}>{renderScene(slide, subStep)}</SlideContainer>

      <DeckChrome
        section={section}
        index={currentIndex}
        total={slides.length}
        visible={showChrome}
        label={SLIDE_LABEL}
        /* badge={<YourWorldBadge … />}  ← the per-talk world can fill the top-right slot */
      />

      <PresenterNotes
        notes={slide.notes}
        title={titleOf(slide)}
        index={currentIndex}
        total={slides.length}
        visible={notesOpen}
        label={SLIDE_LABEL}
      />
    </div>
  )
}
