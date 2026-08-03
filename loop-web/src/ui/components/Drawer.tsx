import { useEffect, useMemo, useState } from "react";
import type { GraphNode, PrInfo, AttemptSummary } from "../../model/types.ts";
import { problemStyle, uiColor } from "../theme/glyphs.ts";

interface AttemptDetail {
  slug: string;
  attempt: number;
  meta: { engine?: string; model?: string; headBefore?: string; headAfter?: string } | null;
  status: { outcome?: string; summary?: string; question?: string; details?: string } | null;
  verifyLog: string | null;
  lastMessage: string | null;
  stderr: string | null;
  transcriptPath: string | null;
}

export function Drawer({
  node,
  pr,
  onClose,
}: {
  node: GraphNode;
  pr: PrInfo | null;
  onClose: () => void;
}) {
  const rt = node.runtime;
  const attempts = rt?.attempts ?? [];
  const defaultK = useMemo(() => pickDefaultAttempt(attempts), [attempts]);
  const [activeK, setActiveK] = useState<number | null>(defaultK);
  useEffect(() => setActiveK(defaultK), [defaultK, node.id]);

  const detail = useAttemptDetail(rt?.slug ?? null, activeK);

  return (
    <div className="drawer">
      <div className="drawer__head">
        <div>
          <div className="eyebrow">{chip(node)}</div>
          <div className="drawer__title">{node.title}</div>
        </div>
        <button className="drawer__close" onClick={onClose} aria-label="close">
          ✕
        </button>
      </div>

      <div className="drawer__body">
        <dl className="kv">
          <dt>state</dt>
          <dd style={{ color: uiColor(node.ui) }}>
            {node.status}
            {node.pulse ? ` · ${node.pulse}` : ""}
          </dd>
          {node.lane && (
            <>
              <dt>lane</dt>
              <dd>{node.lane}</dd>
            </>
          )}
          {rt?.branch && (
            <>
              <dt>branch</dt>
              <dd>{rt.branch}</dd>
            </>
          )}
          {rt?.model && (
            <>
              <dt>last worked by</dt>
              <dd>
                {rt.engine ? `${rt.engine}:` : ""}
                {rt.model}
              </dd>
            </>
          )}
          {rt?.lastHeartbeatAgeSec != null && (
            <>
              <dt>heartbeat</dt>
              <dd>{fmtAge(rt.lastHeartbeatAgeSec)} ago</dd>
            </>
          )}
          {node.kind === "pr-review" && (
            <>
              <dt>pull request</dt>
              <dd>{pr?.url ? <a href={pr.url} target="_blank" rel="noreferrer">{pr.url}</a> : "none yet"}</dd>
              <dt>verdict</dt>
              <dd>{pr?.verdict ?? "—"}</dd>
            </>
          )}
        </dl>

        {rt?.hilOpen && rt.hilMarkdown && (
          <section>
            <p className="section__title" style={{ color: "var(--amber)" }}>
              ⚠ Human-in-the-loop request
            </p>
            <div className="hil-block">
              <pre className="log" style={{ border: "none", background: "transparent", padding: 0 }}>
                {rt.hilMarkdown}
              </pre>
            </div>
          </section>
        )}

        {attempts.length > 0 && (
          <section>
            <p className="section__title">Attempt history</p>
            {attempts
              .slice()
              .sort((a, b) => b.k - a.k)
              .map((a) => (
                <AttemptRow key={a.k} a={a} active={a.k === activeK} onClick={() => setActiveK(a.k)} />
              ))}
          </section>
        )}

        {activeK != null && (
          <section>
            <p className="section__title">
              Attempt a{activeK}
              {detail?.transcriptPath ? ` · ${detail.transcriptPath}` : ""}
            </p>
            {detail === undefined ? (
              <div className="rail__empty" style={{ padding: 12 }}>Loading…</div>
            ) : detail === null ? (
              <div className="rail__empty" style={{ padding: 12 }}>No detail for this attempt.</div>
            ) : (
              <AttemptLogs detail={detail} verifyFail={isVerifyFail(attempts, activeK)} />
            )}
          </section>
        )}
      </div>
    </div>
  );
}

function AttemptRow({ a, active, onClick }: { a: AttemptSummary; active: boolean; onClick: () => void }) {
  const label = a.problem ?? a.outcome ?? (a.ended ? "done" : "running");
  const s = a.problem ? problemStyle(a.problem) : { color: a.ended ? "var(--green)" : "var(--aqua)" };
  return (
    <div className={`attempt${active ? " attempt--active" : ""}`} onClick={onClick}>
      <span className="attempt__k">a{a.k}</span>
      <span style={{ color: "var(--ink-soft)" }}>
        {a.engine ? `${a.engine}:` : ""}
        {a.model ?? "—"}
      </span>
      <span
        className="attempt__tag"
        style={{ color: s.color, border: `1px solid ${s.color}`, opacity: 0.9 }}
      >
        {String(label).replace("-", " ")}
      </span>
    </div>
  );
}

function AttemptLogs({ detail, verifyFail }: { detail: AttemptDetail; verifyFail: boolean }) {
  return (
    <>
      {detail.status?.summary && <Field label="summary" body={detail.status.summary} />}
      {detail.status?.question && <Field label="question" body={detail.status.question} />}
      {detail.status?.details && <Field label="details" body={detail.status.details} />}
      {detail.verifyLog && <Field label="verify.log" body={detail.verifyLog} cls={verifyFail ? "log--verify-fail" : ""} />}
      {detail.lastMessage && <Field label="last.md" body={detail.lastMessage} />}
      {detail.stderr && detail.stderr.trim() && <Field label="stderr.log" body={detail.stderr} />}
    </>
  );
}

function Field({ label, body, cls = "" }: { label: string; body: string; cls?: string }) {
  return (
    <div style={{ marginBottom: 10 }}>
      <p className="section__title" style={{ marginBottom: 4 }}>{label}</p>
      <pre className={`log ${cls}`}>{body}</pre>
    </div>
  );
}

// --- data & helpers ------------------------------------------------------------------

function useAttemptDetail(slug: string | null, k: number | null): AttemptDetail | null | undefined {
  const [state, setState] = useState<AttemptDetail | null | undefined>(undefined);
  useEffect(() => {
    if (!slug || k == null) {
      setState(null);
      return;
    }
    let cancelled = false;
    setState(undefined);
    fetch(`/api/attempt/${encodeURIComponent(slug)}/${k}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d) => !cancelled && setState(d))
      .catch(() => !cancelled && setState(null));
    return () => {
      cancelled = true;
    };
  }, [slug, k]);
  return state;
}

function pickDefaultAttempt(attempts: AttemptSummary[]): number | null {
  if (!attempts.length) return null;
  const ended = attempts.filter((a) => a.ended).sort((a, b) => b.k - a.k);
  return (ended[0] ?? attempts.slice().sort((a, b) => b.k - a.k)[0]).k;
}

function isVerifyFail(attempts: AttemptSummary[], k: number): boolean {
  return attempts.find((a) => a.k === k)?.problem === "verify-fail";
}

function chip(n: GraphNode): string {
  if (n.kind === "plan") return "EFFORT ROOT";
  if (n.kind === "pr-review") return "TERMINAL · PR REVIEW";
  return `PHASE ${n.phase}${n.lane ? ` · LANE ${n.lane}` : ""}`;
}

function fmtAge(sec: number): string {
  if (sec < 90) return `${sec}s`;
  const m = Math.round(sec / 60);
  return m < 90 ? `${m}m` : `${Math.round(m / 60)}h`;
}
