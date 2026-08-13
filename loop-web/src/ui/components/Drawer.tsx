import { useEffect, useMemo, useState } from "react";
import type { GraphNode, PrInfo, PlanOverview, AttemptSummary } from "../../model/types.ts";
import { problemStyle, uiColor, reviewPill } from "../theme/glyphs.ts";
import { useReview } from "../hooks/useReview.ts";
import { Markdown } from "./Markdown.tsx";

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
  plan,
  runId,
  onClose,
}: {
  node: GraphNode;
  pr: PrInfo | null;
  plan: PlanOverview | null;
  runId: string | null;
  onClose: () => void;
}) {
  const rt = node.runtime;
  const attempts = rt?.attempts ?? [];
  const defaultK = useMemo(() => pickDefaultAttempt(attempts), [attempts]);
  const [activeK, setActiveK] = useState<number | null>(defaultK);
  useEffect(() => setActiveK(defaultK), [defaultK, node.id]);
  const canSteer = runId != null && ["phase", "integration", "pr-review"].includes(node.kind);

  const detail = useAttemptDetail(runId, rt?.slug ?? null, activeK);

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
        </dl>

        {node.kind === "plan" && plan && <PlanSection plan={plan} />}
        {node.kind === "pr-review" && <ReviewSection runId={runId} pr={pr} repository={node.repository} />}
        {canSteer && <SteeringNote node={node} runId={runId} />}

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

function SteeringNote({ node, runId }: { node: GraphNode; runId: string }) {
  const [markdown, setMarkdown] = useState(node.noteMarkdown ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const key = node.kind === "pr-review"
    ? node.repository && node.repository !== "primary"
      ? `pr-review.${node.repository.replace("/", "--")}`
      : "pr-review"
    : node.phase;

  useEffect(() => setMarkdown(node.noteMarkdown ?? ""), [node.id, node.noteMarkdown]);

  const update = async (clear: boolean) => {
    if (!key) return;
    setSaving(true);
    setError(null);
    try {
      const response = await fetch(`/api/loops/${encodeURIComponent(runId)}/note`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(clear ? { key, clear: true } : { key, markdown }),
      });
      if (!response.ok) {
        const body = await response.json().catch(() => null);
        throw new Error(body?.error ?? `request failed (${response.status})`);
      }
      if (clear) setMarkdown("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not update note");
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="steering-note">
      <p className="section__title" style={{ color: "var(--aqua)" }}>Steering note</p>
      <textarea
        className="steering-note__input"
        aria-label="Steering note"
        value={markdown}
        placeholder="Extra instructions for this phase…"
        onChange={(event) => setMarkdown(event.target.value)}
      />
      <div className="steering-note__actions">
        <button type="button" disabled={saving} onClick={() => void update(false)}>
          {saving ? "Saving…" : "Save"}
        </button>
        <button type="button" disabled={saving || node.noteMarkdown == null} onClick={() => void update(true)}>
          Clear
        </button>
        {error && <span role="alert">{error}</span>}
      </div>
    </section>
  );
}

// --- plan (effort root) --------------------------------------------------------------

function PlanSection({ plan }: { plan: PlanOverview }) {
  const cfg = plan.loopConfig;
  return (
    <>
      {cfg && (
        <section>
          <p className="section__title">Loop config</p>
          <dl className="kv">
            {cfg.integrationBranch && (
              <>
                <dt>integration</dt>
                <dd>{cfg.integrationBranch}</dd>
              </>
            )}
            {cfg.verify && (
              <>
                <dt>verify</dt>
                <dd>{cfg.verify}</dd>
              </>
            )}
            {cfg.concurrency != null && (
              <>
                <dt>concurrency</dt>
                <dd>{cfg.concurrency}</dd>
              </>
            )}
            <dt>lanes</dt>
            <dd>{plan.laneCount}</dd>
          </dl>
        </section>
      )}

      {plan.repositories.length > 0 && plan.repositories[0].slug !== "primary" && (
        <section>
          <p className="section__title">Repositories</p>
          <div className="plan-repositories">
            {plan.repositories.map((repository) => (
              <div key={repository.slug} className="plan-repository">
                <b>{repository.slug}</b>
                <span>{repository.integrationBranch ?? "—"}</span>
                <span>{repository.verify ?? "missing verify"}</span>
                {repository.pr ? <a href={repository.pr}>{prLabel(repository.pr)}</a> : <span>no PR</span>}
              </div>
            ))}
          </div>
        </section>
      )}

      <ProseSection title="Goal" text={plan.prose.goal} />
      <ProseSection title="Approach & key decisions" text={plan.prose.approach} />
      <ProseSection title="Parallelization guide" text={plan.prose.parallelGuide} />
      <ProseSection title="Progress log" text={plan.prose.progressLog} />

      {plan.phaseSummary.length > 0 && (
        <section>
          <p className="section__title">Phases</p>
          <div className="plan-phases">
            {plan.phaseSummary.map((p) => (
              <div key={p.phase} className="plan-phase">
                <span className="plan-phase__id">P{p.phase}</span>
                <span className="plan-phase__title">{p.title}</span>
                <span className="plan-phase__lane">{p.lane}</span>
                <span className="plan-phase__status" style={{ color: uiColor(p.status) }}>
                  {p.status}
                </span>
              </div>
            ))}
          </div>
        </section>
      )}
    </>
  );
}

function ProseSection({ title, text }: { title: string; text: string | null }) {
  if (!text) return null;
  return (
    <section>
      <p className="section__title">{title}</p>
      <Markdown text={text} />
    </section>
  );
}

// --- pr review -----------------------------------------------------------------------

function ReviewSection({ runId, pr, repository }: {
  runId: string | null;
  pr: PrInfo | null;
  repository: string | null;
}) {
  const review = useReview(runId, true, repository);
  const outcome = review?.outcome ?? pr?.outcome ?? null;
  const summary = review?.summary ?? pr?.verdict ?? null;
  const commentUrl = review?.commentUrl ?? pr?.commentUrl ?? null;
  const prUrl = review?.prUrl ?? pr?.url ?? null;
  const report = review?.reportMarkdown ?? null;
  const pill = reviewPill(outcome);

  return (
    <section>
      <p className="section__title">PR review</p>
      <div className="review__head">
        {pill ? (
          <span className="review__pill" style={{ color: pill.color, borderColor: pill.color }}>
            {pill.label}
          </span>
        ) : (
          <span className="review__pill" style={{ color: "var(--ink-muted)" }}>
            {prUrl ? "awaiting review" : "no PR yet"}
          </span>
        )}
        {prUrl && (
          <a href={prUrl} target="_blank" rel="noreferrer">
            {prLabel(prUrl)}
          </a>
        )}
        {commentUrl && (
          <a href={commentUrl} target="_blank" rel="noreferrer">
            review comment
          </a>
        )}
      </div>

      {summary && <Markdown text={summary} />}

      {review === undefined ? (
        <div className="rail__empty" style={{ padding: 12 }}>Loading review…</div>
      ) : report ? (
        <div className="md--report">
          <Markdown text={report} />
        </div>
      ) : (
        <div className="rail__empty" style={{ padding: 12 }}>No full report stored.</div>
      )}
    </section>
  );
}

// --- attempts ------------------------------------------------------------------------

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

function useAttemptDetail(runId: string | null, slug: string | null, k: number | null): AttemptDetail | null | undefined {
  const [state, setState] = useState<AttemptDetail | null | undefined>(undefined);
  useEffect(() => {
    if (!slug || k == null) {
      setState(null);
      return;
    }
    let cancelled = false;
    setState(undefined);
    const base = runId ? `/api/loops/${encodeURIComponent(runId)}/attempt` : "/api/attempt";
    fetch(`${base}/${encodeURIComponent(slug)}/${k}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d) => !cancelled && setState(d))
      .catch(() => !cancelled && setState(null));
    return () => {
      cancelled = true;
    };
  }, [runId, slug, k]);
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

function prLabel(url: string): string {
  const m = url.match(/\/pull\/(\d+)/) ?? url.match(/\/(\d+)(?:#.*)?$/);
  return m ? `#${m[1]}` : "open PR";
}

function fmtAge(sec: number): string {
  if (sec < 90) return `${sec}s`;
  const m = Math.round(sec / 60);
  return m < 90 ? `${m}m` : `${Math.round(m / 60)}h`;
}
