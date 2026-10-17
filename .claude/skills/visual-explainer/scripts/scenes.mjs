// Fixed scene components. Each one turns validated props into escaped HTML.
// Rule: every props value passes through esc(), inline() or href() before it reaches the page.

const SEVERITIES = ["blocker", "major", "minor", "nit", "style"];
const FINDING_STATUS_ORDER = { open: 0, deferred: 1, fixed: 2 };
const IMPACT_ORDER = { high: 0, medium: 1, low: 2 };
const VERDICT_LABEL = { APPROVE: "Approved", COMMENT: "Comment", REQUEST_CHANGES: "Changes requested" };
const DELIVERED_LABEL = { met: "Met", partial: "Partial", missed: "Missed" };
const STATUS_ICON_PATH = { met: "M6 13l4 4 8-9", partial: "M6 12h12", missed: "M7 7l10 10M17 7L7 17" };
const MAX_STAGGER = 12;

// renderPage(template, props) -> the complete HTML page for validated props.
export function renderPage(template, props) {
  const view = props.mode === "plan" ? planView(props) : shipView(props);
  return fill(template, {
    MODE: esc(props.mode),
    TITLE: esc(props.meta.title),
    DESCRIPTION: esc(props.meta.subtitle),
    HERO: view.hero,
    NAV: nav(view.scenes),
    SCENES: view.scenes.map(scene).join("\n"),
    FOOTER: `visual-explainer · ${esc(props.mode)} · ${esc(props.meta.date)}`,
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

function planView(p) {
  return {
    hero: hero(p.meta, "Plan", planStats(p)),
    scenes: [
      { key: "goal", title: "Goal", body: `<p class="lead goal">${inline(p.goal)}</p>` },
      { key: "architecture", title: "Architecture", body: architectureScene(p.architecture) },
      { key: "timeline", title: "Timeline", body: p.milestones.map(milestoneBlock).join("") },
      { key: "decisions", title: "Decisions", body: decisionsScene(p.decisions) },
      { key: "risks", title: "Risks", body: risksScene(p.risks) },
    ],
  };
}

function shipView(p) {
  return {
    hero: hero(p.meta, "Ship report", shipStats(p)),
    scenes: [
      { key: "outcome", title: "Outcome", body: outcomeScene(p) },
      { key: "delivered", title: "Delivered", body: deliveredScene(p.delivered) },
      { key: "deviations", title: "Deviations", body: deviationsScene(p.deviations) },
      { key: "review", title: "Review", body: p.reviews.map(reviewBlock).join("") },
      { key: "hand-test", title: "Hand test", body: handTestScene(p.handTest) },
      { key: "rollback", title: "Rollback", body: rollbackScene(p.rollback) },
    ],
  };
}

// One pass over the template, with a function replacer: props text is never re-scanned
// for slots and "$&"-style replacement patterns stay literal.
function fill(template, slots) {
  return template.replace(/\{\{([A-Z]+)\}\}/g, (match, name) => slots[name] ?? match);
}

function nav(scenes) {
  const links = scenes.map((s) => `<a href="#s-${s.key}">${esc(s.title)}</a>`).join("");
  return `${links}<button type="button" class="play" data-play aria-pressed="false">Play</button>`;
}

function scene(s, i) {
  return `<section class="scene" id="s-${s.key}">
<div class="scene-head"><span class="kicker">${String(i + 1).padStart(2, "0")}</span><h2>${esc(s.title)}</h2></div>
${s.body}
</section>`;
}

function hero(meta, kicker, aside) {
  const chips = [meta.date, meta.planId, meta.milestone && `Milestone ${meta.milestone}`]
    .filter(Boolean).map((c) => `<span class="chip">${esc(c)}</span>`);
  const links = (meta.links ?? []).map((l) => `<a class="chip chip-link" href="${href(l.url)}">${esc(l.label)}</a>`);
  return `<div class="hero-text"><p class="eyebrow">${esc(kicker)}</p><h1>${esc(meta.title)}</h1>
<p class="subtitle">${inline(meta.subtitle)}</p><div class="chips">${[...chips, ...links].join("")}</div></div>
<div class="hero-aside">${aside}</div>`;
}

// ---- plan scenes ----

function planStats(p) {
  const phases = p.milestones.flatMap((m) => m.phases);
  const lanes = new Set(phases.map((ph) => ph.lane).filter(Boolean)).size;
  let k = 0;
  const strip = p.milestones.map((m, mi) => (mi ? `<i class="sep"></i>` : "") + m.phases.map((ph) =>
    `<i class="${ph.kind === "pr-review" ? "gate" : "work"}" style="--i:${k++}" title="${esc(`P${ph.number} ${ph.title}`)}"></i>`).join("")).join("");
  return `<div class="stat-col"><p class="stat-line"><b>${p.milestones.length}</b> ${plural(p.milestones.length, "milestone")}</p>
<p class="stat-line"><b>${phases.length}</b> ${plural(phases.length, "phase")}${lanes ? ` · <b>${lanes}</b> ${plural(lanes, "lane")}` : ""}</p>
<div class="strip" role="img" aria-label="${phases.length} phases in plan order">${strip}</div></div>`;
}

function plural(n, word) {
  return n === 1 ? word : `${word}s`;
}

function architectureScene(a) {
  const layers = a.layers.map((l, i) => `${i ? `<svg class="arch-link" viewBox="0 0 24 36" aria-hidden="true"><path d="M12 0V36"/></svg>` : ""}
<div class="layer" ${stagger(i)}><p class="label layer-name">${esc(l.name)}</p><div class="comps">${l.components.map((c) =>
    `<div class="comp${c.status ? ` comp-${esc(c.status)}` : ""}"><b>${esc(c.name)}</b>${c.status ? `<span class="badge">${esc(c.status)}</span>` : ""}<span class="role">${inline(c.role)}</span></div>`).join("")}</div></div>`).join("");
  const flows = (a.flows ?? []).map((f) => `<li><span class="node">${esc(f.from)}</span>
<svg viewBox="0 0 34 12" aria-hidden="true"><path class="flow" d="M2 6h27"/><path d="M25 1l5 5-5 5"/></svg>
<span class="node">${esc(f.to)}</span><span class="flow-label">${inline(f.label)}</span></li>`).join("");
  return `<p class="lead">${inline(a.summary)}</p><div class="arch">${layers}</div>${flows && `<h3>Flows</h3><ul class="flows">${flows}</ul>`}`;
}

function milestoneBlock(m) {
  return `<div class="milestone"><div class="ms-head"><span class="ms-num">M${esc(m.number)}</span><h3>${inline(m.title)}</h3>
${m.outcome ? `<p>${inline(m.outcome)}</p>` : ""}</div><ol class="timeline">${m.phases.map(phaseItem).join("")}</ol></div>`;
}

function phaseItem(ph, i) {
  const badges = [
    ph.kind === "pr-review" && `<span class="badge">review</span>`,
    ph.gate && `<span class="badge gate">merge gate</span>`,
    ph.lane && `<span class="badge">lane ${esc(ph.lane)}</span>`,
    ph.repository && `<span class="badge">${esc(ph.repository)}</span>`,
    ph.estimate && `<span class="badge">${esc(ph.estimate)}</span>`,
  ].filter(Boolean).join("");
  const deps = ph.dependsOn?.length ? `<p class="deps">Depends on ${ph.dependsOn.map((n) => `P${esc(n)}`).join(", ")}</p>` : "";
  return `<li class="phase kind-${esc(ph.kind)}" ${stagger(i)}><span class="dot"></span><div class="phase-card">
<div class="phase-top"><span class="num">P${esc(ph.number)}</span>${badges}</div><h4>${inline(ph.title)}</h4>
${ph.doneWhen ? `<p class="done-when"><b>Done when</b> ${inline(ph.doneWhen)}</p>` : ""}${deps}</div></li>`;
}

function decisionsScene(items) {
  return `<div class="grid">${items.map((d, i) => `<article class="card decision" ${stagger(i)}><h3>${inline(d.decision)}</h3>
<p>${inline(d.why)}</p><p class="label">Rejected</p><ul class="rejected">${d.rejected.map((r) =>
    `<li><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8"/></svg><span><b>${inline(r.option)}</b>: ${inline(r.reason)}</span></li>`).join("")}</ul></article>`).join("")}</div>`;
}

function risksScene(items) {
  const sorted = [...items].sort((a, b) => IMPACT_ORDER[a.impact] - IMPACT_ORDER[b.impact]);
  return `<div class="stack">${sorted.map((r, i) => `<article class="card risk impact-${esc(r.impact)}" ${stagger(i)}>
<div class="risk-top"><span class="pill pill-${esc(r.impact)}">impact ${esc(r.impact)}</span>${r.likelihood ? `<span class="pill pill-${esc(r.likelihood)}">likelihood ${esc(r.likelihood)}</span>` : ""}</div>
<p class="risk-title">${inline(r.risk)}</p><p><b>Mitigation</b> ${inline(r.mitigation)}</p></article>`).join("")}</div>`;
}

// ---- ship scenes ----

function shipStats(p) {
  const met = p.delivered.filter((d) => d.status === "met").length;
  const pct = Math.round((met / p.delivered.length) * 100);
  const unresolved = p.reviews.flatMap((r) => r.findings).filter((f) => f.status !== "fixed").length;
  const verdicts = p.reviews.map((r) => verdictBadge(r.verdict)).join("");
  return `${ring(pct, `${met}/${p.delivered.length}`, "Done when met")}
<div class="stat-col">${verdicts}<p class="stat-line"><b>${unresolved}</b> unresolved ${plural(unresolved, "finding")}</p></div>`;
}

function ring(pct, value, label) {
  return `<figure class="ring" style="--pct:${pct}" role="img" aria-label="${esc(`${value} ${label}`)}">
<svg viewBox="0 0 120 120" aria-hidden="true"><circle class="ring-track" cx="60" cy="60" r="52" pathLength="100"/>
<circle class="ring-value" cx="60" cy="60" r="52" pathLength="100"/></svg>
<figcaption><b>${esc(value)}</b><span>${esc(label)}</span></figcaption></figure>`;
}

function verdictBadge(verdict) {
  return `<span class="verdict verdict-${esc(verdict.toLowerCase().replace("_", "-"))}">${esc(VERDICT_LABEL[verdict])}</span>`;
}

function outcomeScene(p) {
  const changes = (p.changes ?? []).map((c, i) => `<article class="card change" ${stagger(i)}>
<h3>${esc(c.area)}</h3><p><b>How</b> ${inline(c.how)}</p><p><b>Why</b> ${inline(c.why)}</p></article>`).join("");
  return `<p class="lead">${inline(p.summary)}</p>${changes && `<div class="grid">${changes}</div>`}`;
}

function deliveredScene(items) {
  return `<ul class="checklist">${items.map((d, i) => `<li class="check check-${esc(d.status)}" ${stagger(i)}>
${statusIcon(d.status)}<div><p class="criterion">${d.phase !== undefined ? `<span class="tag">P${esc(d.phase)}</span>` : ""}${inline(d.criterion)}</p>
<p class="result"><span class="sr">${esc(DELIVERED_LABEL[d.status])}: </span>${inline(d.result)}</p></div></li>`).join("")}</ul>`;
}

function statusIcon(status) {
  return `<svg class="icon icon-${esc(status)}" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="11"/>
<path class="draw" pathLength="1" d="${STATUS_ICON_PATH[status]}"/></svg>`;
}

function deviationsScene(items) {
  if (!items.length) return `<p class="empty">No deviations from the plan.</p>`;
  return `<div class="stack">${items.map((d, i) => `<article class="card deviation" ${stagger(i)}>
<div class="dev-col"><p class="label">Planned</p><p>${inline(d.planned)}</p></div>
<svg class="arrow" viewBox="0 0 40 16" aria-hidden="true"><path class="draw" pathLength="1" d="M2 8h34M30 2l6 6-6 6"/></svg>
<div class="dev-col"><p class="label">Actual</p><p>${inline(d.actual)}</p></div>
<p class="reason"><b>Why</b> ${inline(d.reason)}</p></article>`).join("")}</div>`;
}

function reviewBlock(r) {
  const findings = [...r.findings].sort((a, b) => SEVERITIES.indexOf(a.severity) - SEVERITIES.indexOf(b.severity)
    || FINDING_STATUS_ORDER[a.status] - FINDING_STATUS_ORDER[b.status]);
  const links = [r.pr && `<a href="${href(r.pr)}">Pull request</a>`, r.url && `<a href="${href(r.url)}">Posted review</a>`]
    .filter(Boolean).join(" · ");
  const validation = r.validation.map((v) => `<li><span class="pill pill-${esc(v.result)}">${esc(v.result)}</span>
<code>${esc(v.command)}</code>${v.note ? `<span class="note">${inline(v.note)}</span>` : ""}</li>`).join("");
  return `<article class="review">
<div class="review-head">${verdictBadge(r.verdict)}${r.repository ? `<span class="chip">${esc(r.repository)}</span>` : ""}${r.head ? `<span class="chip mono">${esc(r.head.slice(0, 7))}</span>` : ""}</div>
<p class="lead">${inline(r.summary)}</p>${links && `<p class="links">${links}</p>`}
<h3>Validation</h3><ul class="validation">${validation}</ul>
<h3>Findings</h3>${severityBars(r.findings)}${findings.length ? `<ul class="findings">${findings.map(finding).join("")}</ul>` : `<p class="empty">No findings.</p>`}
${r.split ? `<h3>Stacked-PR split</h3><p>${inline(r.split)}</p>` : ""}</article>`;
}

function severityBars(findings) {
  const counts = SEVERITIES.map((s) => findings.filter((f) => f.severity === s));
  const max = Math.max(1, ...counts.map((c) => c.length));
  const rows = SEVERITIES.map((s, i) => {
    const unresolved = counts[i].filter((f) => f.status !== "fixed").length;
    return `<div class="bar-row" ${stagger(i)}><span class="bar-label">${s}</span>
<span class="bar"><span class="grow sev-${s}" style="--w:${Math.round((counts[i].length / max) * 100)}%"></span></span>
<span class="bar-count">${counts[i].length}${counts[i].length ? ` <small>(${unresolved} unresolved)</small>` : ""}</span></div>`;
  }).join("");
  return `<div class="bars" role="img" aria-label="Findings by severity">${rows}</div>`;
}

function finding(f) {
  return `<li class="finding status-${esc(f.status)} sev-border-${esc(f.severity)}"><div class="finding-top">
<span class="sev sev-${esc(f.severity)}">${esc(f.severity)}</span>${f.id ? `<span class="fid">${esc(f.id)}</span>` : ""}
<span class="pill pill-${esc(f.status)}">${esc(f.status)}</span>${f.location ? `<code>${esc(f.location)}</code>` : ""}</div>
<p class="finding-title">${inline(f.title)}</p>${f.detail ? `<p>${inline(f.detail)}</p>` : ""}${f.fix ? `<p class="fix"><b>Fix</b> ${inline(f.fix)}</p>` : ""}</li>`;
}

function handTestScene(steps) {
  return `<ol class="steps">${steps.map((s, i) => `<li class="step" ${stagger(i)}><span class="step-num">${i + 1}</span><div class="step-body">
${s.title ? `<h3>${esc(s.title)}</h3>` : ""}${command(s.command)}${codeRef(s.ref)}<p>${inline(s.why)}</p>
${s.expect ? `<p class="expect"><b>Expect</b> ${inline(s.expect)}</p>` : ""}</div></li>`).join("")}</ol>`;
}

function command(text) {
  return `<div class="cmd"><pre><code>${esc(text)}</code></pre><button type="button" class="copy" data-copy>Copy</button></div>`;
}

function codeRef(ref) {
  return `<p class="ref"><span class="label">Code</span><code>${esc(ref)}</code></p>`;
}

function rollbackScene(steps) {
  return `<ol class="steps rollback">${steps.map((s, i) => `<li class="step" ${stagger(i)}><span class="step-num">${i + 1}</span><div class="step-body">
<p>${inline(s.step)}</p>${s.command ? command(s.command) : ""}${s.ref ? codeRef(s.ref) : ""}</div></li>`).join("")}</ol>`;
}

// ---- shared helpers ----

// Escape first, then turn `code` spans into <code>. Escaped text has no "<", so the only
// markup in the result is the markup added here.
function inline(text) {
  return esc(text).replace(/`([^`\n]+)`/g, "<code>$1</code>").replace(/\n/g, "<br>");
}

function href(url) {
  return /^https?:\/\//i.test(url) ? esc(url) : "#";
}

// The animation delay index. A cap keeps long lists from waiting seconds for their last item.
function stagger(i) {
  return `style="--i:${Math.min(i, MAX_STAGGER)}"`;
}
