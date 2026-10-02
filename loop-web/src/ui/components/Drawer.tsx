import { useEffect, useMemo, useState } from "react";
import type { GraphNode, PrInfo, PlanOverview, AttemptSummary, ReviewStageRun } from "../../model/types.ts";
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
        {(node.decisions?.length ?? 0) > 0 && <JevDecisionSection decisions={node.decisions!} />}
        {node.kind === "pr-review" && (
          <ReviewSection
            runId={runId}
            pr={pr}
            repository={node.repository}
            currentRound={node.phase == null || node.status === "done" || node.status === "blocked"}
          />
        )}
        {(node.reviewStages?.length ?? 0) > 0 && <ReviewPipelineSection stages={node.reviewStages!} />}
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

/** Every run of the review pipeline (rounds → fixes → final), per repository. */
function ReviewPipelineSection({ stages }: { stages: ReviewStageRun[] }) {
  const repositories = [...new Set(stages.map((s) => s.repository))];
  return (
    <section>
      <p className="section__title">Review pipeline</p>
      {repositories.map((repository) => (
        <div className="review-pipeline" key={repository}>
          {repositories.length > 1 && <div className="review-pipeline__repo">{repository}</div>}
          {stages.filter((s) => s.repository === repository).map((s) => (
            <div className="review-pipeline__run" key={s.k} title={s.summary ?? undefined}>
              <span className="review-pipeline__k">a{s.k}</span>
              <span>{s.stage}</span>
              <span className="review-pipeline__chain">{s.chain.replace("review-", "")}</span>
              <span style={{ color: runColor(s.state) }}>{s.state}</span>
              <span className="review-pipeline__chain">{s.engine ? [s.engine, s.model].filter(Boolean).join(":") : "—"}</span>
            </div>
          ))}
        </div>
      ))}
    </section>
  );
}

function runColor(state: ReviewStageRun["state"]): string {
  if (state === "question") return uiColor("awaiting");
  if (state === "failed" || state === "blocked") return uiColor("problem");
  return uiColor(state);
}

function JevDecisionSection({ decisions }: { decisions: NonNullable<GraphNode["decisions"]> }) {
  return (
    <section>
      <p className="section__title">Jev decisions</p>
      <div className="jev-decisions">
        {decisions.slice().reverse().map((decision) => (
          <div className="jev-decision" key={`${decision.runId}:${decision.phase}:${decision.attempt}:${decision.stage}:${decision.ts ?? ""}:${decision.head ?? ""}`}>
            <div className="jev-decision__head">
              <b>{decision.stage}</b>
              <span>{decision.mode} · a{decision.attempt}</span>
            </div>
            <dl className="kv">
              <dt>{decision.stage === "route" ? "proposed profile" : decision.stage === "question" ? "triage suggestion" : decision.stage === "merge-risk" ? "risk judgment" : "proposed"}</dt>
              <dd>{decision.candidate ?? "—"}</dd>
              <dt>{decision.stage === "route" ? "actual profile" : decision.stage === "question" ? "actual action" : decision.stage === "merge-risk" ? "actual disposition" : "applied"}</dt>
              <dd>{decision.appliedAction ?? "—"}</dd>
              {decision.stage === "question" && (
                <><dt>evidence checked</dt><dd>{decision.evidenceChecked ? decision.evidenceSources?.join(", ") || "yes" : "no"}</dd></>
              )}
              {decision.stage === "merge-risk" && (
                <>
                  {decision.head && <><dt>head</dt><dd>{decision.head}</dd></>}
                  {decision.requiredGates && <><dt>required gates</dt><dd>{decision.requiredGates.join(", ") || "none"}</dd></>}
                  {decision.completedGates && <><dt>completed gates</dt><dd>{decision.completedGates.join(", ") || "none"}</dd></>}
                  {decision.remainingGates && <><dt>remaining gates</dt><dd>{decision.remainingGates.join(", ") || "none"}</dd></>}
                  {decision.focus && <><dt>focus</dt><dd>{decision.focus.join(", ") || "none"}</dd></>}
                </>
              )}
              <dt>confidence</dt><dd>{formatConfidence(decision.confidence)}</dd>
              {Object.keys(decision.probabilities).length > 0 && (
                <><dt>probabilities</dt><dd>{formatProbabilities(decision.probabilities)}</dd></>
              )}
              <dt>fallback</dt><dd>{decision.fallbackReason ?? "none"}</dd>
              <dt>model</dt><dd>{decision.resolvedModel ?? "—"}</dd>
            </dl>
          </div>
        ))}
      </div>
    </section>
  );
}

function formatConfidence(value: number | null): string {
  return value == null ? "—" : `${Math.round(value * 100)}%`;
}

function formatProbabilities(values: Record<string, number>): string {
  return Object.entries(values).map(([key, value]) => `${key} ${Math.round(value * 100)}%`).join(" · ");
}

function SteeringNote({ node, runId }: { node: GraphNode; runId: string }) {
  const [markdown, setMarkdown] = useState(node.noteMarkdown ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const key = node.kind === "pr-review" && node.phase == null
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

function ReviewSection({ runId, pr, repository, currentRound }: {
  runId: string | null;
  pr: PrInfo | null;
  repository: string | null;
  currentRound: boolean;
}) {
  const review = useReview(runId, currentRound, repository);
  const outcome = currentRound ? review?.outcome ?? pr?.outcome ?? null : null;
  const summary = currentRound ? review?.summary ?? pr?.verdict ?? null : null;
  const commentUrl = currentRound ? review?.commentUrl ?? pr?.commentUrl ?? null : null;
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

      {currentRound && review === undefined ? (
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
  if (n.kind === "pr-review") return n.phase ? `PHASE ${n.phase} · PR REVIEW` : "TERMINAL · PR REVIEW";
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
