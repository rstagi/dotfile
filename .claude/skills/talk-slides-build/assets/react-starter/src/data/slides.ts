import type { SlideNotes } from '../components/core/PresenterNotes'

/* ===========================================================================
   THE CONTENT FILE. This is where you spend your time.
   - Assertion titles go in VERBATIM from 02-slides-content.md.
   - `steps` = how many fragments the slide reveals (subStep count).
   - `notes` = the cue card, VERBATIM from 03-delivery-plan.md.
   The data MODEL (BaseSlide, the discriminated union, the example deck shape)
   is the reusable engine contract — keep it; replace the example slides[] below
   with your talk, and add scene types as your world needs them.
   =========================================================================== */

export type SceneType = 'title' | 'statement' | 'data' | 'quote' | 'twocol' | 'image' | 'code'

interface BaseSlide {
  id: string
  type: SceneType
  steps: number // fragment count (subSteps beyond the resting state)
  chrome?: boolean // show deck chrome (default true; set false for cold-open/close)
  notes: SlideNotes
}

export interface TitleSlide extends BaseSlide {
  type: 'title'
  content: { kicker: string; title: string; subtitle?: string; byline: string }
}

export interface StatementSlide extends BaseSlide {
  type: 'statement'
  content: {
    kicker?: string
    /** inline accent tags: <a1>…</a1>, <a2>…</a2>, <em>…</em> */
    main: string
    reveal?: string // appears on subStep 1
    anchor?: string // muted line, appears after the reveal
  }
}

export interface DataSlide extends BaseSlide {
  type: 'data'
  content: { assertion: string; chart: string; source: string; footnote?: string }
}

export interface QuoteSlide extends BaseSlide {
  type: 'quote'
  content: { quote: string; attribution: string; note?: string }
}

export interface TwoColSlide extends BaseSlide {
  type: 'twocol'
  content: {
    assertion: string
    left: { label: string; accent: 'accent-1' | 'accent-2'; items: string[] }
    right: { label: string; accent: 'accent-1' | 'accent-2'; items: string[] }
  }
}

export interface ImageSlide extends BaseSlide {
  type: 'image'
  content: { src: string; caption?: string; kicker?: string }
}

export interface CodeSlide extends BaseSlide {
  type: 'code'
  content: { assertion: string; code: string; highlight?: string[] }
}

export type Slide =
  | TitleSlide
  | StatementSlide
  | DataSlide
  | QuoteSlide
  | TwoColSlide
  | ImageSlide
  | CodeSlide

/* --------------------------------------------------------------------------
   EXAMPLE DECK — replace every entry with your talk. It exists to prove the
   engine runs and to show each scene type. Titles here are placeholders, NOT
   real assertion sentences; yours come verbatim from 02-slides-content.md.
   -------------------------------------------------------------------------- */
export const slides: Slide[] = [
  {
    id: 'title',
    type: 'title',
    steps: 0,
    chrome: false,
    content: {
      kicker: 'Your Event · Year',
      title: 'Your Talk Title Goes Here',
      subtitle: 'The one-line promise of the talk',
      byline: 'Your Name · Role, Org',
    },
    notes: { line: '"Your hand-written opening line — delivered to the lens before anything else."', cues: ['Cold open · hold eye contact'] },
  },
  {
    id: 'statement',
    type: 'statement',
    steps: 1,
    content: {
      kicker: 'The throughline',
      main: 'This is the spine of your talk — one big sentence, with <a1>one idea</a1> accented',
      reveal: '— and the part you reveal a beat later.',
    },
    notes: { cues: ['[PAUSE] before the reveal', 'Stress the accented word'] },
  },
  {
    id: 'data',
    type: 'data',
    steps: 0,
    content: {
      assertion: 'A complete-sentence claim that the chart proves.',
      chart: 'example',
      source: 'Where the numbers came from',
      footnote: 'Any honest caveat about the data.',
    },
    notes: { cues: ['Name the takeaway out loud', 'anchor: the key number'] },
  },
  {
    id: 'twocol',
    type: 'twocol',
    steps: 0,
    content: {
      assertion: 'When two things genuinely differ, split them.',
      left: { label: 'One side', accent: 'accent-2', items: ['point', 'point', 'point'] },
      right: { label: 'The other', accent: 'accent-1', items: ['point', 'point', 'point'] },
    },
    notes: { cues: ['[PAUSE] after — let the split land'] },
  },
  {
    id: 'quote',
    type: 'quote',
    steps: 0,
    content: {
      quote: 'A quotation that earns a turn in your argument.',
      attribution: 'Who said it · where · when',
      note: 'An honest footnote about the quote.',
    },
    notes: { cues: ['Let it sit'] },
  },
  {
    id: 'close',
    type: 'title',
    steps: 0,
    chrome: false,
    content: {
      kicker: 'Your Talk Title',
      title: 'The closing line — a callback.',
      subtitle: 'The one thing to do next.',
      byline: 'Your Name · Org · handle',
    },
    notes: { line: '"Your hand-written closing line."', cues: ['Hold — stop. End clean.'] },
  },
]
