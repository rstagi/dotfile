import { useEffect, useState } from "react";
import type { LoopListEntry } from "../../model/store-types.ts";

const POLL_MS = 5000;

/**
 * Poll `GET /api/loops` every 5s for the loop selector. Polling pauses while the tab is hidden
 * (and fires one immediate refresh when it becomes visible again) so a backgrounded observatory
 * isn't hammering the daemon. Last-known list is kept on a transient fetch error.
 */
export function useLoopList(): LoopListEntry[] {
  const [loops, setLoops] = useState<LoopListEntry[]>([]);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | null = null;

    async function poll() {
      try {
        const r = await fetch("/api/loops");
        if (!r.ok) return;
        const data = (await r.json()) as unknown;
        if (!cancelled && Array.isArray(data)) setLoops(data as LoopListEntry[]);
      } catch {
        /* keep the last-known list */
      }
    }
    const start = () => {
      if (timer == null) timer = setInterval(poll, POLL_MS);
    };
    const stop = () => {
      if (timer != null) {
        clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.hidden) stop();
      else {
        poll();
        start();
      }
    };

    poll();
    if (!document.hidden) start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      cancelled = true;
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, []);

  return loops;
}
