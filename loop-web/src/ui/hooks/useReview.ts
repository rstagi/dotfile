import { useEffect, useState } from "react";

export interface ReviewData {
  outcome: string | null;
  summary: string | null;
  reportMarkdown: string | null;
  commentUrl: string | null;
  prUrl: string | null;
}

/**
 * Fetch `GET /api/loops/:runId/review` (the full PR-review report) when the PR-review node is open.
 * `undefined` = loading, `null` = no loop / fetch failed, otherwise the review payload. Disabled
 * (or a null runId, e.g. plan preview) yields `undefined` and issues no request.
 */
export function useReview(
  runId: string | null,
  enabled: boolean,
  repository: string | null = null,
): ReviewData | null | undefined {
  const [state, setState] = useState<ReviewData | null | undefined>(undefined);

  useEffect(() => {
    if (!enabled || !runId) {
      setState(undefined);
      return;
    }
    let cancelled = false;
    setState(undefined);
    const query = repository && repository !== "primary"
      ? `?repository=${encodeURIComponent(repository)}`
      : "";
    fetch(`/api/loops/${encodeURIComponent(runId)}/review${query}`)
      .then((r) => (r.ok ? (r.json() as Promise<ReviewData>) : null))
      .then((d) => !cancelled && setState(d))
      .catch(() => !cancelled && setState(null));
    return () => {
      cancelled = true;
    };
  }, [runId, enabled, repository]);

  return state;
}
