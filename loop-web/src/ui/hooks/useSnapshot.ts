import { useEffect, useRef, useState } from "react";
import type { Snapshot } from "../../model/types.ts";

export type ConnState = "connecting" | "live" | "dropped";

/**
 * Subscribe to the server's SSE stream. The server pushes a full snapshot on connect and
 * again on every debounced change / 15s reconcile — the state is tiny, so there are no
 * deltas to reconcile client-side. Returns the latest snapshot and the connection state.
 */
export function useSnapshot(): { snapshot: Snapshot | null; conn: ConnState } {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [conn, setConn] = useState<ConnState>("connecting");
  const esRef = useRef<EventSource | null>(null);

  useEffect(() => {
    const es = new EventSource("/events");
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
  }, []);

  return { snapshot, conn };
}
