import { useState } from "react";
import type { LoopListEntry } from "../../model/store-types.ts";
import { loopStatusColor } from "../theme/glyphs.ts";

/**
 * Header popover that switches the observed loop. Each row shows a lifecycle dot, the effort name,
 * a status pill, phase counts ("3/4 done") and a relative timestamp; archived loops warn that their
 * logs may be gone (their worktree was cleaned up). Clicking a row selects that loop.
 */
export function LoopSelector({
  loops,
  currentRunId,
  onSelect,
}: {
  loops: LoopListEntry[];
  currentRunId: string | null;
  onSelect: (runId: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const current = loops.find((l) => l.runId === currentRunId) ?? null;

  return (
    <div className="loopsel">
      <button className="loopsel__btn" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <span className="loopsel__dot" style={{ background: loopStatusColor(current?.status ?? "") }} />
        <span className="loopsel__name">{current?.effort ?? "select loop"}</span>
        <span className="loopsel__caret">▾</span>
      </button>

      {open && (
        <>
          <div className="loopsel__scrim" onClick={() => setOpen(false)} />
          <div className="loopsel__pop">
            {loops.length === 0 ? (
              <div className="rail__empty" style={{ padding: 16 }}>No loops observed yet.</div>
            ) : (
              loops.map((l) => (
                <button
                  key={l.runId}
                  className={`loopsel__row${l.runId === currentRunId ? " loopsel__row--active" : ""}`}
                  onClick={() => {
                    onSelect(l.runId);
                    setOpen(false);
                  }}
                >
                  <span className="loopsel__dot" style={{ background: loopStatusColor(l.status) }} />
                  <span className="loopsel__rowmain">
                    <span className="loopsel__rowtop">
                      <span className="loopsel__name">{l.effort ?? l.runId}</span>
                      <span className="pill">{l.status}</span>
                    </span>
                    <span className="loopsel__rowsub">
                      <span>{doneCount(l)}/{l.phaseCounts.total} done</span>
                      <span className="loopsel__time">{fmtRelative(l.updatedAt)}</span>
                    </span>
                    {l.status === "archived" && (
                      <span className="loopsel__hint">logs may be unavailable</span>
                    )}
                  </span>
                </button>
              ))
            )}
          </div>
        </>
      )}
    </div>
  );
}

function doneCount(l: LoopListEntry): number {
  return l.phaseCounts.done + l.phaseCounts.merged;
}

function fmtRelative(iso: string | null): string {
  if (!iso) return "—";
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return "—";
  const sec = Math.max(0, Math.round((Date.now() - t) / 1000));
  if (sec < 60) return `${sec}s ago`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.round(min / 60);
  if (hr < 48) return `${hr}h ago`;
  return `${Math.round(hr / 24)}d ago`;
}
