// The report page (index.html): one document read top to bottom. Plan mode: TL;DR, Done when,
// phases, the before/after architecture, decisions, rabbit holes. Ship mode: verdict, TL;DR,
// Done when planned vs delivered, changes in reading order (with the one sequence diagram),
// findings, hand test, rollback. Both end with a collapsed "Full detail" and a link to graph.html.
// Rule: every props value passes through esc(), inline() or href() before it reaches the page.
// Each section root has data-shot="<id>": shoot.mjs writes one PNG for each, and the walkthrough
// focuses it.
import fs from "node:fs";
import path from "node:path";
import { architectureDiagram, sequenceDiagram } from "./diagrams.mjs";
import { esc, href, inline } from "./page.mjs";

export const SEVERITIES = ["blocker", "major", "minor", "nit", "style"];
const FINDING_STATUSES = ["open", "deferred", "fixed"];
const IMPACTS = ["high", "medium", "low"];
const SEVERITY_TONE = { blocker: "bad", major: "orange", minor: "warn", nit: "", style: "" };
const DELIVERED_TONE = { met: "ok", partial: "warn", missed: "bad", pending: "" };
// Not met first: the order of the Done when rows and of the status counts. "pending" is a
// checkpoint row of a phase that is not done yet.
const DELIVERED_ORDER = ["missed", "partial", "pending", "met"];
// A Done when table or a hand test with more rows than this one gets the short form.
const LONG_LIST = 8;
const RESULT_TONE = { pass: "ok", fail: "bad", skipped: "" };
const MIME = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", webp: "image/webp", gif: "image/gif" };
const CIRCLED = ["①", "②", "③"];
const GRAPH_PAGE = "graph.html";
const WALKTHROUGH_SCRIPT = path.join(import.meta.dirname, "..", "template", "walkthrough.js");
// The section that each narration scene focuses (its data-shot id). outro focuses no section.
const SCENE_TARGET = { intro: "header", outro: null };
const PROCESS_LABELS = {
  phases: "Phases", attempts: "Attempts", retries: "Retries", escalations: "Escalations", questions: "Questions", hil: "HIL pauses",
  mergeConflicts: "Merge conflicts", reviewRounds: "Review rounds", fixRounds: "Fix rounds", wallTime: "Wall time",
};

// renderReport(template, props, { scenes, dir }) -> the complete report page for validated props.
// `scenes` is the checked narration (narration.mjs checkNarration) or null: with scenes, the page
// has a walkthrough. `dir` is the directory of the props file: media paths resolve against it.
export function renderReport(template, props, { scenes, dir }) {
  const media = (props.media ?? []).map((m) => ({ ...m, src: dataUri(path.join(dir, m.path)) }));
  const sections = props.mode === "plan" ? planSections(props, media) : shipSections(props, media);
  const numbered = sections.filter(Boolean).map((s, i) => section(s, i + 1)).join("\n");
  const steps = scenes ? walkthroughSteps(scenes) : null;
  const body = [header(props), tldr(props), steps ? startBar(steps) : "", numbered, fullDetail(props)].filter(Boolean).join("\n");
  return fill(template, {
    MODE: esc(props.mode),
    TITLE: esc(props.meta.title),
    DESCRIPTION: esc(props.meta.subtitle),
    DATE: esc(props.meta.date),
    BODY: body,
    WALKTHROUGH: steps ? walkthrough(steps) : "",
  });
}

// verdict(props) -> { tone, text } of the ship header pill. All reviews count: the worst wins.
export function verdict(props) {
  const findings = props.reviews.flatMap((r) => r.findings);
  const blocking = findings.filter((f) => f.status === "open" && f.severity === "blocker").length;
  if (blocking || props.reviews.some((r) => r.verdict === "REQUEST_CHANGES")) {
    return { tone: "bad", text: blocking ? `Changes requested, ${blocking} blocking` : "Changes requested" };
  }
  const left = findings.filter((f) => f.status !== "fixed");
  const worst = SEVERITIES.find((s) => left.some((f) => f.severity === s));
  if (!worst) return { tone: "ok", text: "Ready to merge" };
  const n = left.filter((f) => f.severity === worst).length;
  return { tone: "ok", text: `Ready to merge, ${n} ${worst} finding${n === 1 ? "" : "s"}` };
}

// One pass over the template, with a function replacer: props text is never re-scanned.
function fill(template, slots) {
  return template.replace(/\{\{([A-Z_]+)\}\}/g, (match, name) => slots[name] ?? match);
}

function section({ id, title, body }, n) {
  return `<section class="sec" id="${id}" data-shot="${id}"><h2><span class="n">${n}</span>${esc(title)}</h2>\n${body}\n</section>`;
}

// "N more met criteria", or "N more criteria: 2 pending · 5 met" when the rest is not all met.
function moreLabel(rest, statusOf) {
  const noun = `criteri${rest.length === 1 ? "on" : "a"}`;
  const pending = rest.filter((r) => statusOf(r) === "pending").length;
  return pending ? `${rest.length} more ${noun}: ${pending} pending · ${rest.length - pending} met` : `${rest.length} more met ${noun}`;
}

// ---- header and TL;DR ----

function header(props) {
  const meta = props.meta;
  const chips = [meta.date, meta.milestone && `Milestone ${meta.milestone}`].filter(Boolean).map((c) => `<span class="chip">${esc(c)}</span>`);
  const cp = meta.checkpoint;
  chips.push(`<span class="chip mode">${props.mode === "plan" ? "mode: plan" : cp ? "mode: ship · checkpoint" : "mode: ship · PR review"}</span>`);
  const facts = (meta.facts ?? []).map((f) => `<span class="fact"><b>${esc(f.label)}</b>${inline(f.value)}</span>`);
  if (props.mode === "ship") {
    if (props.stats) {
      const s = props.stats;
      facts.push(`<span class="stat"><span class="add">+${s.additions}</span> <span class="del">−${s.deletions}</span> across ${s.files} file${s.files === 1 ? "" : "s"}</span>`);
    }
    if (cp) {
      if (cp.since) facts.push(`<span class="fact"><b>Since</b>${esc(cp.since)}</span>`);
      facts.push(checkpointPill(cp));
    } else {
      const v = verdict(props);
      facts.push(`<span class="pill ${v.tone}">${esc(v.text)}</span>`);
    }
  }
  return `<header data-shot="header"><p class="chips">${chips.join("")}</p><h1>${esc(meta.title)}</h1>`
    + `<p class="sub">${inline(meta.subtitle)}</p>${facts.length ? `<p class="facts">${facts.join("")}</p>` : ""}</header>`;
}

// A checkpoint is progress, not a verdict: a blue pill and a small bar of the phases done.
function checkpointPill(cp) {
  const pct = Math.round((100 * cp.phasesDone) / cp.phasesTotal);
  return `<span class="pill info">Checkpoint ${cp.number} · ${cp.phasesDone} of ${cp.phasesTotal} phases done</span>`
    + `<span class="progress" role="progressbar" aria-label="Phases done" aria-valuemin="0" aria-valuemax="${cp.phasesTotal}" aria-valuenow="${cp.phasesDone}"><i style="width: ${pct}%"></i></span>`;
}

function tldr(props) {
  const rows = props.mode === "plan" ? planTldr(props) : shipTldr(props);
  return `<section class="tldr" data-shot="tldr" aria-label="TL;DR"><p class="label">TL;DR</p>${rows.map((r) => `<p>${r}</p>`).join("")}</section>`;
}

function planTldr(props) {
  const t = props.tldr;
  const risks = t ? (t.risks ?? []) : [...props.risks].sort(byImpact).slice(0, 2).map((r) => r.risk);
  const rows = [`<strong>Goal:</strong> ${inline(t ? t.goal : props.goal)}`];
  if (t) rows.push(`<strong>What I need from you:</strong> ${inline(t.ask)}`);
  if (risks.length) rows.push(`<strong>Top risks:</strong> ${risks.map((r, i) => `${CIRCLED[i]} ${inline(r.replace(/[.;]\s*$/, ""))}`).join("; ")}`);
  return rows;
}

function shipTldr(props) {
  const t = props.tldr;
  const rows = [`<strong>What changed:</strong> ${inline(t ? t.changed : props.summary)}`];
  const tail = [t?.lookFirst && `<strong>Look at first:</strong> ${inline(t.lookFirst)}`, t?.risk && `<strong>Risk:</strong> ${inline(t.risk)}`].filter(Boolean);
  if (tail.length) rows.push(tail.join(" "));
  return rows;
}

// ---- walkthrough ----

// One step for each scene: the data-shot id of the section to focus, or null, and the commentary.
function walkthroughSteps(scenes) {
  return scenes.map(({ id, text }) => ({ scene: id, target: id in SCENE_TARGET ? SCENE_TARGET[id] : id, text }));
}

function startBar(steps) {
  return `<p class="wt-bar"><button type="button" class="wt-start" data-walkthrough-start>Start the walkthrough</button>`
    + `<span class="wt-hint">${steps.length} steps · one section at a time, with commentary</span></p>`;
}

// The steps as JSON data, then the script that runs them. In the JSON, "<" is \u003c, so no
// text can close the script element.
function walkthrough(steps) {
  const json = JSON.stringify(steps).replace(/</g, "\\u003c");
  return `<script type="application/json" id="ve-walkthrough">${json}</script>\n<script>\n${fs.readFileSync(WALKTHROUGH_SCRIPT, "utf8")}</script>`;
}

// ---- plan ----

function planSections(props, media) {
  return [
    planDoneWhen(props),
    { id: "phases", title: "Phases", body: phases(props) },
    { id: "architecture", title: "Architecture: before / after", body: architectureDiagram(props.architecture) },
    mediaSection(media.filter((m) => !m.change)),
    { id: "decisions", title: "Decisions", body: `<div class="decisions">${props.decisions.map(decisionCard).join("")}</div>` },
    props.rabbitHoles?.length ? { id: "rabbit-holes", title: "Rabbit holes & no-gos", body: `<ul class="plain">${props.rabbitHoles.map((r) => `<li>${noGo(r)}</li>`).join("")}</ul>` } : null,
  ];
}

function planDoneWhen(props) {
  const rows = props.doneWhen
    ?? props.milestones[0].phases.filter((p) => p.doneWhen).map((p) => ({ criterion: `${p.title}: ${p.doneWhen}`, check: "—" }));
  if (!rows.length) return null;
  return { id: "done-when", title: "Done when", body: shortTable(["Criterion", "How we’ll check"], rows, (r) => [inline(r.criterion), inline(r.check)]) };
}

function phases(props) {
  const many = props.milestones.length > 1;
  return props.milestones.map((m) => {
    const items = m.phases.map((p) => {
      const line = p.summary ?? p.doneWhen;
      const review = p.kind === "pr-review" ? `<span class="tag info">${p.gate ? "review · gate" : "review"}</span>` : "";
      return `<li class="ph"><span class="box" aria-hidden="true"></span><span class="what"><strong>${inline(p.title)}</strong>${review}${line ? ` — ${inline(line)}` : ""}</span>`
        + `${p.estimate ? `<span class="est">${esc(p.estimate)}</span>` : ""}</li>`;
    }).join("");
    const head = many ? `<h3 class="mini">Milestone ${m.number} — ${inline(m.title)}</h3>` : "";
    return `${head}<ul class="phases">${items}</ul>`;
  }).join("");
}

function decisionCard(d) {
  const title = d.title ? `<h3>${inline(d.title)}</h3>` : "";
  const rejected = d.rejected.map((r) => `<li><s>${inline(r.option)}</s></li>`).join("");
  return `<div class="dc">${title}<p class="k">DECISION</p><p>${inline(d.decision)}</p>`
    + `<p class="k">REJECTED</p><ul>${rejected}</ul><p class="k">WHY</p><p>${inline(d.why)}</p></div>`;
}

function noGo(text) {
  const html = inline(text);
  return html.startsWith("No ") ? `<strong>No</strong>${html.slice(2)}` : html;
}

// ---- ship ----

function shipSections(props, media) {
  const changes = props.changes ?? [];
  return [
    {
      id: "done-when", title: "Done when: planned vs delivered",
      body: shortTable(["Planned criterion", "Delivered", "Status"], props.delivered,
        (d) => [inline(d.criterion), inline(d.result), { html: tag(d.status, DELIVERED_TONE[d.status]), cls: "st" }], (d) => d.status),
    },
    changes.length ? { id: "changes", title: "Changes, in reading order", body: `<div class="changes">${changes.map((c, i) => changeCard(c, i, media)).join("")}</div>` } : null,
    mediaSection(media.filter((m) => !m.change)),
    { id: "findings", title: "Findings", body: findings(props) },
    { id: "hand-test", title: "Hand test", body: handTest(props.handTest) },
    { id: "rollback", title: "Rollback", body: rollback(props.rollback) },
  ];
}

function changeCard(c, i, media) {
  const files = (c.files ?? []).map((f) => `<div class="file"><span class="p">${esc(f.path)}${f.note ? ` <span class="note">(${esc(f.note)})</span>` : ""}</span>`
    + `<span class="d"><span class="add">+${f.additions}</span> <span class="del">−${f.deletions}</span></span></div>`).join("");
  const figs = media.filter((m) => m.change === c.area);
  const seq = c.sequence ? `<div data-shot="sequence">${sequenceDiagram(c.sequence)}</div>` : "";
  return `<div class="chg" data-shot="change-${i + 1}"><h3><span class="badge">${i + 1}</span>${esc(c.area)}</h3>`
    + `<p class="how">${inline(c.how)}</p><p class="why">Why: ${inline(c.why)}</p>${seq}`
    + `${figs.length ? figures(figs) : ""}${files ? `<div class="files">${files}</div>` : ""}</div>`;
}

function findings(props) {
  const list = props.reviews.flatMap((r) => r.findings)
    .map((f, i) => ({ f, i }))
    .sort((a, b) => SEVERITIES.indexOf(a.f.severity) - SEVERITIES.indexOf(b.f.severity)
      || FINDING_STATUSES.indexOf(a.f.status) - FINDING_STATUSES.indexOf(b.f.status) || a.i - b.i)
    .map(({ f }) => f);
  if (!props.reviews.length) return `<p class="empty">No review yet. The milestone review runs at the gate.</p>`;
  if (!list.length) return `<p class="empty">No findings.</p>`;
  return `<div class="findings">${list.map((f) => {
    const status = f.status === "open" ? "" : tag(f.status, f.status === "fixed" ? "ok" : "");
    return `<div class="fd" data-severity="${f.severity}" data-status="${f.status}"><div class="fd-top">${tag(f.severity, SEVERITY_TONE[f.severity])}`
      + `<span class="t">${f.id ? `<span class="id">${esc(f.id)}</span>` : ""}${inline(f.title)}</span>${status}${f.location ? `<code class="loc">${esc(f.location)}</code>` : ""}</div>`
      + `${f.detail ? `<p>${inline(f.detail)}</p>` : ""}${f.fix ? `<p class="fix">Fix: ${inline(f.fix)}</p>` : ""}</div>`;
  }).join("")}</div>`;
}

// More than LONG_LIST steps: the compact form, a title and a command for each step, and the why,
// the expectation and the reference in one small muted line.
function handTest(steps) {
  if (steps.length > LONG_LIST) {
    const items = steps.map((s) => {
      const head = s.title ? esc(s.title) : inline(s.why);
      const rest = [s.title && inline(s.why), s.expect && `Expect: ${inline(s.expect)}`, `<span class="mono">${esc(s.ref)}</span>`].filter(Boolean);
      return `<li><p><strong>${head}</strong></p>${codeBlock(s.command)}<p class="aside">${rest.join(" · ")}</p></li>`;
    });
    return `<ol class="steps compact">${items.join("")}</ol>`;
  }
  const items = steps.map((s) => `<li><p>${s.title ? `<strong>${esc(s.title)}.</strong> ` : ""}${inline(s.why)}</p>${codeBlock(s.command)}`
    + `${s.expect ? `<p class="expect">Expect: ${inline(s.expect)}</p>` : ""}<p class="ref">${esc(s.ref)}</p></li>`);
  return `<ol class="steps">${items.join("")}</ol>`;
}

function rollback(steps) {
  const item = (s) => `<p>${inline(s.step)}</p>${s.command ? codeBlock(s.command) : ""}${s.ref ? `<p class="ref">${esc(s.ref)}</p>` : ""}`;
  const body = steps.length === 1 ? item(steps[0]) : `<ol>${steps.map((s) => `<li>${item(s)}</li>`).join("")}</ol>`;
  return `<div class="callout">${body}</div>`;
}

// ---- media ----

function mediaSection(list) {
  return list.length ? { id: "media", title: "Screenshots", body: figures(list) } : null;
}

function figures(list) {
  return `<div class="figs">${list.map((m) => `<figure class="fig"><img src="${m.src}" alt="${esc(m.caption)}"><figcaption>${inline(m.caption)}</figcaption></figure>`).join("")}</div>`;
}

// The file as a data URI. The extension is one of MIME (the schema pattern), so the type is fixed.
function dataUri(file) {
  const ext = path.extname(file).slice(1).toLowerCase();
  return `data:${MIME[ext]};base64,${fs.readFileSync(file).toString("base64")}`;
}

// ---- full detail ----

function fullDetail(props) {
  const parts = props.mode === "plan" ? planDetail(props) : shipDetail(props);
  parts.push(`<h3>Run</h3>${kv(metaRows(props.meta))}`);
  return `<details class="more"><summary>Full detail</summary><div class="more-body">${parts.join("\n")}`
    + `<a class="graph-link" href="${GRAPH_PAGE}">Explore the concept graph →</a></div></details>`;
}

function planDetail(props) {
  const milestones = props.milestones.map((m) => {
    const rows = m.phases.map((p) => [String(p.number), inline(p.title), esc(p.kind + (p.gate ? " · gate" : "")), esc(p.lane ?? ""), esc(p.repository ?? ""),
      esc((p.dependsOn ?? []).map((n) => `P${n}`).join(", ")), esc(p.estimate ?? ""), inline(p.doneWhen ?? "")]);
    return `<h4>Milestone ${m.number} — ${inline(m.title)}</h4>${m.outcome ? `<p>${inline(m.outcome)}</p>` : ""}`
      + table(["#", "Phase", "Kind", "Lane", "Repository", "Depends on", "Estimate", "Done when"], rows, "wide");
  });
  const risks = [...props.risks].sort(byImpact).map((r) => [inline(r.risk), esc(r.impact), esc(r.likelihood ?? ""), inline(r.mitigation), esc((r.mitigatedBy ?? []).map((n) => `P${n}`).join(", "))]);
  const arch = props.architecture;
  const comps = arch.layers.flatMap((l) => l.components.map((c) => [esc(l.name), esc(c.name), esc(c.status ?? "existing"), inline(c.role)]));
  const flows = (arch.flows ?? []).map((f) => `<li>${esc(f.from)} → ${esc(f.to)}: ${inline(f.label)}${f.status && f.status !== "existing" ? ` <span class="tag">${esc(f.status)}</span>` : ""}</li>`);
  return [
    `<h3>Milestones and phases</h3>${milestones.join("")}`,
    `<h3>Risks</h3>${table(["Risk", "Impact", "Likelihood", "Mitigation", "Mitigated by"], risks, "wide")}`,
    `<h3>Architecture</h3><p>${inline(arch.summary)}</p>${table(["Layer", "Component", "Status", "Role"], comps, "wide")}`
      + `${flows.length ? `<h4>Flows</h4><ul class="plain">${flows.join("")}</ul>` : ""}`,
  ];
}

function shipDetail(props) {
  const parts = props.reviews.map((r, i) => {
    const head = [r.repository && ["Repository", esc(r.repository)], ["Verdict", esc(r.verdict)], r.pr && ["PR", link(r.pr, r.pr)], r.url && ["Review", link(r.url, "review")],
      r.head && ["Head", `<code>${esc(r.head)}</code>`]].filter(Boolean);
    const checks = table(["Command", "Result", "Note"], r.validation.map((v) => [`<code>${esc(v.command)}</code>`, { html: tag(v.result, RESULT_TONE[v.result]), cls: "st" }, inline(v.note ?? "")]));
    return `<h3>Review${props.reviews.length > 1 ? ` ${i + 1}` : ""}</h3>${kv(head)}<p style="margin-top:8px">${inline(r.summary)}</p><h4>Validation</h4>${checks}`
      + `${r.split ? `<h4>Split</h4><p>${inline(r.split)}</p>` : ""}`;
  });
  parts.push(`<h3>Deviations</h3>${props.deviations.length
    ? table(["Planned", "Actual", "Reason"], props.deviations.map((d) => [inline(d.planned), inline(d.actual), inline(d.reason)]))
    : `<p class="empty">None: the result matches the plan.</p>`}`);
  if (props.process) {
    const p = props.process;
    const rows = Object.entries(p).map(([k, v]) => [esc(PROCESS_LABELS[k] ?? k), esc(v)]);
    parts.push(`<h3>Process</h3>${kv(rows)}`);
  }
  if (props.setbacks?.length) {
    parts.push(`<h3>Setbacks</h3>${table(["Kind", "Setback", "Cost", "Resolution", "Lesson"], props.setbacks.map((s) => [
      esc(s.kind), `<strong>${esc(s.title)}</strong>${s.phase !== undefined ? ` <span class="muted">P${s.phase}</span>` : ""}<br>${inline(s.what)}`,
      esc(s.cost ?? ""), inline(s.resolution), inline(s.lesson ?? "")]), "wide")}`);
  }
  const phased = (props.changes ?? []).filter((c) => c.phases?.length);
  if (phased.length) parts.push(`<h3>Phases of each change</h3>${kv(phased.map((c) => [esc(c.area), esc(c.phases.map((n) => `P${n}`).join(", "))]))}`);
  return parts;
}

function metaRows(meta) {
  return [["Date", esc(meta.date)], meta.planId && ["Plan", `<code>${esc(meta.planId)}</code>`], meta.milestone && ["Milestone", esc(meta.milestone)],
    meta.links?.length && ["Links", meta.links.map((l) => link(l.url, l.label)).join(" · ")]].filter(Boolean);
}

// ---- helpers ----

// shortTable(headers, rows, cells, statusOf) -> a Done when table of at most LONG_LIST rows; the
// other rows sit in a collapsed <details>. With `statusOf`, the rows show in DELIVERED_ORDER: every
// missed and partial row stays visible, then at most LONG_LIST pending rows, then met rows up to
// LONG_LIST rows in total. A chip line above the table counts each status.
function shortTable(headers, rows, cells, statusOf) {
  if (rows.length <= LONG_LIST) return table(headers, rows.map(cells), "stack");
  let shown = rows.slice(0, LONG_LIST);
  let rest = rows.slice(LONG_LIST);
  let counts = "";
  const of = (st) => rows.filter((r) => statusOf(r) === st);
  if (statusOf) {
    const failed = [...of("missed"), ...of("partial")];
    const pending = of("pending");
    const visible = [...failed, ...pending.slice(0, LONG_LIST)];
    const fill = Math.max(0, LONG_LIST - visible.length);
    shown = [...visible, ...of("met").slice(0, fill)];
    rest = [...pending.slice(LONG_LIST), ...of("met").slice(fill)];
    counts = `<p class="counts">${DELIVERED_ORDER.filter((st) => of(st).length).map((st) => tag(`${of(st).length} ${st}`, DELIVERED_TONE[st])).join(" ")}</p>`;
  }
  const label = statusOf ? moreLabel(rest, statusOf) : `${rest.length} more criteri${rest.length === 1 ? "on" : "a"}`;
  const more = rest.length ? `<details class="more-rows"><summary>${label}</summary>${table(headers, rest.map(cells), "stack")}</details>` : "";
  return `${counts}${table(headers, shown.map(cells), "stack")}${more}`;
}

// table(headers, rows, cls) -> a table in a box that scrolls. A cell is HTML, or { html, cls }.
// A "stack" table shows each row as a block of labelled cells on a phone.
function table(headers, rows, cls = "") {
  const label = (i) => (cls === "stack" ? ` data-label="${esc(headers[i])}"` : "");
  const cell = (c, i) => (typeof c === "string" ? `<td${label(i)}>${c}</td>` : `<td class="${c.cls}"${label(i)}>${c.html}</td>`);
  return `<div class="tbl${cls ? ` ${cls}` : ""}"><table><thead><tr>${headers.map((h) => `<th>${esc(h)}</th>`).join("")}</tr></thead>`
    + `<tbody>${rows.map((r) => `<tr>${r.map(cell).join("")}</tr>`).join("")}</tbody></table></div>`;
}

function kv(rows) {
  return `<dl class="kv">${rows.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join("")}</dl>`;
}

function tag(text, tone) {
  return `<span class="tag${tone ? ` ${tone}` : ""}">${esc(text)}</span>`;
}

function codeBlock(command) {
  return `<pre><code>${esc(command)}</code></pre>`;
}

function link(url, label) {
  return `<a href="${href(url)}">${esc(label)}</a>`;
}

function byImpact(a, b) {
  return IMPACTS.indexOf(a.impact) - IMPACTS.indexOf(b.impact);
}
