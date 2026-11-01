// Custom canvas nodes. Each node changes its detail with the zoom (semantic zoom).
import { Handle, Position, useStore } from '@xyflow/react';
import { fmt, PX } from './layout.js';

const useFar = () => useStore((s) => s.transform[2] < 0.5);

const STATUS = {
  done: { label: 'merged', cls: 'st-done' },
  drift: { label: 'merged · drift', cls: 'st-drift' },
  active: { label: 'working', cls: 'st-active' },
  hil: { label: 'needs you', cls: 'st-hil' },
  todo: { label: 'planned', cls: 'st-todo' },
};
const KIND_ICON = { read: '👁', edit: '✎', test: '✓', browse: '◐', think: '◇' };

function Handles() {
  return (
    <>
      <Handle type="target" position={Position.Left} id="left" />
      <Handle type="source" position={Position.Right} id="right" />
      <Handle type="source" position={Position.Bottom} id="bottom" />
    </>
  );
}

export function RunNode({ data, selected }) {
  const { run, summary, finished } = data;
  const far = useFar();
  return (
    <div className={`run ${far ? 'far' : ''} ${selected ? 'is-selected' : ''}`}>
      <div className="run-head">
        <span className={`kind kind-${run.kind.toLowerCase()}`}>{run.kind}</span>
        <span className="run-title">{run.title}</span>
        <span className="run-sub">{run.sub}</span>
        <span className="run-chips">
          {finished && <span className="chip st-done">✓ {run.result} · click to collapse</span>}
          {!finished && summary.done + summary.drift > 0 && <span className="chip st-done">{summary.done + summary.drift} merged</span>}
          {summary.active > 0 && <span className="chip st-active">{summary.active} working</span>}
          {summary.hil > 0 && <span className="chip st-hil">{summary.hil} needs you</span>}
          {summary.drift > 0 && <span className="chip st-drift">⚠ {summary.drift} drift</span>}
          {summary.todo > 0 && <span className="chip st-todo">{summary.todo} planned</span>}
        </span>
      </div>
    </div>
  );
}

export function LaneNode({ data }) {
  const { lane, current, coordinator, waiting } = data;
  const far = useFar();
  let state = 'idle';
  let line = waiting ? waiting.phase.waiting : 'idle';
  if (coordinator) { state = 'active'; line = 'polling every 30 s'; }
  if (current) {
    state = current.status === 'hil' ? 'hil' : 'active';
    line = current.status === 'hil' ? 'waiting for you' : current.live.at(-1)?.actions.at(-1)?.text ?? current.phase.action;
  }
  return (
    <div className={`lane lane-${state} ${far ? 'far' : ''}`}>
      <div className="lane-agent">
        <span className={`pulse pulse-${state}`} />
        <span className="lane-name">{lane.agent}</span>
      </div>
      {!far && <div className="lane-engine">{lane.engine}</div>}
      {!far && <div className="lane-line">{state === 'active' ? '▸ ' : ''}{line}</div>}
    </div>
  );
}

export function PhaseNode({ data, selected }) {
  const { phase, status, intents, minutes, drift, ask } = data;
  const far = useFar();
  const st = STATUS[status];
  const est = phase.est ? `${phase.est[0]}–${phase.est[1]}m` : null;
  const over = phase.est && minutes > phase.est[1];
  const pct = phase.est && minutes != null ? Math.min(100, (minutes / phase.est[1]) * 100) : null;
  return (
    <div className={`phase ${st.cls} ${far ? 'far' : ''} ${selected ? 'is-selected' : ''}`}>
      <Handles />
      <div className="phase-head">
        {phase.key !== phase.title && <span className="phase-key">{phase.key}</span>}
        <span className="phase-title">{phase.title}</span>
        <span className={`pill ${st.cls}`}>{st.label}</span>
      </div>
      {far ? (
        <div className="phase-far">
          {status === 'todo' ? (phase.waiting ?? `after ${(phase.deps ?? []).join(', ')}`) : `${minutes}m`}
          {drift.length > 0 && <span className="far-drift"> ⚠ {drift.length}</span>}
        </div>
      ) : (
        <>
          <div className="phase-meta">
            {phase.task && <span className="taskchip">{phase.task}</span>}
            {minutes != null && <span className={over ? 'over' : ''}>{minutes}m{est ? ` / plan ${est}` : ''}</span>}
            {status === 'todo' && <span>plan {est} · {phase.waiting ?? `after ${(phase.deps ?? []).join(', ')}`}</span>}
            {phase.inferred && <span className="inferred-tag">phase inferred</span>}
          </div>
          {pct != null && <div className="bar"><div className={`bar-fill ${over ? 'bar-over' : ''}`} style={{ width: `${pct}%` }} /></div>}
          {status === 'todo' ? (
            <div className="todo-done-when">Done when: {phase.doneWhen}</div>
          ) : (
            <div className="intents">
              {data.groups.map((g) => (
                <button
                  type="button"
                  key={g.gi}
                  className="igroup nodrag"
                  title="Open the merged intents"
                  onClick={(e) => { e.stopPropagation(); data.openGroup?.(g.gi); }}
                >
                  <span className="ig-text">{g.text}</span>
                  <span className="ig-meta">{g.members.length} {g.members.length === 1 ? 'intent' : 'intents'} · {g.from}{g.to !== g.from ? `–${g.to}` : ''} ›</span>
                </button>
              ))}
              {data.live.map((it, i) => {
                const current = i === data.live.length - 1;
                return (
                  <div key={it.index} className={`ilive ${current ? 'ilive-now' : ''}`}>
                    <div className="il-head">
                      <span className="il-text">{it.text}</span>
                      <span className="it-time">{current ? 'now' : it.t}{it.conf ? ` · ${Math.round(it.conf * 100)}%` : ''}</span>
                    </div>
                    {current && it.actions.slice(-2).map((x) => (
                      <div key={x.t + x.text} className="il-action"><span className={`tool tool-${x.tool.toLowerCase()}`}>{x.tool}</span>{x.text}</div>
                    ))}
                  </div>
                );
              })}
            </div>
          )}
          {ask && <div className="ask">❓ {ask.text}</div>}
          {!ask && drift.length > 0 && <div className="drifts">{drift.map((d) => <span key={d.kind} className="drift-chip">⚠ {d.kind}</span>)}</div>}
        </>
      )}
    </div>
  );
}

export function MediaNode({ data, selected }) {
  const { media } = data;
  const far = useFar();
  return (
    <div className={`media ${selected ? 'is-selected' : ''}`}>
      <Handle type="target" position={Position.Top} />
      <div className="thumb">
        <img src={media.src} alt={media.name} draggable={false} />
        {media.gif && <span className="gif">▶ GIF</span>}
      </div>
      {!far && <div className="media-name">{media.name}</div>}
    </div>
  );
}

export function CheckpointNode({ data, selected }) {
  const { cp } = data;
  const far = useFar();
  return (
    <div className={`cp ${far ? 'far' : ''} ${selected ? 'is-selected' : ''}`}>
      <div className={`diamond ${/drift/.test(cp.label) ? 'diamond-drift' : ''}`}><span>{cp.n.replace('CP', '')}</span></div>
      <div className="cp-text">
        <div className="cp-n">{cp.n} · {cp.t}</div>
        {!far && <div className="cp-label">{cp.label}</div>}
      </div>
    </div>
  );
}

export function MergeNode({ data }) {
  return <div className={`merge ${data.drift ? 'merge-drift' : ''}`} title={`${data.phase.key} merged`}><span className="merge-dot" />{data.phase.key}</div>;
}

export function RulerNode({ data }) {
  const ticks = [];
  for (let m = Math.ceil(data.from / 15) * 15; m <= data.to; m += Math.max(15, Math.ceil((data.to - data.from) / 100 / 15) * 15)) ticks.push(m);
  return (
    <div className="ruler" style={{ width: (data.to - data.from) * PX }}>
      {ticks.map((m) => (
        <div key={m} className={`tick ${m % 60 === 0 ? 'tick-hour' : ''}`} style={{ left: (m - data.from) * PX }}>{fmt(m)}</div>
      ))}
    </div>
  );
}

export function NowLineNode({ data }) {
  return (
    <div className="nowline" style={{ height: data.height + 40 }}>
      <span className="now-label">now {fmt(data.now)}</span>
    </div>
  );
}

export function RunCollapsedNode({ data, selected }) {
  const { run, phases, drift, minutes, media } = data;
  const far = useFar();
  return (
    <div className={`run-collapsed ${far ? 'far' : ''} ${selected ? 'is-selected' : ''}`}>
      <div className="rc-main">
        <div className="rc-top">
          <span className={`kind kind-${run.kind.toLowerCase()}`}>{run.kind}</span>
          <span className="rc-title">{run.title}</span>
          <span className="rc-done">✓ {run.result}</span>
        </div>
        <div className="rc-meta">
          {phases} phases · {drift ? <span className="over">⚠ {drift} drift</span> : 'on plan'} · {Math.floor(minutes / 60) ? `${Math.floor(minutes / 60)}h ` : ''}{minutes % 60}m · finished {run.finishedAt}
        </div>
        <div className="rc-hint">Click to expand</div>
      </div>
      <div className="rc-strip">
        {media.slice(0, 4).map((m) => (
          <div key={m.name} className="rc-thumb"><img src={m.src} alt={m.name} draggable={false} />{m.gif && <span className="gif">▶</span>}</div>
        ))}
      </div>
    </div>
  );
}

export function SectionLabelNode({ data }) {
  return <div className="section-label">{data.text}</div>;
}

export const nodeTypes = {
  runCollapsed: RunCollapsedNode, sectionLabel: SectionLabelNode,
  run: RunNode, lane: LaneNode, phase: PhaseNode, media: MediaNode,
  checkpoint: CheckpointNode, merge: MergeNode, ruler: RulerNode, nowLine: NowLineNode,
};
