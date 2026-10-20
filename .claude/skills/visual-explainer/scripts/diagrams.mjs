// The two generated diagrams of the report page, as inline SVG strings: the before/after
// architecture (plan mode) and the sequence diagram of one change (ship mode). Every props value
// passes through esc(). Text width is an estimate (no font metrics in Node): labels are clipped
// to the estimate and the full text stays in a <title>.
import { esc } from "./page.mjs";

// Architecture boxes: one row per layer, at most ARCH_COLS boxes in a row (a long layer wraps).
const ARCH = { boxW: 150, boxH: 46, gapX: 22, gapY: 36, padX: 24, padY: 14, cols: 3, name: 12.5, role: 10.5 };
// minPhone: below this width the diagram scrolls in its box instead of shrinking further.
const SEQ = { boxH: 30, maxBox: 170, minPhone: 560, minStep: 120, maxStep: 230, top: 64, row: 32, selfRow: 42, pad: 16, font: 11.5 };
// Average glyph width as a share of the font size, for the system sans stack.
const CHAR_EM = 0.56;
const FLOW_STATUSES = ["new", "changed", "existing", "removed"];
const STATUS_LABEL = { new: "new", changed: "changed", existing: "unchanged", removed: "removed" };

// architectureDiagram(architecture) -> the HTML of the architecture figure: two panels (before,
// after) with a legend, or one panel when no component or flow has a status.
export function architectureDiagram(arch) {
  const components = arch.layers.flatMap((l, li) => l.components.map((c) => ({ ...c, layer: li, status: c.status ?? "existing" })));
  const flows = (arch.flows ?? []).map((f) => ({ ...f, status: f.status ?? "existing" }));
  const changed = components.some((c) => c.status !== "existing") || flows.some((f) => f.status !== "existing");
  const panels = changed
    ? [
      { id: "before", title: label("Before", arch.beforeTitle), comps: components.filter((c) => c.status !== "new").map((c) => ({ ...c, shown: c.status === "removed" ? "removed" : "existing" })), flows: flows.filter((f) => f.status !== "new") },
      { id: "after", title: label("After", arch.afterTitle), comps: components.filter((c) => c.status !== "removed").map((c) => ({ ...c, shown: c.status })), flows: flows.filter((f) => f.status !== "removed") },
    ]
    : [{ id: "now", title: "Architecture", comps: components.map((c) => ({ ...c, shown: "existing" })), flows }];
  const cols = Math.min(ARCH.cols, Math.max(...panels.flatMap((p) => rowsOf(p.comps).map((r) => r.length))));
  const width = ARCH.padX * 2 + cols * ARCH.boxW + (cols - 1) * ARCH.gapX;
  const svgs = panels.map((p) => `<figure class="arch-panel"><figcaption>${esc(p.title)}</figcaption>${archSvg(p, width)}</figure>`);
  const used = new Set(panels.flatMap((p) => p.comps.map((c) => c.shown)));
  const legend = ["new", "changed", "existing", "removed"].filter((s) => used.has(s))
    .map((s) => `<span class="lg"><i class="sw st-${s}"></i>${STATUS_LABEL[s]}</span>`).join("");
  return `<div class="arch" style="--panel-min: ${width + 26}px">${svgs.join("")}</div><p class="legend">${legend}</p>`;
}

// sequenceDiagram(sequence) -> the HTML of one sequence diagram (an inline SVG in a scroll box).
export function sequenceDiagram(seq) {
  const names = seq.participants.map((p) => p.name);
  const col = new Map(names.map((n, i) => [n, i]));
  const boxW = Math.round(Math.min(SEQ.maxBox, Math.max(84, ...names.map((n) => textW(n, 12) + 24))));
  const need = seq.messages.filter((m) => m.from !== m.to)
    .map((m) => (textW(m.label, SEQ.font) + 28) / Math.abs(col.get(m.from) - col.get(m.to)));
  const step = Math.round(Math.min(SEQ.maxStep, Math.max(SEQ.minStep, boxW + 24, ...need)));
  const selfTail = Math.max(0, ...seq.messages.filter((m) => m.from === m.to && col.get(m.from) === names.length - 1).map((m) => textW(m.label, SEQ.font) + 44 - boxW / 2));
  const x = (name) => SEQ.pad + boxW / 2 + col.get(name) * step;
  const width = Math.round(SEQ.pad * 2 + boxW + (names.length - 1) * step + selfTail);
  let y = SEQ.top;
  const rows = seq.messages.map((m) => {
    const self = m.from === m.to || m.kind === "self";
    const at = y;
    y += self ? SEQ.selfRow : SEQ.row;
    return self ? selfMessage(m, x(m.from), at, width) : message(m, x(m.from), x(m.to), at);
  });
  const height = y + 4;
  const heads = seq.participants.map((p) => {
    const cx = x(p.name);
    return `<g class="part st-${p.status ?? "existing"}"><rect x="${cx - boxW / 2}" y="6" width="${boxW}" height="${SEQ.boxH}" rx="5"/>`
      + `<text x="${cx}" y="${6 + SEQ.boxH / 2 + 4}" text-anchor="middle">${esc(clip(p.name, boxW - 12, 12))}</text>`
      + `<line class="life" x1="${cx}" y1="${6 + SEQ.boxH}" x2="${cx}" y2="${height - 2}"/></g>`;
  }).join("");
  const title = `Sequence diagram: ${names.join(", ")}`;
  return `<div class="seq-box"><svg class="seq" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" style="min-width: ${Math.min(Math.round(width * 0.8), SEQ.minPhone)}px" role="img" aria-label="${esc(title)}" xmlns="http://www.w3.org/2000/svg">`
    + `<defs>${marker("ve-seq-ah")}</defs>${heads}${rows.join("")}</svg></div>`;
}

// ---- architecture ----

function label(base, extra) {
  return extra ? `${base} — ${extra}` : base;
}

// The rows of one panel: one or more rows for each layer that has components in it.
function rowsOf(comps) {
  const rows = [];
  const layers = [...new Set(comps.map((c) => c.layer))].sort((a, b) => a - b);
  for (const layer of layers) {
    const inLayer = comps.filter((c) => c.layer === layer);
    for (let i = 0; i < inLayer.length; i += ARCH.cols) rows.push(inLayer.slice(i, i + ARCH.cols));
  }
  return rows;
}

function archSvg(panel, width) {
  const rows = rowsOf(panel.comps);
  const boxes = new Map();
  rows.forEach((row, ri) => {
    const rowW = row.length * ARCH.boxW + (row.length - 1) * ARCH.gapX;
    const left = (width - rowW) / 2;
    row.forEach((c, i) => boxes.set(c.name, { c, row: ri, x: left + i * (ARCH.boxW + ARCH.gapX), y: ARCH.padY + ri * (ARCH.boxH + ARCH.gapY), w: ARCH.boxW, h: ARCH.boxH }));
  });
  const height = ARCH.padY * 2 + rows.length * ARCH.boxH + Math.max(0, rows.length - 1) * ARCH.gapY;
  const all = [...boxes.values()];
  const arrows = panel.flows.filter((f) => boxes.has(f.from) && boxes.has(f.to) && f.from !== f.to)
    
    .map((f) => `<path class="flow fs-${f.status}" d="${route(boxes.get(f.from), boxes.get(f.to), all, width)}" marker-end="url(#ve-ah-${panel.id}-${f.status})"><title>${esc(`${f.from} → ${f.to}: ${f.label}`)}</title></path>`);
  const nodes = all.map(({ c, x, y, w, h }) => `<g class="comp st-${c.shown}"><title>${esc(`${c.name}: ${c.role}`)}</title><rect x="${x}" y="${y}" width="${w}" height="${h}" rx="6"/>`
    + `<text class="cn" x="${x + w / 2}" y="${y + 19}" text-anchor="middle">${esc(clip(c.name, w - 14, ARCH.name))}</text>`
    + `<text class="cr" x="${x + w / 2}" y="${y + 35}" text-anchor="middle">${esc(clip(plain(c.role), w - 14, ARCH.role))}</text></g>`);
  return `<svg class="arch-svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" style="min-width: ${Math.round(width * 0.85)}px" role="img" aria-label="${esc(panel.title)}" xmlns="http://www.w3.org/2000/svg">`
    + `<defs>${FLOW_STATUSES.map((st) => marker(`ve-ah-${panel.id}-${st}`, `fs-${st}`)).join("")}</defs>${arrows.join("")}${nodes.join("")}</svg>`;
}

// A straight arrow between the facing edges of two boxes. When the straight line would cross
// another box, the arrow runs in the empty bands between the rows and in the right margin.
function route(a, b, all, width) {
  const ax = a.x + a.w / 2;
  const bx = b.x + b.w / 2;
  if (a.row === b.row) {
    const dir = b.x > a.x ? 1 : -1;
    if (Math.abs(b.x - a.x) <= a.w + ARCH.gapX + 1) return `M${dir > 0 ? a.x + a.w : a.x} ${a.y + a.h / 2} L${dir > 0 ? b.x : b.x + b.w} ${b.y + b.h / 2}`;
    // Along the band above the row, or below it for the top row.
    const up = a.row > 0;
    const edge = up ? a.y : a.y + a.h;
    const band = up ? a.y - ARCH.gapY / 2 : a.y + a.h + ARCH.gapY / 2;
    return `M${ax} ${edge} V${band} H${bx} V${edge}`;
  }
  const down = b.row > a.row;
  const start = [ax, down ? a.y + a.h : a.y];
  const end = [bx, down ? b.y : b.y + b.h];
  if (!all.some((o) => o !== a && o !== b && crosses(start, end, o))) return `M${start[0]} ${start[1]} L${end[0]} ${end[1]}`;
  // Around the boxes: into the empty band past a, along the band to a free channel through the
  // rows between a and b, then along the band before b and into b.
  const out = down ? a.y + a.h + ARCH.gapY / 2 : a.y - ARCH.gapY / 2;
  const into = down ? b.y - ARCH.gapY / 2 : b.y + b.h + ARCH.gapY / 2;
  const lo = Math.min(a.row, b.row);
  const hi = Math.max(a.row, b.row);
  const x = channel(all.filter((o) => o.row > lo && o.row < hi), (ax + bx) / 2, width);
  return `M${start[0]} ${start[1]} V${out} H${x} V${into} H${end[0]} V${end[1]}`;
}

// The x nearest to `want` that no box in `blockers` covers, with a margin of 6 px.
function channel(blockers, want, width) {
  const free = (x) => blockers.every((o) => x < o.x - 6 || x > o.x + o.w + 6);
  const gaps = [width - 10, 10, ...blockers.flatMap((o) => [o.x - ARCH.gapX / 2, o.x + o.w + ARCH.gapX / 2])];
  return gaps.filter(free).sort((p, q) => Math.abs(p - want) - Math.abs(q - want))[0];
}

function crosses([x1, y1], [x2, y2], box) {
  for (let t = 0.05; t < 1; t += 0.05) {
    const px = x1 + (x2 - x1) * t;
    const py = y1 + (y2 - y1) * t;
    if (px > box.x - 4 && px < box.x + box.w + 4 && py > box.y - 4 && py < box.y + box.h + 4) return true;
  }
  return false;
}

// ---- sequence ----

function message(m, x1, x2, y) {
  const kind = m.kind === "return" ? "ret" : "call";
  const dir = x2 > x1 ? 1 : -1;
  const span = Math.abs(x2 - x1);
  return `<g class="msg ${kind}"><text x="${(x1 + x2) / 2}" y="${y - 6}" text-anchor="middle">${esc(clip(m.label, span - 12, SEQ.font))}</text>`
    + `<line x1="${x1}" y1="${y}" x2="${x2 - dir * 2}" y2="${y}" marker-end="url(#ve-seq-ah)"/><title>${esc(m.label)}</title></g>`;
}

function selfMessage(m, x, y, width) {
  const w = 26;
  const room = width - (x + w + 8) - 4;
  return `<g class="msg self"><path d="M${x} ${y - 6} H${x + w} V${y + 8} H${x + 2}" marker-end="url(#ve-seq-ah)"/>`
    + `<text x="${x + w + 8}" y="${y + 5}">${esc(clip(m.label, room, SEQ.font))}</text><title>${esc(m.label)}</title></g>`;
}

// ---- helpers ----

// An arrowhead. `cls` gives it the color of its line: an SVG marker cannot inherit the stroke.
function marker(id, cls = "") {
  return `<marker id="${id}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path class="ah${cls ? ` ${cls}` : ""}" d="M0 0L10 5L0 10z"/></marker>`;
}

function textW(text, size) {
  return String(text).length * size * CHAR_EM;
}

// Clip text to an estimated pixel width at a font size.
function clip(text, px, size) {
  const max = Math.max(4, Math.floor(px / (size * CHAR_EM)));
  const s = String(text);
  return s.length <= max ? s : `${s.slice(0, max - 1).trimEnd()}…`;
}

function plain(text) {
  return String(text).replaceAll("`", "");
}
