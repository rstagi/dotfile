// A tiny ZERO-DEP markdown renderer — deliberately NOT react-markdown (the observatory keeps
// exactly three runtime deps: react, react-dom, @xyflow/react). It renders the small subset the
// plan prose + PR-review reports actually use: paragraphs, `- `/`* ` bullets, **bold**, `code`,
// and [text](url) links. Any block it doesn't recognize (headings, fences, tables, quotes) is
// emitted verbatim in a <pre className="log"> — same escape hatch as the HIL block in Drawer.

import type { ReactNode } from "react";

export type MdInline =
  | { t: "text"; v: string }
  | { t: "bold"; v: string }
  | { t: "code"; v: string }
  | { t: "link"; v: string; href: string };

export type MdBlock =
  | { t: "p"; spans: MdInline[] }
  | { t: "ul"; items: MdInline[][] }
  | { t: "raw"; text: string };

/** Ordered by scan priority; the earliest match in the remaining text wins. */
const INLINE: { t: MdInline["t"]; re: RegExp }[] = [
  { t: "code", re: /`([^`]+)`/ },
  { t: "link", re: /\[([^\]]+)\]\(([^)\s]+)\)/ },
  { t: "bold", re: /\*\*([^*]+?)\*\*/ },
];

const BULLET = /^\s*[-*]\s+/;
const RAW_LINE = /^\s*(#{1,6}\s|>|`{3}|~{3}|\||-{3,}\s*$|={3,}\s*$|\*{3,}\s*$)/;

export function Markdown({ text }: { text: string | null | undefined }) {
  if (!text || !text.trim()) return null;
  return <div className="md">{parseMarkdown(text).map(renderBlock)}</div>;
}

/** Pure block+inline parser — the unit-tested core (no React, never throws). */
export function parseMarkdown(md: string): MdBlock[] {
  const lines = (md ?? "").replace(/\r\n/g, "\n").split("\n");
  const blocks: MdBlock[] = [];
  let i = 0;
  while (i < lines.length) {
    if (!lines[i].trim()) {
      i++;
    } else if (BULLET.test(lines[i])) {
      const items: MdInline[][] = [];
      while (i < lines.length && BULLET.test(lines[i])) items.push(parseInline(lines[i++].replace(BULLET, "")));
      blocks.push({ t: "ul", items });
    } else if (RAW_LINE.test(lines[i])) {
      const buf: string[] = [];
      while (i < lines.length && lines[i].trim() && !BULLET.test(lines[i])) buf.push(lines[i++]);
      blocks.push({ t: "raw", text: buf.join("\n") });
    } else {
      const buf: string[] = [];
      while (i < lines.length && lines[i].trim() && !BULLET.test(lines[i]) && !RAW_LINE.test(lines[i]))
        buf.push(lines[i++]);
      blocks.push({ t: "p", spans: parseInline(buf.join(" ")) });
    }
  }
  return blocks;
}

/** Split one line of prose into text / bold / code / link spans (left-to-right, non-overlapping). */
export function parseInline(s: string): MdInline[] {
  const out: MdInline[] = [];
  let rest = s;
  while (rest) {
    const best = earliest(rest);
    if (!best) {
      out.push({ t: "text", v: rest });
      break;
    }
    if (best.idx > 0) out.push({ t: "text", v: rest.slice(0, best.idx) });
    out.push(best.span);
    rest = rest.slice(best.idx + best.len);
  }
  return out;
}

function earliest(s: string): { idx: number; len: number; span: MdInline } | null {
  let best: { idx: number; len: number; span: MdInline } | null = null;
  for (const { t, re } of INLINE) {
    const m = s.match(re);
    if (!m || m.index == null) continue;
    if (best && m.index >= best.idx) continue;
    const span: MdInline =
      t === "link" ? { t: "link", v: m[1], href: m[2] } : t === "code" ? { t: "code", v: m[1] } : { t: "bold", v: m[1] };
    best = { idx: m.index, len: m[0].length, span };
  }
  return best;
}

// --- rendering -----------------------------------------------------------------------

function renderBlock(b: MdBlock, i: number): ReactNode {
  if (b.t === "raw") return <pre key={i} className="log">{b.text}</pre>;
  if (b.t === "ul")
    return (
      <ul key={i} className="md__ul">
        {b.items.map((spans, j) => (
          <li key={j}>{spans.map(renderInline)}</li>
        ))}
      </ul>
    );
  return <p key={i} className="md__p">{b.spans.map(renderInline)}</p>;
}

function renderInline(span: MdInline, i: number): ReactNode {
  if (span.t === "bold") return <strong key={i}>{span.v}</strong>;
  if (span.t === "code") return <code key={i} className="md__code">{span.v}</code>;
  if (span.t === "link")
    return (
      <a key={i} href={span.href} target="_blank" rel="noreferrer">
        {span.v}
      </a>
    );
  return <span key={i}>{span.v}</span>;
}
