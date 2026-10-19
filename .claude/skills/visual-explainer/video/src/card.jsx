// The card on the right of the frame: the title, the key facts of one node, a list of the focus
// nodes, the groups of an overview, or the end. The storyboard has cut every text to fit.
import { CARD } from "./storyboard.mjs";
import { COLORS as C, FONTS } from "./theme.mjs";
import { Inline, useRise } from "./ui.jsx";

const PAD = 36;
const CARDS = { title: TitleCard, end: EndCard, overview: OverviewCard, node: NodeCard, list: ListCard };

export function StopCard({ stop, kinds }) {
  const Card = CARDS[stop.card.type];
  return (
    <div style={{ position: "absolute", left: CARD.x, top: CARD.y, width: CARD.w, height: CARD.h, boxSizing: "border-box", padding: PAD,
      background: C.surface, border: `2px solid ${C.line}`, borderRadius: 28, overflow: "hidden", display: "flex", flexDirection: "column", gap: 22 }}>
      <Card card={stop.card} kinds={kinds} />
    </div>
  );
}

function TitleCard({ card, kinds }) {
  return (
    <>
      <p style={{ ...eyebrow(C.accent), ...useRise(0) }}>{card.eyebrow}</p>
      <h1 style={{ margin: 0, fontSize: 62, lineHeight: 1.05, letterSpacing: "-0.02em", ...useRise(4) }}>{card.title}</h1>
      <p style={{ margin: 0, fontSize: 30, lineHeight: 1.35, color: C["ink-2"], ...useRise(8) }}><Inline text={card.subtitle} /></p>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 12, marginTop: "auto", ...useRise(14) }}>
        {card.stats.map(({ kind, count }) => (
          <span key={kind} style={{ display: "inline-flex", alignItems: "center", gap: 10, padding: "6px 16px", borderRadius: 999, border: `2px solid ${C.line}`, fontSize: 24 }}>
            <i style={{ width: 14, height: 14, borderRadius: 4, background: kindColor(kinds, kind) }} />
            <b>{count}</b> {kinds[kind]?.label ?? kind}
          </span>
        ))}
      </div>
    </>
  );
}

function EndCard({ card }) {
  return (
    <>
      <p style={{ ...eyebrow(C.accent), ...useRise(0) }}>Open the page</p>
      <h1 style={{ margin: 0, fontSize: 58, lineHeight: 1.05, ...useRise(4) }}>{card.title}</h1>
      <p style={{ margin: 0, fontSize: 32, lineHeight: 1.35, color: C["ink-2"], ...useRise(8) }}>{card.text}</p>
    </>
  );
}

function OverviewCard({ card }) {
  return (
    <>
      <p style={{ ...eyebrow(C.accent), ...useRise(0) }}>Overview</p>
      <h2 style={{ margin: 0, fontSize: 44, lineHeight: 1.1, ...useRise(4) }}>{card.title}</h2>
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        {card.items.map((g, i) => (
          <div key={g.id} style={{ display: "flex", justifyContent: "space-between", gap: 16, fontSize: 30, borderBottom: `1px solid ${C.line}`, paddingBottom: 8, ...useRise(8 + i * 3) }}>
            <span>{g.name}</span><b style={{ color: C["ink-2"] }}>{g.count}</b>
          </div>
        ))}
      </div>
    </>
  );
}

function NodeCard({ card, kinds }) {
  const color = kindColor(kinds, card.kind);
  return (
    <>
      <p style={{ ...eyebrow(color), ...useRise(0) }}>{card.kicker}</p>
      <h2 style={{ margin: 0, fontSize: 42, lineHeight: 1.15, borderLeft: `8px solid ${card.tone ? C[card.tone] : color}`, paddingLeft: 18, ...useRise(4) }}>
        <Inline text={card.title} />
      </h2>
      {card.badges.length > 0 && (
        <div style={{ display: "flex", flexWrap: "wrap", gap: 10, ...useRise(8) }}>
          {card.badges.map((b) => <span key={b} style={{ fontSize: 22, padding: "3px 14px", borderRadius: 999, background: C["surface-2"], color: C["ink-2"] }}>{b}</span>)}
        </div>
      )}
      {card.facts.map((f, i) => (
        <div key={f.label} style={{ display: "flex", flexDirection: "column", gap: 4, ...useRise(10 + i * 4) }}>
          <span style={label()}>{f.label}</span>
          <span style={{ fontSize: 26, lineHeight: 1.35 }}><Inline text={f.text} /></span>
        </div>
      ))}
      {card.connections > 0 && <p style={{ margin: "auto 0 0", fontSize: 22, color: C.muted, ...useRise(22) }}>{card.connections} {card.connections === 1 ? "connection" : "connections"} on the page</p>}
    </>
  );
}

function ListCard({ card, kinds }) {
  return (
    <>
      <h2 style={{ margin: 0, fontSize: 40, lineHeight: 1.15, ...useRise(0) }}><Inline text={card.title} /></h2>
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        {card.items.map((item, i) => (
          <div key={item.id} style={{ display: "grid", gridTemplateColumns: "8px minmax(0, 1fr)", gap: 16, ...useRise(6 + i * 3) }}>
            <i style={{ borderRadius: 4, background: item.tone ? C[item.tone] : kindColor(kinds, item.kind) }} />
            <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0 }}>
              <span style={{ ...eyebrow(kindColor(kinds, item.kind)), fontSize: 18 }}>{item.kicker}</span>
              <span style={{ fontSize: 27, lineHeight: 1.25 }}><Inline text={item.title} /></span>
            </div>
          </div>
        ))}
      </div>
      {card.more > 0 && <p style={{ margin: "auto 0 0", fontSize: 24, color: C.accent, fontWeight: 600 }}>+{card.more} more on the page</p>}
    </>
  );
}

function kindColor(kinds, kind) {
  return C[kinds[kind]?.color] ?? C.accent;
}

function eyebrow(color) {
  return { margin: 0, color, fontWeight: 700, letterSpacing: "0.07em", textTransform: "uppercase", fontSize: 22, fontFamily: FONTS.sans };
}

function label() {
  return { fontSize: 20, textTransform: "uppercase", letterSpacing: "0.06em", color: C.muted, fontWeight: 700 };
}
