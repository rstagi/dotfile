import { useEffect, useRef, useState } from "react";
import type { Snapshot } from "../../model/types.ts";

export type ConnState = "connecting" | "live" | "dropped";

/**
 * Subscribe to a loop's SSE stream. The server pushes a full snapshot on connect and again on
 * every debounced change / reconcile — the state is tiny, so there are no deltas to reconcile
 * client-side. Pass a `runId` to watch that loop (`/events?runId=…`); pass null for the bare
 * `/events` back-compat stream (the default loop). Re-subscribes and clears the snapshot when
 * the runId changes, so the "Waiting for the observer…" empty state shows until the first frame.
 */
export function useSnapshot(runId: string | null): { snapshot: Snapshot | null; conn: ConnState } {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [conn, setConn] = useState<ConnState>("connecting");
  const esRef = useRef<EventSource | null>(null);

  useEffect(() => {
    setSnapshot(null); // clear the previous loop's graph until the new stream's first frame
    setConn("connecting");
    const url = runId ? `/events?runId=${encodeURIComponent(runId)}` : "/events";
    const es = new EventSource(url);
    esRef.current = es;
    es.addEventListener("open", () => setConn("live"));
    es.addEventListener("snapshot", (e) => {
      try {
        const snap = JSON.parse((e as MessageEvent).data) as Snapshot;
        setSnapshot(snap);
        setConn("live");
      } catch {
        /* ignore a malformed frame */
      }
    });
    es.addEventListener("error", () => setConn("dropped")); // EventSource auto-reconnects
    return () => es.close();
  }, [runId]);

  return { snapshot, conn };
}
