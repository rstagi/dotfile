import type { TimelineEvent } from "../../model/types.ts";

export function Timeline({ events }: { events: TimelineEvent[] }) {
  return (
    <aside className="rail">
      <div className="rail__head">
        <span className="rail__title">Event Log</span>
        <span className="rail__count">{events.length}</span>
      </div>
      <div className="rail__body tl" style={{ gap: 0 }}>
        {events.length === 0 ? (
          <div className="rail__empty">No events logged yet.</div>
        ) : (
          events.map((e, i) => {
            const tone = e.tone;
            return (
              <div
                key={`${e.ts ?? "x"}-${i}`}
                className={`evt${tone ? ` evt--${tone}` : ""}${e.known ? "" : " evt--unknown"}`}
              >
                <span className="evt__glyph">{e.glyph}</span>
                <div className="evt__body">
                  <div className="evt__row1">
                    <span className="evt__label">{e.label}</span>
                    {e.phase && <span className="evt__phase">P{e.phase}</span>}
                    {e.repository && <span className="evt__phase">{e.repository}</span>}
                    <span className="evt__time">{fmtTime(e.ts)}</span>
                  </div>
                  {e.detail && <div className="evt__detail">{e.detail}</div>}
                </div>
              </div>
            );
          })
        )}
      </div>
    </aside>
  );
}

function fmtTime(ts: string | null): string {
  if (!ts) return "—";
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts;
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}
