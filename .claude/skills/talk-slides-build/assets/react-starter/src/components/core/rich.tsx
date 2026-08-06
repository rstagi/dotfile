import { Fragment, type ReactNode } from 'react'

/* Minimal inline markup for statements:
   <a1>…</a1> → accent-1 (--talk-accent), <a2>…</a2> → accent-2 (--talk-accent-2),
   <em>…</em> → italic emphasis.
   The two accents are *meaning-mapped* per 04-slides-style.md — decide what each
   stands for there, then tag the words here. Don't add a third accent. */
export function renderRich(text: string): ReactNode {
  const tokens = text.split(/(<\/?(?:a1|a2|em)>)/g)
  const out: ReactNode[] = []
  const stack: string[] = []

  tokens.forEach((tok, i) => {
    const open = tok.match(/^<(a1|a2|em)>$/)
    const close = tok.match(/^<\/(a1|a2|em)>$/)
    if (open) {
      stack.push(open[1])
    } else if (close) {
      stack.pop()
    } else if (tok) {
      const tag = stack[stack.length - 1]
      if (tag === 'a1') out.push(<span key={i} className="accent-1">{tok}</span>)
      else if (tag === 'a2') out.push(<span key={i} className="accent-2">{tok}</span>)
      else if (tag === 'em') out.push(<em key={i}>{tok}</em>)
      else out.push(<Fragment key={i}>{tok}</Fragment>)
    }
  })
  return out
}
