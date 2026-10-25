// The page: the concept graph as an SVG, an outline of every node with all of its facts, and the
// graph data for the page script. Rule: every props value passes through esc(), inline(), href()
// or json() before it reaches the page.
import { buildGraph, EDGE_LABELS, KINDS, MODE_KINDS } from "./graph.mjs";

const TONES = ["ok", "warn", "bad", "info", "muted"];
const NEAR_TITLE_CHARS = 60;
// The characters that json() writes as \u escapes: the line and paragraph separators are U+2028 and U+2029.
const JSON_UNSAFE = new RegExp(`[<>&'${String.fromCharCode(0x2028, 0x2029)}]`, "g");

// renderPage(template, props, { stops }) -> the complete HTML page for validated props. `stops`
// is the tour (narration.mjs tourStops), each { title, caption, nodes, overview }.
export function renderPage(template, props, { stops }) {
  const graph = buildGraph(props);
  const byId = new Map(graph.nodes.map((n) => [n.id, n]));
  const kinds = MODE_KINDS[graph.mode].filter((kind) => graph.nodes.some((n) => n.kind === kind));
  return fill(template, {
    MODE: esc(graph.mode),
    TITLE: esc(props.meta.title),
    DESCRIPTION: esc(props.meta.subtitle),
    EYEBROW: graph.mode === "plan" ? "Plan" : "Ship",
    KIND_STYLES: kindStyles(kinds),
    LEGEND: legend(graph, kinds),
    GRAPH: graphSvg(graph, byId),
    OUTLINE: outline(graph, props, kinds, byId),
    DATA: json(pageData(graph, kinds, stops)),
  });
}

export function esc(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

// One pass over the template, with a function replacer: props text is never re-scanned
// for slots and "$&"-style replacement patterns stay literal.
function fill(template, slots) {
  return template.replace(/\{\{([A-Z_]+)\}\}/g, (match, name) => slots[name] ?? match);
}

// The colors of the kinds on the page and of the tones: one rule for each, from KINDS of graph.mjs.
function kindStyles(kinds) {
  const rules = kinds.map((kind) => `.k-${kind}, [data-kind="${kind}"] { --kc: var(--${KINDS[kind].color}); }`);
  const tones = TONES.map((tone) => `.t-${tone}, [data-tone="${tone}"] { --tc: var(--${tone}); }`);
  return [...rules, ...tones].join("\n");
}

// ---- the graph ----

function graphSvg(graph, byId) {
  const root = graph.nodes.find((n) => n.group === null);
  const cx = root.x + root.w / 2;
  const cy = root.y + root.h / 2;
  const spokes = graph.groups.map((g) => {
    const px = Math.max(g.x, Math.min(cx, g.x + g.w));
    const py = Math.max(g.y, Math.min(cy, g.y + g.h));
    return `<path class="spoke" d="M${cx} ${cy} L${px} ${py}"/>`;
  }).join("");
  const frames = graph.groups.map((g) => `<g class="grp" data-group="${esc(g.id)}"><rect class="frame" x="${g.x}" y="${g.y}" width="${g.w}" height="${g.h}" rx="22"/>`
    + `<text class="glabel" x="${g.x + 26}" y="${g.y + 38}">${esc(g.label)}</text></g>`).join("\n");
  const edges = graph.edges.map((e) => {
    const inner = e.type === "contains" && byId.get(e.from).group === byId.get(e.to).group;
    const arrow = e.type === "contains" ? "" : ` marker-end="url(#ve-arrow)"`;
    return `<path class="e e-${e.type}${inner ? " e-inner" : ""}" data-from="${esc(e.from)}" data-to="${esc(e.to)}" d="${e.d}"${arrow}/>`;
  }).join("\n");
  const nodes = graph.nodes.map(nodeSvg).join("\n");
  const label = `Concept graph with ${graph.nodes.length} nodes. Tab moves between the nodes, Enter opens one. The outline lists the same nodes.`;
  return `<svg class="graph" id="graph" viewBox="0 0 ${graph.width} ${graph.height}" role="group" aria-label="${esc(label)}" tabindex="0" xmlns="http://www.w3.org/2000/svg">
<defs><marker id="ve-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path class="arrow" d="M0 0L10 5L0 10z"/></marker></defs>
<g class="spokes">${spokes}</g>
<g class="frames">${frames}</g>
<g class="edges">${edges}</g>
<g class="nodes">${nodes}</g>
</svg>`;
}

function nodeSvg(node) {
  const root = node.group === null;
  const classes = ["n", `k-${node.kind}`, node.tone && `t-${node.tone}`, root && "root"].filter(Boolean).join(" ");
  const pad = root ? 20 : 18;
  const kickY = root ? 34 : 26;
  const firstY = root ? 70 : 52;
  const step = root ? 28 : 20;
  const lines = node.lines.map((line, i) => `<text class="ttl" x="${pad}" y="${firstY + i * step}">${esc(line)}</text>`).join("");
  const subY = firstY + (node.lines.length - 1) * step + 26;
  const sub = (node.sub ?? []).map((line, i) => `<text class="sub" x="${pad}" y="${subY + i * 19}">${esc(line)}</text>`).join("");
  return `<g class="${classes}" data-id="${esc(node.id)}" transform="translate(${node.x} ${node.y})" tabindex="0" role="button" aria-label="${esc(`${node.kicker}: ${plain(node.title)}`)}">`
    + `<rect class="box" width="${node.w}" height="${node.h}" rx="${root ? 18 : 12}"/><rect class="stripe" width="6" height="${node.h}" rx="3"/>`
    + `<text class="kick" x="${pad}" y="${kickY}">${esc(node.kicker)}</text>${lines}${sub}</g>`;
}

function legend(graph, kinds) {
  const rows = kinds.map((kind) => `<label class="lk"><input type="checkbox" data-kind="${kind}" checked><i class="sw k-${kind}"></i><span>${esc(KINDS[kind].label)}</span><b>${graph.nodes.filter((n) => n.kind === kind).length}</b></label>`);
  return `<div class="legend" id="legend" role="group" aria-label="Show kinds"><p class="label">Show</p>${rows.join("")}</div>`;
}

// ---- the outline ----

function outline(graph, props, kinds, byId) {
  const meta = props.meta;
  const chips = [meta.date, meta.planId, meta.milestone && `Milestone ${meta.milestone}`].filter(Boolean).map((c) => `<span class="chip">${esc(c)}</span>`);
  const links = (meta.links ?? []).map((l) => `<a class="chip chip-link" href="${href(l.url)}">${esc(l.label)}</a>`);
  const sections = kinds.map((kind) => {
    const nodes = graph.nodes.filter((n) => n.kind === kind);
    return `<section class="o-kind" id="o-k-${kind}" data-kind="${kind}"><h2><i class="sw k-${kind}"></i>${esc(KINDS[kind].label)} <span class="count">${nodes.length}</span></h2>
<ol class="o-list">
${nodes.map((n) => outlineNode(graph, n, byId)).join("")}</ol></section>`;
  }).join("\n");
  return `<section class="outline" id="outline" aria-label="Outline"><div class="wrap">
<header class="o-head"><p class="eyebrow">${graph.mode === "plan" ? "Plan" : "Ship"}</p><h1>${esc(meta.title)}</h1>
<p class="subtitle">${inline(meta.subtitle)}</p><div class="chips">${[...chips, ...links].join("")}</div></header>
${sections}
<footer class="o-foot">visual-explainer · ${esc(graph.mode)} · ${esc(meta.date)}</footer>
</div></section>`;
}

function outlineNode(graph, node, byId) {
  const badges = node.badges.map((b) => `<span class="badge">${esc(b)}</span>`).join("");
  return `<li class="o-node" id="o-${esc(node.id)}" data-id="${esc(node.id)}" data-kind="${node.kind}"${node.tone ? ` data-tone="${node.tone}"` : ""}>`
    + `<a class="o-link" href="#node=${esc(node.id)}"><span class="o-kick">${esc(node.kicker)}</span><span class="o-title">${inline(node.title)}</span></a>`
    + `${badges && `<p class="badges">${badges}</p>`}<p class="o-sum">${inline(node.summary)}</p>`
    + `<details class="o-more"><summary>Facts and connections</summary><div class="facts-wrap">${facts(node.facts)}${near(graph, node, byId)}</div></details></li>\n`;
}

function facts(list) {
  return `<dl class="facts">${list.map((f) => `<div class="f"><dt>${esc(f.label)}</dt><dd>${factValue(f)}</dd></div>`).join("")}</dl>`;
}

function factValue(f) {
  if (f.list) return `<ul>${f.list.map((item) => `<li>${inline(item)}</li>`).join("")}</ul>`;
  if (f.command !== undefined) return `<div class="cmd"><pre><code>${esc(f.command)}</code></pre><button type="button" class="copy" data-copy>Copy</button></div>`;
  if (f.code !== undefined) return `<code>${esc(f.code)}</code>`;
  if (f.links) return f.links.map((l) => `<a href="${href(l.url)}">${esc(l.label)}</a>`).join(" · ");
  return inline(f.text);
}

// The connections of a node, by relation, each one a deep link to its node.
function near(graph, node, byId) {
  const groups = new Map();
  for (const e of graph.edges) {
    if (e.from !== node.id && e.to !== node.id) continue;
    const out = e.from === node.id;
    const label = EDGE_LABELS[e.type][out ? 0 : 1];
    const other = byId.get(out ? e.to : e.from);
    if (!groups.has(label)) groups.set(label, []);
    groups.get(label).push(nearLink(other, e.label));
  }
  if (!groups.size) return "";
  const rows = [...groups].map(([label, links]) => `<div class="f"><dt>${esc(label)}</dt><dd>${links.join("")}</dd></div>`).join("");
  return `<div class="near"><h3>Connections</h3><dl class="facts">${rows}</dl></div>`;
}

function nearLink(node, label) {
  const title = clip(plain(node.title), NEAR_TITLE_CHARS);
  const text = node.code && !title.startsWith(node.code) ? `${node.code} ${title}` : title;
  return `<a class="near-link k-${node.kind}" href="#node=${esc(node.id)}">${esc(text)}${label ? `<small>${esc(label)}</small>` : ""}</a>`;
}

// ---- data ----

function pageData(graph, kinds, stops) {
  return {
    mode: graph.mode,
    title: graph.title,
    width: graph.width,
    height: graph.height,
    kinds: kinds.map((kind) => ({ kind, label: KINDS[kind].label })),
    nodes: graph.nodes.map((n) => ({
      id: n.id, kind: n.kind, group: n.group, code: n.code, kicker: n.kicker, title: plain(n.title), summary: plain(n.summary),
      x: n.x, y: n.y, w: n.w, h: n.h,
    })),
    edges: graph.edges.map(({ from, to, type }) => ({ from, to, type })),
    groups: graph.groups.map(({ id, name, x, y, w, h }) => ({ id, name, x, y, w, h })),
    stops: stops.map(({ title, caption, nodes, overview }) => ({ title, text: caption, nodes, overview })),
  };
}

// JSON for a <script type="application/json"> block: no "<", so the text cannot close the element,
// and no raw "&", ">" or "'" either.
function json(value) {
  return JSON.stringify(value).replace(JSON_UNSAFE, (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, "0")}`);
}

// ---- text helpers ----

// Escape first, then turn `code` spans into <code>. Escaped text has no "<", so the only
// markup in the result is the markup added here.
export function inline(text) {
  return esc(text).replace(/`([^`\n]+)`/g, "<code>$1</code>").replace(/\n/g, "<br>");
}

function plain(text) {
  return String(text).replaceAll("`", "");
}

function clip(text, max) {
  return text.length <= max ? text : `${text.slice(0, max - 1).trimEnd()}…`;
}

export function href(url) {
  return /^https?:\/\//i.test(url) ? esc(url) : "#";
}
