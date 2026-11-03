// Adapt the event-store snapshot to the original canvas components.
export let runs = [];
export let attention = [];
export let observer = { daemon: 'cockpit-observer', lastPass: 'waiting', engine: 'Haiku / Sonnet', text: 'Reads local session and Loop files every 30 s.' };
export let sessions = [];
export let archive = [];
const started = Date.now();
export let T0 = new Date(started - 60_000).toISOString();
export let T_END = new Date(started + 30 * 60_000).toISOString();
export let LIVE_NOW = new Date(started).toISOString();

export function setSnapshot(snapshot, live = true) {
  if (live) LIVE_NOW = snapshot.timestamp;
  const eventTime = (run, type, id) => run.eventTimes?.[type]?.[id] ?? run.updatedAt;
  const time = (value, fallback) => value ?? fallback;
  const raw = snapshot.runs ?? [];
  const midnight = new Date(LIVE_NOW); midnight.setHours(0, 0, 0, 0);
  const earlier = raw.filter(run => run.meta.status !== 'active' && Date.parse(run.updatedAt) < midnight.getTime());
  archive = earlier.map(run => ({ title: run.meta.title ?? run.id, when: new Date(run.updatedAt).toLocaleDateString() }));
  runs = raw.filter(run => !earlier.includes(run)).map(run => {
    const phases = run.phases.length ? run.phases : [{ id: 'session', title: run.meta.title ?? 'Session activity', startedAt: run.createdAt }];
    const laneIds = [...new Set([...(run.meta.lanes ?? []).map(lane => lane.id), ...phases.map(phase => phase.laneId ?? 'session')])];
    const chat = run.meta.chatId ?? run.meta.sessionId ?? run.meta.sessions?.[0]?.chatId ?? run.meta.sessions?.[0]?.id;
    const lanes = laneIds.map(id => ({
      id, role: 'agent', agent: run.meta.lanes?.find(lane => lane.id === id)?.title ?? run.meta.source ?? 'Agent',
      engine: run.meta.source ?? 'Session', chat, chats: [chat, ...(run.meta.sessions ?? []).flatMap(session => [session.chatId, session.id])].filter(Boolean),
      phases: phases.filter(phase => (phase.laneId ?? 'session') === id).map(phase => {
        const intents = run.intents.filter(intent => intent.phaseId === phase.id || (!intent.phaseId && phases.length === 1));
        const mapped = intents.map(intent => ({ id: intent.id, status: intent.status,
          t: time(run.actions.find(action => intent.actionIds.includes(action.id))?.timestamp, phase.startedAt ?? run.createdAt),
          text: intent.text, reason: intent.reason,
          actions: run.actions.filter(action => intent.actionIds.includes(action.id)).map(action => ({ t: action.timestamp ?? run.updatedAt, tool: action.tool, text: action.summary })),
        }));
        const asks = run.asks.filter(ask => !ask.resolved && (!ask.phaseId || ask.phaseId === phase.id));
        return { key: phase.id, title: phase.title, inferred: phase.inferred,
          start: phase.startedAt ?? run.createdAt, end: phase.endedAt, intents: mapped,
          groups: run.groups.filter(group => group.phaseId === phase.id).map(group => ({ text: group.text, members: mapped.flatMap((intent, index) => group.intentIds.includes(intent.id) ? [index] : []) })),
          drift: run.drifts.filter(drift => drift.phaseId === phase.id && drift.summary !== 'No drift from the plan.').map(drift => ({ t: eventTime(run, 'drift', drift.id), kind: 'drift', detail: drift.summary })),
          ask: asks[0] ? { t: eventTime(run, 'ask', asks[0].id), text: asks[0].text } : null,
          media: run.media.filter(media => media.phaseId === phase.id || (!media.phaseId && phases.length === 1)).map(media => ({ t: eventTime(run, 'media', media.id), name: media.title ?? media.id, src: `/media/${encodeURIComponent(media.id)}`, gif: media.mimeType === 'image/gif' })),
        };
      }),
    }));
    if (run.checkpoints.length) lanes.unshift({ id: 'observer', agent: 'Checkpoints', role: 'coordinator', engine: 'cockpit-observer', chat,
      checkpoints: run.checkpoints.map((cp, index) => ({ n: `CP${index + 1}`, t: eventTime(run, 'checkpoint', cp.id), label: cp.summary, detail: cp.summary })) });
    return { id: run.id, title: run.meta.title ?? run.id, kind: run.meta.source === 'loop' ? 'Loop' : 'Session',
      sub: run.meta.branch ?? run.meta.cwd ?? '', task: '', result: 'finished', lanes,
      startedAt: run.createdAt, finishedAt: run.meta.status === 'done' ? run.updatedAt : null };
  });
  attention = raw.flatMap(run => run.asks.filter(ask => !ask.resolved).map(ask => ({ run: run.id, phase: ask.phaseId ?? run.phases.at(-1)?.id, t: eventTime(run, 'ask', ask.id), text: ask.text })));
  sessions = [...new Map((snapshot.sessions ?? []).map(session => {
    const chat = session.chatId ?? session.id;
    return [chat, { chat, label: session.title ?? chat }];
  })).values()];
  if (!sessions.length) sessions = [{ chat: '', label: 'No sessions yet' }];
  if (live) {
    T0 = new Date(Math.min(Date.parse(LIVE_NOW) - 60_000, ...runs.map(run => Date.parse(run.startedAt)))).toISOString();
    T_END = new Date(Date.parse(LIVE_NOW) + 30 * 60_000).toISOString();
  }
  observer = { ...observer, lastPass: snapshot.observer.lastPass ? new Date(snapshot.observer.lastPass).toLocaleTimeString() : 'waiting',
    text: snapshot.observer.lastError ?? 'Reads local session and Loop files every 30 s.' };
}
