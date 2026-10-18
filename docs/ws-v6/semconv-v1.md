# WS semantic conventions v1

This document defines the WS telemetry contract from [architecture §7 and §12](architecture.md#7-telemetry).
It makes the inherited session, message and edge mappings explicit.
The Rust constants live in `wsd/crates/ws-core/src/semconv.rs`.
A source event means a durable session log entry unless the table identifies agent telemetry or collector metadata.
A requirement applies when the producer emits the specified signal.
This catalog does not require an implementation of future hub operations in the local milestone.

## Version and identity rules

- Changes within v1 must be additive. Do not rename a name or change its type, unit, requirement or meaning.
- A breaking change requires a new major convention version and an explicit migration.
- Every WS event carries `ws.semconv.version=1` and `event_id`.
- Log records and span events for one source entry retain the same source `event_id`.
- Retries and replay retain that identity. A sink deduplicates by `event_id` within each signal.
- When an agent record lacks an identity, assign an `event_id` once at durable ingestion. Retain it for replay.
- Native agent names and payload fields remain producer data. They do not become WS names through this catalog.
- Imported agent signals retain their producer schema. Normalized WS events use this version and identity contract.

## Trace and span rules

Decode the session ULID into its 16 bytes to obtain the canonical trace id.
A session retains that trace id after a move or resume.
Each segment has a stable span id. Replay retains that id.
A segment covers one turn or one phase attempt. No session root span remains open between turns.
Tool spans and model-call spans are children of their matching turn in the same session trace.
Parent sessions, dependencies and Loop relations use span links between traces.
Never use a parent span from another session.
Link successive segments and the known root segments of parent and `depends_on` sessions.

Match tools by tool-use id. Match model records by session id and the turn interval.
A blocked Stop does not close the turn.
Interruption, process exit and abandonment close open turns and tools with an error status.
A missing tool post event closes the tool at turn closure.
Use the hook result for tool status. Codex's native `success` field alone does not prove command success.
Claude model intervals use `duration_ms` and the API record timestamp.
Codex completion records with zero output tokens are prewarm and do not create model spans.
Without native timing, a Codex model span uses surrounding hooks and carries `ws.timing=estimated`.
Native spans can supply measured timing. They must not duplicate canonical turn, tool or model spans.
Retain available model, request id, input/output tokens and cache tokens from agent records as producer attributes.
Do not invent missing request ids, durations or token fields.

## Resource attributes

Apply these attributes to all WS signals. Omit an absent parent or project. A session can have no repository or workspace (D18).

| Name | Type | Requirement | Source log event or value |
|---|---|---|---|
| `ws.session.id` | string (ULID) | Required | `session.created` envelope `session_id` |
| `ws.parent_session.id` | string (ULID) | Optional, when a parent exists | `session.created` body `parent_session_id` or a `parent` edge |
| `ws.device.id` | string (ULID) | Required | Source envelope `device_id`, including after a move |
| `ws.project` | string | Optional, when a project exists | `session.created` project or associated workspace metadata |

## Common attributes and values

These attributes describe events or spans unless a row specifies a metric point. `1` and `ws.session` are constant values.

| Name | Type | Requirement | Source log event or value |
|---|---|---|---|
| `ws.semconv.version` | string | Required on every WS event, span and metric point | Exporter convention version, independent of the source event |
| `1` | string version value | Required value of `ws.semconv.version` | This v1 contract |
| `event_id` | string (ULID) | Required on every event | Source envelope `event_id` |
| `event.domain` | string | Required on WS log records and span events | Constant domain value for this contract |
| `ws.session` | string domain value | Required value of `event.domain` | WS session exporter, including phase and quota events |
| `event.name` | string | Required on WS log records | Mapped event name from the event table |
| `ws.session.state` | string | Required on state transition records | State suffix of `session.*`, or `finished` for `done` |
| `ws.session.name` | string | Optional | `session.created` or `session.renamed` body `name` |
| `ws.session.kind` | string | Optional | `session.created` body kind: `interactive`, `headless` or `loop` |
| `ws.session.epoch` | int64 | Required on durable session events | Source envelope `epoch` |
| `ws.session.seq` | int64 | Required on durable session events | Source envelope `seq` |
| `ws.session.message.id` | string (ULID) | Required on message records | `message.sent` event id, or `message.delivered/read` body `message_id` |
| `ws.session.message.from` | string (ULID) | Required on sent records, optional otherwise | `message.sent` body `from_session` or sender envelope |
| `ws.session.message.to` | string (ULID) | Required on sent records, optional otherwise | `message.sent` body `to_session` or legacy `recipient` |
| `ws.session.message.pair_seq` | int64 | Required on ordered sent records, optional for legacy imports | `message.sent` body `pair_seq` |
| `ws.session.edge.id` | string (ULID) | Required on edge records | `edge.added` event id, or `edge.removed` body `edge_id` |
| `ws.session.edge.kind` | string | Required on edge additions, optional on removals | `edge.added` body `kind`, or `session.depends_on/block_on` type |
| `ws.session.edge.target` | string (ULID) | Required on edge additions, optional on removals | Edge body `target` |
| `ws.session.trigger.id` | string (ULID) | Required on trigger records | `edge.action_*` body `trigger_id` |
| `ws.session.turn.id` | string | Required on turn spans | `turn.started` prompt id, or its event id when absent |
| `ws.session.tool.use_id` | string | Required on tool spans | `tool.pre/post` callback tool-use id |
| `ws.session.tool.name` | string | Optional | `tool.pre/post` callback tool name |
| `ws.timing` | string enum: `measured`, `estimated` | Required on synthesized model spans | Agent duration or native span, otherwise the surrounding hook interval |
| `ws.phase.id` | string | Required on phase records and attempt spans | `phase.started/finished` phase identifier, or `phase.spawned` body phase |
| `ws.phase.attempt` | int64 | Required on phase records and attempt spans | Phase attempt number, retained across replay |
| `ws.session.reason` | string | Required on claim rejection, optional on other failures | Source event body `reason` |

## Spans

| Name | Type | Requirement | Source log event or value |
|---|---|---|---|
| `ws.session.turn` | span (INTERNAL) | Required for each observed turn start | `turn.started` to accepted `turn.ended`, `turn.cancelled`, exit or abandonment |
| `ws.session.tool` | span (INTERNAL) | Required for each observed tool start | `tool.pre` to matching `tool.post`, or turn closure |
| `ws.session.model_call` | span (CLIENT) | Optional, when model-call evidence exists | `claude_code.api_request`, or non-prewarm `codex.sse_event` response completion |
| `ws.session.phase_attempt` | span (INTERNAL) | Required for each phase attempt | `phase.started` to `phase.finished`, normalized from `phase.spawned` and `process.exited` |

## Log records and span events

Emit one OTLP log record for each source event below.
Set its `event.name` and body event name to the mapped name.
Use the source timestamp for the log timestamp. Preserve the source body subject to the content policy.
Set `event.domain=ws.session`, including for phase and quota records.
Also attach the same event to the matching open span, when one exists.
That span event is optional when no span is open. Never create a span solely to contain an event.
Emit a closure event before the span closes.
Raw `hook.received` entries are capture evidence. Their normalized events below supply telemetry semantics.
`api.called`, workspace updates and process adoption are internal records outside this catalog.
Map a phase process exit to `phase.finished`. Map other process exits to `ws.session.exit`.
Do not emit two log records for the same source entry.
Aliases in the source column describe compatible input forms, not additional emissions for one entry.
Future lease and resume events appear here so their names remain stable.

| Name | Type | Requirement | Source log event or value |
|---|---|---|---|
| `ws.session.created` | log record + span event | Required when the source exists | `session.created` |
| `ws.session.running` | log record + span event | Required when the source exists | `session.running` |
| `ws.session.blocked` | log record + span event | Required when the source exists | `session.blocked` |
| `ws.session.finished` | log record + span event | Required when the source exists | `session.finished`, or legacy `done` |
| `ws.session.failed` | log record + span event | Required when the source exists | `session.failed` |
| `ws.session.reopened` | log record + span event | Required when the source exists | `session.reopened` |
| `ws.session.renamed` | log record + span event | Required when the source exists | `session.renamed` |
| `ws.session.orphaned` | log record + span event | Required when the source exists | `session.orphaned` after a stale-epoch append |
| `ws.session.resumed` | log record + span event | Required when the source exists | `session.resumed` after checkpoint resume |
| `ws.session.failed_over` | log record + span event | Required when the source exists | `session.failed_over` after lease transfer |
| `ws.session.claim_rejected` | log record + span event | Required when the source exists | `session.claim_rejected` from the resume gate |
| `ws.session.message.sent` | log record + span event | Required when the source exists | `message.sent` in the sender log |
| `ws.session.message.delivered` | log record + span event | Required when the source exists | `message.delivered` in the recipient log |
| `ws.session.message.read` | log record + span event | Required when the source exists | `message.read` in the recipient log |
| `ws.session.edge.added` | log record + span event | Required when the source exists | `edge.added`, `session.depends_on` or `session.block_on` |
| `ws.session.edge.removed` | log record + span event | Required when the source exists | `edge.removed` |
| `ws.session.trigger.registered` | log record + span event | Required when the source exists | `edge.action_registered` |
| `ws.session.trigger.removed` | log record + span event | Required when the source exists | `edge.action_removed` |
| `ws.session.trigger.fired` | log record + span event | Required when the source exists | `edge.action_fired` |
| `ws.session.trigger.failed` | log record + span event | Required when the source exists | `edge.action_failed` |
| `ws.session.turn.started` | log record + span event | Required when the source exists | `turn.started` from prompt submission |
| `ws.session.turn.ended` | log record + span event | Required when the source exists | `turn.ended` from an accepted Stop |
| `ws.session.turn.continued` | log record + span event | Required when the source exists | `turn.continued` from a blocked Stop |
| `ws.session.turn.cancelled` | log record + span event | Required when the source exists | `turn.cancelled` from interruption |
| `ws.session.tool.pre` | log record + span event | Required when the source exists | `tool.pre` from PreToolUse |
| `ws.session.tool.post` | log record + span event | Required when the source exists | `tool.post` from PostToolUse or PostToolUseFailure |
| `ws.session.agent.waiting` | log record + span event | Required when the source exists | `agent.waiting` from permission or notification hooks |
| `ws.session.subagent.started` | log record + span event | Required when the source exists | `subagent.started` |
| `ws.session.subagent.stopped` | log record + span event | Required when the source exists | `subagent.stopped` |
| `ws.session.exit` | log record + span event | Required when the source exists | `exit` hook or non-phase supervised `process.exited` |
| `phase.started` | log record + span event | Required when the source exists | `phase.started`, or legacy `phase.spawned` on a phase runner |
| `phase.finished` | log record + span event | Required when the source exists | `phase.finished`, or `process.exited` on a phase runner |
| `ws.runner.usage.updated` | log record + span event | Required for each collected reading | `runner.usage.updated` from the quota collector or token estimator |

## Runner usage attributes

The quota collector writes a `runner.usage.updated` source event for each account, bucket and window reading.
Its body holds the normalized fields below and any known metric values.
Account identity comes from the binding at session start. A bucket or plan type does not identify an account.
Keep unknown windows and missing values explicit in the reading. Do not emit a numeric point for an unknown value.
Do not use context-window usage as subscription quota.

| Name | Type | Requirement | Source log event or value |
|---|---|---|---|
| `ws.runner` | string enum: `claude`, `codex`, `grok` | Required on quota readings and metric points | Collector runner identity |
| `ws.runner.account.id` | string (opaque) | Required on quota readings and metric points | Account binding at session start from S3, never an email or credential |
| `ws.runner.usage.bucket` | string | Optional, when the provider exposes a bucket | `runner.usage.updated` provider limit id or model bucket |
| `ws.runner.usage.window` | string | Required on quota readings and metric points | `runner.usage.updated` window identity, independent of primary/secondary position |
| `ws.runner.usage.window.duration` | int64 (s) | Optional, when the window duration is known | `runner.usage.updated` normalized duration, including 18000 and 604800 |
| `ws.runner.usage.window.start` | int64 (Unix ms) | Optional, when the window start is known | `runner.usage.updated` provider period start |
| `ws.runner.usage.window.reset` | int64 (Unix ms) | Optional, when the reset time is known | `runner.usage.updated` normalized provider reset or period end |
| `ws.runner.usage.source` | string enum: `reported`, `estimated` | Required on every quota value and metric point | `runner.usage.updated` reading provenance |
| `ws.runner.usage.source.name` | string | Required on every quota reading | Collector interface, for example app-server, rollout, status-line or token telemetry |
| `ws.runner.usage.observed_at` | int64 (Unix ms) | Required on every quota reading | Collector observation time, not export time |
| `ws.runner.usage.coverage` | string | Required on estimates, optional on reported readings | Estimator scope and known gaps, including unobserved devices or sessions |
| `ws.runner.usage.model` | string | Optional, when usage belongs to a model | Provider bucket or deduplicated token telemetry model |

## Metrics

Metric points carry the resource attributes, version, runner, account, window and provenance.
Include the bucket when it exists. Use the observation time for the point timestamp.
These gauges describe a window snapshot. Do not add repeated snapshots or account-wide percentages across sessions or devices.
The hub retains the latest reported reading for each account, bucket and window.
After reset, treat the old reading as expired. A fresh reading must identify the next window.
An estimate must retain its coverage and must not silently replace a reported reading.

Normalize percentages to ratios once. Token categories must not overlap.
Codex cached input is part of input. Claude input categories are separate.
Grok reasoning is part of output, and its total already includes cached input.
Deduplicate model records before summing tokens. Use cumulative deltas rather than repeated cumulative totals.
Without a known denominator, emit tokens and coverage only. Omit both ratio gauges.
Sections 7 and 12 define no additional WS counter or histogram.
Agent metrics retain their original names and units.

| Name | Type | Requirement | Source log event or value |
|---|---|---|---|
| `ws.runner.usage.used_ratio` | gauge (double, unit 1, range 0..1) | Optional, when a used ratio is known | `runner.usage.updated` provider ratio, or calibrated estimate |
| `ws.runner.usage.remaining_ratio` | gauge (double, unit 1, range 0..1) | Optional, when a remaining ratio is known | `runner.usage.updated` reported ratio, or one minus a known used ratio |
| `ws.runner.usage.tokens` | gauge (int64, unit {token}) | Optional, when a token total is known | `runner.usage.updated` deduplicated token total for the account and window |

## Evidence and implementation checks

[S2](spikes/s2-agent-otel.md) records agent export formats and hook timing limits.
[S3](spikes/s3-runner-quota.md) records quota sources, units and missing values.
The captured inputs live in [S2 fixtures](../../wsd/crates/ws-daemon/tests/fixtures/spikes/s2/)
and [S3 fixtures](../../wsd/crates/ws-daemon/tests/fixtures/spikes/s3/).
The reports remain decision records. Their samples are test inputs, not complete production telemetry.

Run `cargo test -p ws-core semconv::tests` from `wsd/` to check every constant against this catalog.
The test reads each public string constant. A new constant must have a table row with its type, requirement and source.
