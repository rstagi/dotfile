import type { Problem } from "../../model/types.ts";
import { problemStyle } from "../theme/glyphs.ts";

export function ProblemsRail({
  problems,
  onSelect,
}: {
  problems: Problem[];
  onSelect: (nodeId: string) => void;
}) {
  return (
    <aside className="rail">
      <div className="rail__head">
        <span className="rail__title">Problems</span>
        <span className="rail__count">{problems.length}</span>
      </div>
      <div className="rail__body">
        {problems.length === 0 ? (
          <div className="rail__empty">
            <b>All clear</b>
            No blocked phases, crashes, or HIL pauses. The bench is quiet.
          </div>
        ) : (
          problems.map((p) => {
            const s = problemStyle(p.class);
            return (
              <div
                key={p.id}
                className={`prob${p.class === "hil" ? " prob--hil" : ""}`}
                style={{ borderLeftColor: s.color }}
                onClick={() => onSelect(p.nodeId)}
              >
                <div className="prob__top">
                  <span className="prob__glyph" style={{ color: s.color }}>
                    {s.glyph}
                  </span>
                  <span className="prob__class" style={{ color: s.color }}>
                    {s.label}
                  </span>
                  {p.phase && <span className="prob__phase">Phase {p.phase}</span>}
                </div>
                <div className="prob__title">{p.title}</div>
                {p.detail && <div className="prob__detail">{p.detail}</div>}
              </div>
            );
          })
        )}
      </div>
    </aside>
  );
}
