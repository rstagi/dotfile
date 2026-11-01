export type RunStatus = "active" | "idle" | "done";
export type PhaseStatus = "running" | "done" | "failed";

export interface Action {
  id: string;
  tool: string;
  summary: string;
  input?: unknown;
  timestamp?: string;
  sessionId?: string;
  laneId?: string;
  phaseId?: string;
}

export interface Intent {
  id: string;
  text: string;
  reason?: string;
  actionIds: string[];
  phaseId?: string;
  laneId?: string;
  status?: "live" | "earlier";
}

export interface IntentGroup {
  id: string;
  text: string;
  intentIds: string[];
  phaseId?: string;
  laneId?: string;
}

export interface Phase {
  id: string;
  title: string;
  laneId?: string;
  inferred?: boolean;
  status?: PhaseStatus;
  startedAt?: string;
  endedAt?: string;
}

export interface Lane {
  id: string;
  title: string;
  phaseIds?: string[];
}

export interface Plan {
  id: string;
  title: string;
  content?: string;
}

export interface Milestone {
  id: string;
  title: string;
  phaseIds?: string[];
}

export interface RunMeta {
  title?: string;
  status?: RunStatus;
  source?: string;
  cwd?: string;
  branch?: string;
  sessionId?: string;
  chatId?: string;
  isSidechain?: boolean;
  lanes?: Lane[];
  plan?: Plan;
  milestones?: Milestone[];
}

export interface Drift {
  id: string;
  summary: string;
  phaseId?: string;
  estimateMinutes?: number;
  elapsedMinutes?: number;
  diff?: string;
  verification?: string;
}

export interface Ask {
  id: string;
  text: string;
  resolved?: boolean;
  phaseId?: string;
  sessionId?: string;
}

export interface Media {
  id: string;
  path: string;
  mimeType: string;
  title?: string;
  phaseId?: string;
}

export interface Checkpoint {
  id: string;
  summary: string;
  phaseId?: string;
  mediaIds?: string[];
}

export interface EventBase {
  id: string;
  runId: string;
  timestamp: string;
  laneId?: string;
  phaseId?: string;
  sessionId?: string;
}

export type CockpitEvent = EventBase & (
  | { type: "action"; action: Action }
  | { type: "intent.upsert"; intent: Intent }
  | { type: "group.upsert"; group: IntentGroup }
  | { type: "phase.start"; phase: Phase }
  | { type: "phase.end"; phase: Pick<Phase, "id"> & Partial<Phase> }
  | { type: "drift"; drift: Drift }
  | { type: "ask"; ask: Ask }
  | { type: "media"; media: Media }
  | { type: "checkpoint"; checkpoint: Checkpoint }
  | { type: "run.meta"; meta: RunMeta }
);

export interface RunState {
  id: string;
  createdAt: string;
  updatedAt: string;
  meta: RunMeta;
  actions: Action[];
  intents: Intent[];
  groups: IntentGroup[];
  phases: Phase[];
  drifts: Drift[];
  asks: Ask[];
  media: Media[];
  checkpoints: Checkpoint[];
}
