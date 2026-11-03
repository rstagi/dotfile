// Agent cockpit prototype: every run of the owner on one infinite canvas, with a time scrubber.
// Two views: all active runs (the default, for a dedicated tab) and one session (a pane next to a chat).
// The URL holds the view, so a host can open a session directly: ?view=session&chat=<chat id>.
import { useEffect, useMemo, useState } from 'react';
import { ReactFlow, ReactFlowProvider, Background, Controls, useReactFlow } from '@xyflow/react';
import { runs, attention, observer, sessions, archive, T0, T_END, LIVE_NOW, setSnapshot } from './data.js';
import { layout, mins, fmt, xOf, isFinished } from './layout.js';
import { nodeTypes } from './nodes.jsx';

export default function App() {
  return (
    <ReactFlowProvider>
      <Cockpit />
    </ReactFlowProvider>
  );
}

function Cockpit() {
  const [revision, setRevision] = useState(0);
  const [live, setLive] = useState(true);
  const [connection, setConnection] = useState('Connecting');
  const [now, setNow] = useState(mins(LIVE_NOW));
  const [playing, setPlaying] = useState(false);
  const [selectedId, setSelectedId] = useState(null);
  const [openGroup, setOpenGroup] = useState(null);
  const [scope, setScope] = useState(readScope);
  const [expanded, setExpanded] = useState(() => new Set());
  const flow = useReactFlow();
  const { nodes, edges, runs: shownRuns } = useMemo(() => layout(now, { ...scope, expanded }), [now, scope, expanded, revision]);
  const selected = nodes.find((n) => n.id === selectedId && n.type !== 'runCollapsed') ?? null;

  useEffect(() => {
    const params = new URLSearchParams(scope.view === 'session' ? { view: 'session', chat: scope.chat } : {});
    window.history.replaceState(null, '', `${window.location.pathname}${params.size ? `?${params}` : ''}`);
    setSelectedId(null);
    setTimeout(() => {
      // A session pane is narrow: show the live end of its run at a readable zoom, not the whole day.
      const run = scope.view === 'session' && flow.getNodes().find((n) => n.type === 'run');
      if (run) flow.setCenter(xOf(now) - 120, run.position.y + Number.parseFloat(run.style.height) / 2, { zoom: 0.62, duration: 400 });
      else flow.fitView({ padding: 0.06, duration: 400 });
    }, 80);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, flow, revision > 0]);

  useEffect(() => {
    let cancelled = false;
    let refreshTimer;
    let controller;
    async function refresh() {
      controller?.abort();
      controller = new AbortController();
      try {
        const params = new URLSearchParams(scope.view === 'session' ? { view: 'session', chat: scope.chat } : { view: 'all' });
        if (!live) params.set('at', new Date(now * 60_000).toISOString());
        const response = await fetch(`/api/state?${params}`, { signal: controller.signal });
        if (!response.ok) throw new Error('The snapshot request failed.');
        const snapshot = await response.json();
        if (cancelled) return;
        setSnapshot(snapshot, live);
        setRevision(value => value + 1);
        setConnection('Connected');
        if (live) setNow(mins(LIVE_NOW));
      } catch (error) { if (!cancelled && error.name !== 'AbortError') setConnection('Connection unavailable'); }
    }
    void refresh();
    const source = new EventSource('/api/stream');
    source.addEventListener('cockpit', () => {
      if (!live) return;
      clearTimeout(refreshTimer);
      refreshTimer = setTimeout(refresh, 150);
    });
    source.onerror = () => setConnection('Reconnecting');
    return () => { cancelled = true; controller?.abort(); source.close(); clearTimeout(refreshTimer); };
  }, [scope, live, live ? null : now]);

  const toggleRun = (runId) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(runId)) next.delete(runId); else next.add(runId);
      return next;
    });
  };
  const onNodeClick = (_, n) => {
    if (n.type === 'runCollapsed') { toggleRun(n.data.run.id); return; }
    if (n.type === 'run' && n.data.finished) { toggleRun(n.data.run.id); return; }
    if (n.selectable !== false) { setSelectedId(n.id); setOpenGroup(null); }
  };

  useEffect(() => {
    if (!playing) return undefined;
    const id = setInterval(() => setNow((m) => {
      if (m >= mins(LIVE_NOW)) { setPlaying(false); return mins(LIVE_NOW); }
      return Math.min(m + 1, mins(LIVE_NOW));
    }), 70);
    return () => clearInterval(id);
  }, [playing]);

  const focus = (nodeId, zoom) => {
    setSelectedId(nodeId);
    const node = nodes.find((n) => n.id === nodeId);
    if (node?.type === 'run' || node?.type === 'lane') {
      // A run is as wide as the day: center on "now" in that run instead of fitting all of it.
      const run = nodes.find((n) => n.id === `run:${nodeId.split(':')[1]}`);
      const y = node.type === 'run' ? run.position.y + run.style.height / 2 : node.position.y + 120;
      setTimeout(() => flow.setCenter(xOf(now) - 300, y, { zoom: zoom ?? 0.75, duration: 600 }), 30);
      return;
    }
    const w = Number.parseFloat(node.style?.width ?? 160);
    setTimeout(() => flow.setCenter(node.position.x + w / 2, node.position.y + 150, { zoom: zoom ?? 1, duration: 600 }), 30);
  };
  const play = () => {
    setLive(false);
    if (now >= mins(LIVE_NOW)) setNow(mins(T0));
    setPlaying(true);
  };

  return (
    <div className={`app ${scope.view === 'session' ? 'session' : ''}`}>
      <Rail now={now} focus={focus} scope={scope} setScope={setScope} shownRuns={shownRuns} toggleRun={toggleRun} />
      <div className="main">
        <div className="connection-status">{connection}{!runs.length && ' · No observed runs in this view.'}</div>
        <SessionBar now={now} scope={scope} setScope={setScope} shownRuns={shownRuns} focus={focus} />
        <div className="canvas">
          <ReactFlow
            nodes={nodes.map((n) => ({
              ...n,
              selected: n.id === selectedId,
              data: n.type === 'phase' ? { ...n.data, openGroup: (gi) => { setSelectedId(n.id); setOpenGroup(gi); } } : n.data,
            }))}
            edges={edges}
            nodeTypes={nodeTypes}
            nodesDraggable={false}
            nodesConnectable={false}
            elevateNodesOnSelect={false}
            onNodeClick={onNodeClick}
            onPaneClick={() => setSelectedId(null)}
            fitView
            fitViewOptions={{ padding: 0.06 }}
            minZoom={0.12}
            maxZoom={2.2}
            proOptions={{ hideAttribution: true }}
          >
            <Background gap={24} size={1.2} color="#252c3b" />
            <Controls showInteractive={false} />
          </ReactFlow>
        </div>
        <TimeBar now={now} setNow={(m) => { setPlaying(false); setLive(false); setNow(m); }} playing={playing} play={play} pause={() => setPlaying(false)} goLive={() => { setLive(true); setPlaying(false); setNow(mins(LIVE_NOW)); }} />
      </div>
      {selected && <Details node={selected} openGroup={openGroup} close={() => setSelectedId(null)} />}
    </div>
  );
}

function readScope() {
  const params = new URLSearchParams(window.location.search);
  return params.get('view') === 'session' ? { view: 'session', chat: params.get('chat') ?? sessions[0]?.chat ?? '' } : { view: 'all' };
}

function needsFor(now, shownRuns) {
  const ids = new Set(shownRuns.map((r) => r.id));
  return attention.filter((a) => mins(a.t) <= now && ids.has(a.run));
}

function Rail({ now, focus, scope, setScope, shownRuns, toggleRun }) {
  const needs = needsFor(now, shownRuns);
  const active = shownRuns.filter((r) => !isFinished(r, now));
  const finished = shownRuns.filter((r) => isFinished(r, now));
  return (
    <aside className="rail">
      <div className="rail-head">
        <div className="logo">◐ Cockpit</div>
        <span className="private">🔒 Private</span>
      </div>
      <div className="views">
        <button type="button" className={scope.view === 'all' ? 'on' : ''} onClick={() => setScope({ view: 'all' })}>All active runs</button>
        <button type="button" className={scope.view === 'session' ? 'on' : ''} onClick={() => setScope({ view: 'session', chat: sessions[0]?.chat ?? '' })}>One session</button>
      </div>
      <div className="rail-sub">rstagi · {active.length} active runs · {active.reduce((n, r) => n + r.lanes.length, 0)} agents</div>

      <section>
        <h4>Needs you <span className="count">{needs.length}</span></h4>
        {needs.map((a) => (
          <button key={a.text} type="button" className="attn" onClick={() => focus(`phase:${a.run}:${a.phase}`)}>
            <span className="attn-t">{fmt(mins(a.t))}</span>{a.text}
          </button>
        ))}
        {!needs.length && <div className="empty">Nothing right now.</div>}
      </section>

      <section>
        <h4>Active</h4>
        {active.map((run) => (
          <div key={run.id} className="rail-run">
            <button type="button" className="rail-run-title" onClick={() => focus(`run:${run.id}`, 0.8)}>
              <span className={`kind kind-${run.kind.toLowerCase()}`}>{run.kind}</span>{run.title}
            </button>
            {run.lanes.map((lane) => {
              const s = laneStatus(lane, now);
              return (
                <button type="button" key={lane.id} className="rail-agent" onClick={() => focus(`lane:${run.id}:${lane.id}`, 0.9)}>
                  <span className={`pulse pulse-${s.state}`} />
                  <span className="ra-name">{lane.agent}</span>
                  <span className="ra-line">{s.line}</span>
                </button>
              );
            })}
          </div>
        ))}
      </section>

      {finished.length > 0 && (
        <section>
          <h4>Finished today</h4>
          {finished.map((run) => (
            <button type="button" key={run.id} className="rail-finished" onClick={() => toggleRun(run.id)}>
              <span className="ok">✓</span>{run.title}<span className="res">{fmt(mins(run.finishedAt))}</span>
            </button>
          ))}
        </section>
      )}

      <section>
        <h4>Earlier</h4>
        {archive.map((a) => <div key={a.title} className="archive-row"><span className="when">{a.when}</span>{a.title}</div>)}
      </section>

      <section className="observer">
        <h4>Intent observer</h4>
        <div className="obs-name"><span className="pulse pulse-active" />{observer.daemon}</div>
        <div className="obs-text">Last pass {observer.lastPass} · {observer.engine}</div>
        <div className="obs-text">{observer.text}</div>
      </section>
    </aside>
  );
}

/** The header of the session view: which chat, what needs you, and a way back to all runs. */
function SessionBar({ now, scope, setScope, shownRuns, focus }) {
  if (scope.view !== 'session') return null;
  const needs = needsFor(now, shownRuns);
  return (
    <div className="session-bar">
      <div className="sb-top">
        <span className="logo">◐</span>
        <select className="session-pick" value={scope.chat} onChange={(e) => setScope({ view: 'session', chat: e.target.value })}>
          {sessions.map((s) => <option key={s.chat} value={s.chat}>{s.label}</option>)}
        </select>
        <button type="button" onClick={() => setScope({ view: 'all' })}>All runs ↗</button>
      </div>
      {needs.map((a) => (
        <button key={a.text} type="button" className="attn" onClick={() => focus(`phase:${a.run}:${a.phase}`)}>
          <span className="attn-t">{fmt(mins(a.t))}</span>{a.text}
        </button>
      ))}
    </div>
  );
}

function laneStatus(lane, now) {
  if (lane.role === 'coordinator') return { state: 'active', line: 'Recorded checkpoints' };
  const live = (lane.phases ?? []).find((p) => p.start && mins(p.start) <= now && (!p.end || mins(p.end) > now));
  if (!live) return { state: 'idle', line: 'idle' };
  if (live.ask && mins(live.ask.t) <= now) return { state: 'hil', line: 'waiting for you' };
  return { state: 'active', line: `${live.key} · ${(live.intents ?? []).filter((i) => mins(i.t) <= now).at(-1)?.text ?? 'starting'}` };
}

function TimeBar({ now, setNow, playing, play, pause, goLive }) {
  const from = mins(T0);
  const to = mins(T_END);
  const live = mins(LIVE_NOW);
  const cps = runs.flatMap((r) => r.lanes.flatMap((l) => l.checkpoints ?? []));
  const pct = (m) => `${((m - from) / (to - from)) * 100}%`;
  return (
    <div className="timebar">
      <button type="button" className="tb-btn" onClick={playing ? pause : play}>{playing ? '❚❚' : '▶'} Replay</button>
      <div className="tb-track">
        <div className="tb-future" style={{ left: pct(live), right: 0 }} />
        {cps.map((c) => <span key={c.n} className="tb-cp" style={{ left: pct(mins(c.t)) }} title={`${c.n} ${c.label}`}>{c.n}</span>)}
        <input type="range" min={from} max={live} value={now} onChange={(e) => setNow(Number(e.target.value))} style={{ width: pct(live) }} />
      </div>
      <span className="tb-time">{fmt(now)}</span>
      <button type="button" className={`tb-live ${now === live ? 'is-live' : ''}`} onClick={goLive}>● Live</button>
    </div>
  );
}

function Details({ node, openGroup, close }) {
  return (
    <aside className="details">
      <button type="button" className="close" onClick={close}>✕</button>
      {node.type === 'phase' && <PhaseDetails d={node.data} openGroup={openGroup} />}
      {node.type === 'media' && (
        <>
          <div className="d-kicker">{node.data.run.title} · {node.data.phase.key}</div>
          <h3>{node.data.media.name}</h3>
          <img className="d-big" src={node.data.media.src} alt="" />
          <div className="d-muted">Saved {fmt(mins(node.data.media.t))}</div>
        </>
      )}
      {node.type === 'checkpoint' && (
        <>
          <div className="d-kicker">{node.data.run.title}</div>
          <h3>{node.data.cp.n} · {node.data.cp.label}</h3>
          <div className="d-muted">{node.data.cp.t}</div>
          <p>{node.data.cp.detail}</p>
          {node.data.cp.image && <img className="d-big" src={node.data.cp.image} alt="" />}
        </>
      )}
      {node.type === 'run' && (
        <>
          <div className="d-kicker">{node.data.run.kind} · {node.data.run.task}</div>
          <h3>{node.data.run.title}</h3>
          <div className="d-muted">{node.data.run.sub}</div>
          <ul className="d-list">{Object.entries(node.data.summary).filter(([, v]) => v).map(([k, v]) => <li key={k}>{v} {k}</li>)}</ul>
        </>
      )}
      {node.type === 'lane' && (
        <>
          <div className="d-kicker">{node.data.run.title}</div>
          <h3>{node.data.lane.agent}</h3>
          <div className="d-muted">{node.data.lane.engine}{node.data.lane.chat ? ` · ${node.data.lane.chat}` : ''}</div>
          <p>{node.data.current ? `Now: ${node.data.current.phase.key} · ${node.data.current.phase.title}` : 'No phase running.'}</p>
        </>
      )}
    </aside>
  );
}

function PhaseDetails({ d, openGroup }) {
  const { phase, run, lane, status, minutes, drift, media, ask, groups, live } = d;
  return (
    <>
      <div className="d-kicker">{run.title} · {lane.agent}</div>
      <h3>{phase.key === phase.title ? phase.title : `${phase.key} · ${phase.title}`}</h3>
      <div className="d-row">
        {phase.task && <span className="taskchip">{phase.task}</span>}
        <span className={`pill st-${status}`}>{{ done: 'done', drift: 'done · drift', active: 'working', hil: 'needs you', todo: 'planned' }[status]}</span>
        {minutes != null && <span className="d-muted">{minutes} min{phase.est ? ` · plan ${phase.est[0]}–${phase.est[1]}` : ''}</span>}
      </div>
      {ask && <div className="ask big">❓ {ask.text}</div>}
      {phase.doneWhen && <><h5>Done when</h5><p>{phase.doneWhen}</p></>}
      {drift.length > 0 && <><h5>Drift from the plan</h5><ul className="d-list drift">{drift.map((x) => <li key={x.kind}><b>{x.kind}</b> · {x.detail}</li>)}</ul></>}
      {(groups.length > 0 || live.length > 0) && (
        <>
          <h5>Intents <span className="d-muted">inferred by the observer</span></h5>
          <div className="levels">Groups merge older intents. Open one to see its intents and the tool calls behind them.</div>
          {live.length > 0 && (
            <details className="d-group live" open>
              <summary><span className="dg-text">Now</span><span className="dg-meta">{live.length} live intents</span></summary>
              {live.map((it) => <IntentDetail key={it.index} it={it} />)}
            </details>
          )}
          {[...groups].reverse().map((g) => (
            <details key={`${phase.key}-${g.gi}-${openGroup}`} className="d-group" open={openGroup === g.gi}>
              <summary><span className="dg-text">{g.text}</span><span className="dg-meta">{g.members.length} {g.members.length === 1 ? 'intent' : 'intents'} · {g.from}{g.to !== g.from ? `–${g.to}` : ''}</span></summary>
              {g.members.map((it) => <IntentDetail key={it.index} it={it} />)}
            </details>
          ))}
        </>
      )}
      {phase.files?.length > 0 && <><h5>Files</h5><ul className="d-list mono">{phase.files.map((f) => <li key={f}>{f}</li>)}</ul></>}
      {media.length > 0 && <><h5>Proof</h5><div className="d-media">{media.map((m) => <figure key={m.name}><img src={m.src} alt="" /><figcaption>{m.name}</figcaption></figure>)}</div></>}
    </>
  );
}

function IntentDetail({ it }) {
  return (
    <div className="d-intent">
      <div className="di-text"><span className="di-t">{fmt(mins(it.t))}</span>{it.text}{it.conf ? <span className="d-muted"> · {Math.round(it.conf * 100)}% sure</span> : null}</div>
      {it.actions.map((x) => <div key={x.t + x.text} className="il-action"><span className={`tool tool-${x.tool.toLowerCase()}`}>{x.tool}</span>{x.text}</div>)}
    </div>
  );
}
