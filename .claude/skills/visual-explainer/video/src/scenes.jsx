// The body of each scene. Each view gets the scene data of storyboard.mjs (sceneData plus the
// beat frames of `timed`) and draws the section of the report page that it narrates.
import { AbsoluteFill, Easing, Img, interpolate, Sequence, staticFile, useCurrentFrame } from "remotion";
import { StatusCycle } from "@/components/snap-cn/status-cycle";
import { TerminalSimulator } from "@/components/snap-cn/terminal-simulator";
import { TextHighlight } from "@/components/snap-cn/text-highlight";
import { TextReveal } from "@/components/snap-cn/text-reveal";
import { Chip, Clamp, fitFont, Inline, More, useProgress, useRise } from "./parts.jsx";
import { CONTENT, TERMINAL } from "./storyboard.mjs";
import { COLORS as C, FONTS } from "./theme.mjs";

const BOX = { position: "absolute", left: CONTENT.x, top: CONTENT.y, width: CONTENT.w, height: CONTENT.h };
const STAGGER = 6;
const SHOT_PAN = 1.8;
// A stable empty list: status-cycle memoizes on the identity of its chips.
const NO_CHIPS = [];
// The hand-test terminal: 24 px text in a 1040 x 600 box. The snapcn terminal sizes its text
// from fontSize / 18 x 44 px, and centres its focus point in `viewport`: these values put the
// accent bar 20 px and the first row 24 px from the top left corner of the box.
const TERM_FONT = 24;
const TERM_VIEWPORT = { width: 314, height: 282 };

export const SCENE_VIEWS = {
  intro: Intro,
  tldr: Tldr,
  "done-when": DoneWhen,
  phases: Phases,
  architecture: Architecture,
  decisions: Decisions,
  "rabbit-holes": RabbitHoles,
  changes: Changes,
  findings: Findings,
  "hand-test": HandTest,
  rollback: Rollback,
  outro: Outro,
};

// ---- both modes ----

function Intro({ data }) {
  const titleSize = Math.min(96, Math.floor(1640 / (data.title.length * 0.55)));
  return (
    <AbsoluteFill>
      <div style={{ position: "absolute", top: 250, left: 0, right: 0, display: "flex", justifyContent: "center", ...useRise(0) }}>
        <Chip tone="open" size={24}>{data.chip}</Chip>
      </div>
      <div style={{ position: "absolute", top: 310, left: 80, right: 80, height: 170 }}>
        <TextReveal text={data.title} fontSize={titleSize} fontWeight={700} />
      </div>
      <div style={{ position: "absolute", top: 500, left: 260, right: 260, textAlign: "center", fontSize: 32, lineHeight: 1.4, color: C["ink-2"], ...useRise(34) }}>
        <Clamp lines={2}><Inline text={data.subtitle} /></Clamp>
      </div>
      <div style={{ position: "absolute", top: 630, left: 120, right: 120, display: "flex", justifyContent: "center", flexWrap: "wrap", gap: "12px 36px", fontSize: 28, color: C.muted, ...useRise(46) }}>
        {data.facts.map((f) => (
          <span key={f.label}>{f.label} <span style={{ color: C.ink }}><Inline text={f.value} /></span></span>
        ))}
      </div>
      {data.checkpoint ? <CheckpointProgress checkpoint={data.checkpoint} /> : null}
      {data.verdict ? (
        <div style={{ position: "absolute", top: 710, left: 0, right: 0, display: "flex", justifyContent: "center", alignItems: "center", gap: 28, fontSize: 30, ...useRise(58) }}>
          {data.stats ? (
            <span style={{ color: C.muted }}>
              <b style={{ color: C.ok }}>+{data.stats.additions}</b> <b style={{ color: C.bad }}>−{data.stats.deletions}</b> across {data.stats.files} files
            </span>
          ) : null}
          <Chip tone={data.verdict.tone} size={28}>● {data.verdict.label}</Chip>
        </div>
      ) : null}
    </AbsoluteFill>
  );
}

// "Checkpoint N · X of Y phases" with a bar that fills to X of Y.
function CheckpointProgress({ checkpoint }) {
  const fill = useProgress(60, 40);
  const share = checkpoint.total ? checkpoint.done / checkpoint.total : 0;
  return (
    <div style={{ position: "absolute", top: 700, left: 560, right: 560, ...useRise(52) }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", fontSize: 30, marginBottom: 14 }}>
        <b>Checkpoint {checkpoint.number}</b>
        <span style={{ color: C["ink-2"] }}>
          <b style={{ color: C.ink }}>{Math.round(checkpoint.done * fill)}</b> of {checkpoint.total} phases{checkpoint.since ? <span style={{ color: C.muted }}> · since <Inline text={checkpoint.since} /></span> : null}
        </span>
      </div>
      <div style={{ height: 18, borderRadius: 9, background: C["surface-2"], border: `1px solid ${C.line}`, overflow: "hidden" }}>
        <div style={{ height: "100%", width: `${share * fill * 100}%`, background: C.accent, borderRadius: 9 }} />
      </div>
    </div>
  );
}

// The TL;DR callout: each beat arrives at its frame with a snapcn marker over its label.
function Tldr({ data }) {
  const box = { ...BOX, top: 130, height: CONTENT.y + CONTENT.h - 130 };
  const area = { w: CONTENT.w - 120, h: (box.height - 90) / data.beats.length };
  return (
    <div style={{ ...box, background: C["callout-bg"], borderLeft: `6px solid ${C.accent}`, borderRadius: 12, ...useRise(0) }}>
      <div style={{ position: "absolute", left: 40, top: 26, color: C.accent, fontWeight: 700, fontSize: 24, letterSpacing: "0.1em" }}>TL;DR</div>
      {data.beats.map((beat, i) => {
        const size = fitFont(`${beat.label}: ${beat.text}`, { w: area.w * 0.8, h: area.h - 16 }, { min: 26, max: 46 });
        return (
          <Sequence key={beat.label} from={data.starts[i]} layout="none">
            <div style={{ position: "absolute", left: 60, width: area.w, top: 70 + i * area.h, height: area.h }}>
              <TextHighlight preset="marker" before="" highlight={beat.label} after={<span>: <Inline text={beat.text} /></span>}
                fontSize={size} fontWeight={500} startAt={4} drawDuration={16} />
            </div>
          </Sequence>
        );
      })}
    </div>
  );
}

function DoneWhen({ data, mode }) {
  const ship = mode === "ship";
  const grid = ship ? "1.1fr 1fr 170px" : "1fr 1fr";
  return (
    <div style={BOX}>
      {data.counts ? (
        <div style={{ display: "flex", gap: 14, marginBottom: 18, ...useRise(0) }}>
          {data.counts.map((c) => <Chip key={c.status} tone={c.status} size={24}>{c.n} {c.status}</Chip>)}
        </div>
      ) : null}
      <div style={{ border: `1px solid ${C.line}`, borderRadius: 12, overflow: "hidden" }}>
        <div style={{ display: "grid", gridTemplateColumns: grid, background: C["surface-2"], fontWeight: 700, fontSize: 24, color: C["ink-2"] }}>
          {data.columns.map((col) => <div key={col} style={{ padding: "14px 22px" }}>{col}</div>)}
        </div>
        {data.rows.map((row, i) => (
          <Row key={i} delay={8 + i * STAGGER} grid={grid}>
            {row.cells.map((cell, j) => (
              <div key={j} style={{ padding: "14px 22px" }}>
                <Clamp lines={2} style={{ fontSize: 26, lineHeight: 1.35, color: j ? C["ink-2"] : C.ink }}><Inline text={cell} /></Clamp>
              </div>
            ))}
            {row.status ? <div style={{ padding: "14px 22px" }}><Chip tone={row.status}>{row.status}</Chip></div> : null}
          </Row>
        ))}
      </div>
      <More n={data.more} what={ship ? "more delivered rows" : "more criteria"} />
    </div>
  );
}

function Row({ delay, grid, children }) {
  return <div style={{ display: "grid", gridTemplateColumns: grid, borderTop: `1px solid ${C.line}`, alignItems: "center", ...useRise(delay) }}>{children}</div>;
}

// ---- plan ----

function Phases({ data }) {
  const count = data.groups.reduce((n, g) => n + g.rows.length, 0);
  const big = count <= 5 && data.groups.length === 1;
  let index = 0;
  return (
    <div style={BOX}>
      {data.groups.map((group) => (
        <div key={group.title ?? "phases"} style={{ marginBottom: 14 }}>
          {group.title ? <div style={{ color: C.muted, fontSize: 22, fontWeight: 700, letterSpacing: "0.06em", textTransform: "uppercase", margin: "6px 0" }}><Inline text={group.title} /></div> : null}
          {group.rows.map((row) => <PhaseRow key={index} row={row} big={big} delay={8 + (index++) * STAGGER * 2} />)}
        </div>
      ))}
      <More n={data.more} what="more phases" />
    </div>
  );
}

function PhaseRow({ row, delay, big }) {
  const tick = useProgress(delay + 14, 8);
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 20, height: big ? 88 : 62, borderBottom: `1px dashed ${C.line}`, fontSize: big ? 32 : 28, ...useRise(delay) }}>
      <div style={{ width: 30, height: 30, flex: "none", borderRadius: 6, border: `2px solid ${tick > 0 ? C.accent : C.muted}`, background: tick > 0 ? C.accent : "transparent", display: "grid", placeItems: "center", color: C.bg, fontSize: 22, fontWeight: 900, transform: `scale(${1 + 0.2 * Math.sin(Math.PI * tick)})` }}>
        {tick > 0.5 ? "✓" : ""}
      </div>
      <div style={{ flex: 1, minWidth: 0, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
        <b><Inline text={row.title} /></b>
        {row.review ? <span style={{ marginLeft: 12 }}><Chip tone="open" size={18}>review</Chip></span> : null}
        {row.summary ? <span style={{ color: C["ink-2"] }}> — <Inline text={row.summary} /></span> : null}
      </div>
      <div style={{ flex: "none", color: C.muted, fontFamily: FONTS.mono, fontSize: 24 }}>{row.estimate}</div>
    </div>
  );
}

function Architecture({ data, frames }) {
  if (!data.shot) return <Layers data={data} />;
  const second = data.media ? data.starts[1] : frames;
  return (
    <>
      <Shot shot={data.shot} from={0} to={second} />
      {data.media ? <Shot shot={data.media} from={second} to={frames} /> : null}
    </>
  );
}

// A screenshot of the report page, fit into the content box, with a slow push-in.
function Shot({ shot, from, to }) {
  const frame = useCurrentFrame();
  if (frame < from - 10 || frame > to + 10) return null;
  const opacity = interpolate(frame, [from - 10, from + 6, to - 6, to + 10], [0, 1, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const t = interpolate(frame, [from + 15, to - 15], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.inOut(Easing.cubic) });
  const area = { w: CONTENT.w + 80, h: CONTENT.h + 30 };
  // A tall shot is shown at up to SHOT_PAN times the height of the area and pans from its top to
  // its bottom. A shot that fits pushes in slowly.
  const fit = Math.min(area.w / shot.width, (area.h * SHOT_PAN) / shot.height);
  const w = shot.width * fit;
  const h = shot.height * fit;
  const pan = Math.max(0, h - area.h);
  const scale = pan ? 1 : 1 + 0.07 * t;
  return (
    <div style={{ position: "absolute", left: CONTENT.x - 40, top: CONTENT.y - 10, width: area.w, height: area.h, opacity, overflow: "hidden" }}>
      <Img src={staticFile(shot.src)} style={{ position: "absolute", left: (area.w - w) / 2, top: pan ? -pan * t : (area.h - h) / 2, width: w, height: h, borderRadius: 12, border: `1px solid ${C.line}`, transform: `scale(${scale})` }} />
    </div>
  );
}

// Without a screenshot: the layers of the after state, each component boxed in its status color.
function Layers({ data }) {
  return (
    <div style={BOX}>
      {data.layers.map((layer, i) => (
        <div key={layer.name} style={{ display: "flex", alignItems: "center", gap: 18, marginBottom: 22, ...useRise(8 + i * STAGGER * 2) }}>
          <div style={{ width: 260, flex: "none", color: C.muted, fontSize: 24, fontWeight: 700 }}><Inline text={layer.name} /></div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 14 }}>
            {layer.components.map((c) => <Chip key={c.name} tone={c.status} size={24}><Inline text={c.name} /></Chip>)}
          </div>
        </div>
      ))}
      <Clamp lines={2} style={{ color: C["ink-2"], fontSize: 26, marginTop: 12 }}><Inline text={data.summary} /></Clamp>
    </div>
  );
}

function Decisions({ data, frames }) {
  const frame = useCurrentFrame();
  return (
    <div style={BOX}>
      {data.pages.map((page, p) => {
        const from = data.starts[p];
        const to = data.starts[p + 1] ?? frames;
        if (frame < from - 8 || frame > to + 8) return null;
        const opacity = interpolate(frame, [from - 8, from + 4, to - 4, to + 8], [0, 1, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
        return (
          <Sequence key={p} from={from} layout="none">
            <div style={{ position: "absolute", inset: 0, display: "grid", gridTemplateColumns: "repeat(3, 1fr)", alignItems: "start", gap: 28, opacity }}>
              {page.map((card, i) => <DecisionCard key={i} card={card} delay={4 + i * STAGGER * 2} />)}
            </div>
          </Sequence>
        );
      })}
      {data.more ? <div style={{ position: "absolute", bottom: -40, color: C.muted, fontSize: 24 }}>+{data.more} more decisions</div> : null}
    </div>
  );
}

function DecisionCard({ card, delay }) {
  const strike = useProgress(delay + 18, 14);
  const label = { color: C.muted, fontSize: 20, fontWeight: 700, letterSpacing: "0.1em", marginTop: 18, marginBottom: 6 };
  return (
    <div style={{ background: C.surface, border: `1px solid ${C.line}`, borderRadius: 12, padding: 28, ...useRise(delay) }}>
      <Clamp lines={2} style={{ fontSize: 34, fontWeight: 700, lineHeight: 1.25 }}><Inline text={card.title} /></Clamp>
      <div style={label}>DECISION</div>
      <Clamp lines={3} style={{ fontSize: 28, lineHeight: 1.35 }}><Inline text={card.decision} /></Clamp>
      <div style={label}>REJECTED</div>
      {card.rejected.map((option) => (
        <div key={option} style={{ fontSize: 25, color: C.muted, lineHeight: 1.4, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
          <span style={{ position: "relative" }}>
            <Inline text={option} />
            <span style={{ position: "absolute", left: 0, top: "52%", height: 3, width: `${strike * 100}%`, background: C.bad, borderRadius: 2 }} />
          </span>
        </div>
      ))}
      <div style={label}>WHY</div>
      <Clamp lines={4} style={{ fontSize: 26, lineHeight: 1.35, color: C["ink-2"] }}><Inline text={card.why} /></Clamp>
    </div>
  );
}

function RabbitHoles({ data }) {
  return (
    <div style={{ ...BOX, paddingLeft: 40 }}>
      {data.items.map((item, i) => (
        <div key={i} style={{ display: "flex", gap: 22, fontSize: 32, lineHeight: 1.35, marginBottom: 22, ...useRise(8 + i * STAGGER * 2) }}>
          <span style={{ color: C.muted }}>•</span>
          <Clamp lines={2}>{item.lead ? <b style={{ color: C.bad }}>{item.lead} </b> : null}<Inline text={item.text} /></Clamp>
        </div>
      ))}
      <More n={data.more} />
    </div>
  );
}

// ---- ship ----

// The list of all changes on the left; the featured changes one by one on the right. The beat of
// the sequence diagram shows its screenshot over the whole content box.
function Changes({ data, frames }) {
  const frame = useCurrentFrame();
  let current = 0;
  data.starts.forEach((s, i) => { if (frame >= s) current = i; });
  const active = data.beats[current]?.n;
  const window = (i) => [data.starts[i], data.starts[i + 1] ?? frames];
  const shotBeat = data.beats.findIndex((b) => b.type === "shot");
  const [shotFrom, shotTo] = shotBeat >= 0 ? window(shotBeat) : [frames + 99, frames + 99];
  const listOpacity = interpolate(frame, [shotFrom - 8, shotFrom + 4, shotTo - 4, shotTo + 8], [1, 0, 0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <div style={BOX}>
      <div style={{ position: "absolute", left: 0, top: 0, width: 540, opacity: listOpacity }}>
        {data.list.map((c, i) => (
          <div key={c.n} style={{ display: "flex", alignItems: "center", gap: 14, height: 40, fontSize: 24, color: c.n === active ? C.ink : C.muted, fontWeight: c.n === active ? 700 : 400, ...useRise(4 + i * 3) }}>
            <Badge n={c.n} on={c.n === active} size={30} />
            <span style={{ whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}><Inline text={c.area} /></span>
          </div>
        ))}
        <More n={data.listMore} what="more changes" />
      </div>
      {data.beats.map((beat, i) => {
        const [from, to] = window(i);
        if (frame < from - 8 || frame > to + 8) return null;
        if (beat.type === "shot") {
          return <div key={`shot-${beat.n}`} style={{ position: "absolute", left: -CONTENT.x, top: -CONTENT.y }}><Shot shot={data.sequenceShot} from={from} to={to} /></div>;
        }
        const opacity = interpolate(frame, [from - 8, from + 6, to - 4, to + 8], [0, 1, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
        return (
          <Sequence key={beat.n} from={from} layout="none">
            <ChangeCard change={data.featured.find((c) => c.n === beat.n)} opacity={opacity} />
          </Sequence>
        );
      })}
    </div>
  );
}

function ChangeCard({ change, opacity }) {
  const rise = useRise(0);
  return (
    <div style={{ position: "absolute", left: 600, top: 0, width: 1080, height: CONTENT.h, background: C.surface, border: `1px solid ${C.line}`, borderRadius: 14, padding: "30px 36px", opacity, transform: rise.transform }}>
      <div style={{ display: "flex", alignItems: "center", gap: 18 }}>
        <Badge n={change.n} on size={44} />
        <Clamp lines={1} style={{ fontSize: 38, fontWeight: 700 }}><Inline text={change.area} /></Clamp>
      </div>
      <Clamp lines={3} style={{ fontSize: 28, lineHeight: 1.4, marginTop: 18 }}><Inline text={change.how} /></Clamp>
      <Clamp lines={2} style={{ fontSize: 24, lineHeight: 1.4, marginTop: 12, color: C.muted }}>Why: <Inline text={change.why} /></Clamp>
      <div style={{ marginTop: 22 }}>
        {change.files.map((f, i) => (
          <div key={f.path} style={{ display: "flex", gap: 16, alignItems: "baseline", fontFamily: FONTS.mono, fontSize: 21, padding: "7px 0", borderTop: `1px solid ${C.line}`, ...useRise(10 + i * 4) }}>
            <span style={{ flex: 1, minWidth: 0, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis", color: C["ink-2"] }}>
              {f.path}{f.note ? <span style={{ fontFamily: FONTS.sans, color: C.muted }}> ({f.note})</span> : null}
            </span>
            <b style={{ color: C.ok }}>+{f.additions}</b>
            <b style={{ color: C.bad }}>−{f.deletions}</b>
          </div>
        ))}
        <More n={change.filesMore} what="more files" />
      </div>
    </div>
  );
}

function Badge({ n, on, size }) {
  return (
    <span style={{ width: size, height: size, flex: "none", borderRadius: "50%", display: "grid", placeItems: "center", fontSize: size * 0.5, fontWeight: 700, background: on ? C.accent : C["surface-2"], color: on ? C.bg : C.muted }}>{n}</span>
  );
}

function Findings({ data }) {
  return (
    <div style={BOX}>
      <div style={{ display: "flex", gap: 14, marginBottom: 20, ...useRise(0) }}>
        {data.counts.map((c) => <Chip key={c.severity} tone={c.severity} size={24}>{c.n} {c.severity}</Chip>)}
      </div>
      {data.items.map((f, i) => (
        <div key={i} style={{ display: "flex", alignItems: "center", gap: 18, background: C.surface, border: `1px solid ${C.line}`, borderRadius: 12, padding: "16px 22px", marginBottom: 12, ...useRise(8 + i * STAGGER * 2) }}>
          <Chip tone={f.severity}>{f.severity}</Chip>
          <Clamp lines={2} style={{ flex: 1, minWidth: 0, fontSize: 26, fontWeight: 600, lineHeight: 1.3 }}><Inline text={f.title} /></Clamp>
          {f.location ? <span style={{ fontFamily: FONTS.mono, fontSize: 20, color: C["ink-2"], background: C["code-bg"], borderRadius: 6, padding: "4px 10px", whiteSpace: "nowrap" }}>{f.location}</span> : null}
          {f.status !== "open" ? <Chip tone={f.status}>{f.status}</Chip> : null}
        </div>
      ))}
      <More n={data.more} what="more findings" />
    </div>
  );
}

// The steps on the left; the snapcn terminal types the commands of the first steps on the right.
function HandTest({ data }) {
  const frame = useCurrentFrame();
  let current = 0;
  data.starts.forEach((s, i) => { if (frame >= s) current = i; });
  const lines = data.lines.map(({ text, type, delay, pause }) => ({ text, type, delay, pause }));
  return (
    <div style={BOX}>
      <div style={{ position: "absolute", left: 0, top: 0, width: 600 }}>
        {data.steps.map((s, i) => {
          const on = i === current && i < data.terminalSteps;
          return (
            <div key={s.n} style={{ display: "flex", gap: 14, alignItems: "flex-start", marginBottom: 14, fontSize: 25, lineHeight: 1.3, color: on ? C.ink : i < data.terminalSteps ? C["ink-2"] : C.muted, ...useRise(4 + i * 4) }}>
              <Badge n={s.n} on={on} size={34} />
              <Clamp lines={2} style={{ fontWeight: on ? 700 : 400 }}><Inline text={s.title} /></Clamp>
            </div>
          );
        })}
        <More n={data.more} what="more steps" />
      </div>
      <div style={{ position: "absolute", left: 640, top: 0, width: CONTENT.w - 640, height: CONTENT.h, borderRadius: 14, overflow: "hidden", border: `1px solid ${C.line}` }}>
        <TerminalSimulator lines={lines} intro={null} command={null} zoom={{ enabled: true, scale: 1 }} viewport={TERM_VIEWPORT}
          fontSize={(18 * TERM_FONT) / 44} charsPerFrame={TERMINAL.charsPerFrame} chunkSize={TERMINAL.chunkSize} />
      </div>
    </div>
  );
}

function Rollback({ data }) {
  return (
    <div style={{ ...BOX, background: C.surface, borderLeft: `6px solid ${C.warn}`, borderRadius: 12, padding: "30px 40px" }}>
      {data.steps.map((s, i) => (
        <div key={i} style={{ display: "flex", gap: 18, marginBottom: 22, ...useRise(8 + i * STAGGER * 3) }}>
          <Badge n={i + 1} on size={36} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <Clamp lines={2} style={{ fontSize: 27, lineHeight: 1.35 }}><Inline text={s.step} /></Clamp>
            {s.command ? (
              <Clamp lines={2} style={{ marginTop: 8, fontFamily: FONTS.mono, fontSize: 22, whiteSpace: "pre-wrap", background: C["code-bg"], border: `1px solid ${C.line}`, borderRadius: 8, padding: "8px 14px", color: C["ink-2"] }}>{s.command}</Clamp>
            ) : null}
          </div>
        </div>
      ))}
      <More n={data.more} what="more steps" />
    </div>
  );
}

// The snapcn status-cycle: "<title> is" with a pill that rolls through the statuses. Each status
// holds for an equal share of the scene, so the chip act of the component never starts.
function Outro({ data }) {
  return (
    <StatusCycle prefix={data.prefix} statuses={data.statuses} chips={NO_CHIPS} statusHold={data.statusHold} fieldColor={data.tone === "bad" ? C.bad : C.accent}
      pageColor={C.bg} pillColor={C.ink} prefixColor={C.bg} fontWeight={600} fontFamily={FONTS.sans} />
  );
}
