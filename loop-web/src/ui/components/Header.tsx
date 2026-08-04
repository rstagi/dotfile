import type { Snapshot } from "../../model/types.ts";
import type { LoopListEntry } from "../../model/store-types.ts";
import type { ConnState } from "../hooks/useSnapshot.ts";
import { LoopSelector } from "./LoopSelector.tsx";

export function Header({
  snapshot,
  conn,
  loops = [],
  currentRunId = null,
  onSelectRun,
}: {
  snapshot: Snapshot | null;
  conn: ConnState;
  loops?: LoopListEntry[];
  currentRunId?: string | null;
  onSelectRun?: (runId: string) => void;
}) {
  const e = snapshot?.effort;
  const connLabel = conn === "live" ? "streaming" : conn === "connecting" ? "connecting" : "reconnecting";
  return (
    <header className="hdr">
      <div className="hdr__brand">
        <span className="hdr__logo">◉ LOOP OBSERVATORY</span>
      </div>
      <div className="hdr__sep" />
      {onSelectRun && <LoopSelector loops={loops} currentRunId={currentRunId} onSelect={onSelectRun} />}
      <div className="hdr__field">
        <span className="eyebrow">effort</span>
        <b className="display">{e?.name ?? "—"}</b>
      </div>
      {e?.integrationBranch && (
        <div className="hdr__field">
          <span className="eyebrow">integration</span>
          <b>{e.integrationBranch}</b>
        </div>
      )}
      {e?.pr?.url && (
        <div className="hdr__field">
          <span className="eyebrow">pull request</span>
          <b>
            <a href={e.pr.url} target="_blank" rel="noreferrer">
              {prLabel(e.pr.url)}
            </a>
          </b>
        </div>
      )}

      <div className="hdr__spacer" />

      {e?.status && <span className="pill">{e.status}</span>}
      {snapshot?.loopActive && (
        <span className="pill" style={{ color: "var(--aqua)", borderColor: "var(--aqua-deep)" }}>
          loop active
        </span>
      )}
      <span className={`conn conn--${conn}`}>
        <span className="conn__dot" />
        {connLabel}
      </span>
    </header>
  );
}

function prLabel(url: string): string {
  const m = url.match(/\/pull\/(\d+)/) ?? url.match(/\/(\d+)$/);
  return m ? `#${m[1]}` : "open";
}
