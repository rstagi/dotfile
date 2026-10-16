# S2 — Agent OTel export: Claude Code

## Question

Which signals can Claude Code export to a local OTLP receiver?
Do custom resource attributes and an environment `TRACEPARENT` survive?
Can hooks identify turn and tool boundaries?

## Method

Test date: 2026-10-16. Agent: Claude Code **2.1.302**.
The `haiku` alias resolved to `claude-haiku-5-5`.
The host ran macOS with arm64 hardware and Python 3.15.0.

Agent-home mode: **sandbox**. Each probe used the same temporary `HOME`,
`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, and `XDG_CONFIG_HOME`.
The sandbox received copies of the live onboarding and OAuth account metadata.
Authentication used the inherited `ANTHROPIC_API_KEY`. No credential value appears in these artifacts.
The probes ran in an empty temporary project, with new sessions.
They did not use the live agent settings or edit existing transcripts.

Eight synthetic `claude -p` probes ran sequentially.
Two additional CLI calls read `--version` and `--help`.
The total was **10 CLI invocations**, with eight inference runs.
Every inference run returned `S2_DONE`, made two assistant/model turns, and executed exactly one Bash tool call.
The command printed a synthetic marker and its `TRACEPARENT` environment variable.

The [capture sink](s2/capture-sink.py) uses only Python's standard library.
It binds to `127.0.0.1` and saves each POST entity body without decoding it.
It writes separate metadata with the path, encoding, length, timestamp, and SHA-256.
It accepts Content-Length and chunked requests, including binary protobuf and compressed bodies.
It preserves duplicates and excludes authorization headers from metadata.
Its HTTP response is `{}` for JSON and an empty protobuf message for protobuf.
The sink does not implement gRPC or decode protobuf.

The probes used these common settings:

```sh
CLAUDE_CODE_ENABLE_TELEMETRY=1
OTEL_METRICS_EXPORTER=otlp
OTEL_LOGS_EXPORTER=otlp
OTEL_TRACES_EXPORTER=otlp
OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:<port>
OTEL_EXPORTER_OTLP_PROTOCOL=http/json
OTEL_METRIC_EXPORT_INTERVAL=1000
OTEL_LOGS_EXPORT_INTERVAL=1000
OTEL_TRACES_EXPORT_INTERVAL=1000
OTEL_RESOURCE_ATTRIBUTES=ws.session.id=s2-synthetic-session,ws.parent_session.id=s2-synthetic-parent,ws.device.id=s2-synthetic-device,ws.project=s2-synthetic-project
TRACEPARENT=00-11111111111111111111111111111111-2222222222222222-01
```

The beta runs added `CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1`.
The detailed runs also added `ENABLE_BETA_TRACING_DETAILED=1` and
`BETA_TRACING_ENDPOINT=http://127.0.0.1:<port>`.
Beta and detailed runs enabled `OTEL_LOG_USER_PROMPTS`, `OTEL_LOG_TOOL_DETAILS`,
and `OTEL_LOG_TOOL_CONTENT`, each with value `1`.
The [manifest](s2/samples/manifest.json) records every run's settings and request counts.

A command hook recorded stdin JSON and receipt time for each of these events:
`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`,
`PostToolUseFailure`, `Stop`, and `SessionEnd`.
The recorder returned no output and did not change tool decisions.
The CLI used `--tools Bash --allowedTools Bash`, an empty strict MCP configuration,
`--setting-sources user`, and sandbox hook settings through `--settings`.
It used permission bypass for the synthetic command.

The sink stopped two seconds after each CLI process exited.
A decoder in a temporary virtual environment inspected protobuf bodies.
The sink and its tests do not need that decoder.

The official [monitoring reference](https://code.claude.com/docs/en/monitoring-usage)
and [hook reference](https://code.claude.com/docs/en/hooks) supplied the configuration names.
No Context7 tool was available in this runner.
The findings below describe captured requests, including differences from the tracing documentation.

## Findings

### Signals and switches

Counts below are HTTP requests, not individual records.
`M`, `L`, and `T` mean metrics, logs, and traces.
All exporter selectors were `otlp` unless the row states otherwise.

| Probe | Protocol setting | Additional settings | M / L / T |
|---|---|---|---|
| telemetry-off | JSON | `CLAUDE_CODE_ENABLE_TELEMETRY=0` | 0 / 0 / 0 |
| logs-only | JSON | metrics and traces selectors `none` | 0 / 2 / 0 |
| metrics-only | JSON | logs and traces selectors `none` | 2 / 0 / 0 |
| baseline | JSON | no beta switch | 3 / 2 / 0 |
| beta-json | JSON | enhanced beta | 3 / 2 / 1 |
| beta-protobuf | protobuf | enhanced beta | 2 / 2 / 1 |
| detailed-json | JSON | enhanced beta plus detailed pair | 3 / 1 / 1 |
| detailed-protobuf | protobuf | enhanced beta plus detailed pair | 3 / 1 / 1 |

The routes were `/v1/metrics`, `/v1/logs`, and `/v1/traces`.
The ordinary exporters honored `http/json` and `http/protobuf`.
They sent `application/json` and `application/x-protobuf`, respectively.
**The detailed pair sent JSON logs and traces in both detailed runs.**
Only metrics used protobuf in `detailed-protobuf`.
Thus the receiver must accept mixed encodings, even under a general protobuf setting.
Per-signal endpoint overrides and gRPC were not tested.

The observed metrics were `claude_code.session.count`, `claude_code.cost.usage`,
`claude_code.token.usage`, and `claude_code.active_time.total`.
They supplied counters and data-point attributes, rather than execution boundaries.

### Resource attributes and trace context

All four synthetic `ws.*` values survived on every captured signal's resource.
They also appeared as attributes on log records and spans.
The built-in resource included `service.name=claude-code` and `service.version=2.1.302`.

`TRACEPARENT` support depends on the tracing mode:

| Mode | Observed behavior |
|---|---|
| Baseline | All 27 log records used the supplied trace and parent span IDs. No spans arrived. Bash inherited the supplied value. |
| Enhanced beta alone | Model and tool spans arrived, but no interaction span arrived. The JSON probe used three separate traces. None used the supplied trace ID. Most logs retained the supplied trace, while the tool-result record used the tool trace. Bash received the tool execution context. |
| Detailed pair | All 10 spans and all 32 log records used the supplied trace ID. The interaction span had parent `2222222222222222` and `parent.source=env`. Bash received the tool execution span's context within that trace. |

Both detailed probes produced this hierarchy:

```text
supplied parent span
└── claude_code.interaction
    ├── claude_code.llm_request × 2
    ├── claude_code.hook × 4
    └── claude_code.tool
        ├── claude_code.tool.blocked_on_user
        └── claude_code.tool.execution
```

The enhanced-only JSON probe produced five spans.
The enhanced-only protobuf probe produced four, with only one model span despite two API log records.
The spike does not establish why that span was absent.
Enhanced-only tracing therefore did not provide a complete session trace in these tests.
The documentation describes interaction parenting more broadly than these observations support.
Do not assume the enhanced beta switch alone supplies that behavior on this version.

### Model and tool records

The event body contains the full name below.
The `event.name` attribute contains its suffix, such as `api_request`.
Common attributes include the four `ws.*` values, `session.id`, `user.id`,
`terminal.type`, `event.timestamp`, `event.sequence`, and usually `prompt.id`.
Request identifiers are distinct from the tool-use identifier.

| Event | Observed fields beyond common attributes |
|---|---|
| `claude_code.api_request` | `model`, `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_creation_tokens`, `cache_creation_5m_tokens`, `cache_creation_1h_tokens`, `cost_usd`, `cost_usd_micros`, `duration_ms`, `ttft_ms`, `request_id`, `client_request_id`, `speed`, `query_source`, `effort` |
| `claude_code.tool_decision` | `decision`, `source`, `tool_name`, `tool_use_id`, `tool_source`, `permission_mode` |
| `claude_code.tool_result` | `tool_name`, `tool_use_id`, `success`, `duration_ms`, `tool_input_size_bytes`, `tool_result_size_bytes` |
| `claude_code.user_prompt` | `prompt_length`, `prompt`, `prompt_text`, `message.uuid` |
| `claude_code.assistant_response` | `response_length`, `response`, `request_id`, `message.uuid`, `model`, `query_source` |

With `OTEL_LOG_TOOL_DETAILS=1`, the tool-result record also contained
`tool_parameters` and `tool_input`. The baseline omitted those fields.
The agent redacted baseline prompt and response content.
The detailed runs included `claude_code.system_prompt` and `claude_code.tool` records.
The committed samples replace system prompts, tool definitions, and other large context values with redaction markers.

`claude_code.llm_request` spans included token counts, duration, success, attempt,
request identifiers, and `gen_ai.*` model attributes.
Tool spans included `tool_use_id` and `gen_ai.tool.call.id`.
`OTEL_LOG_TOOL_CONTENT=1` produced a `tool.output` span event with command output.
This was not a separate tool-result log containing stdout.
The hook's `tool_response` also contained stdout, stderr, and interruption status.

### Hook boundaries

Every run produced exactly this callback sequence:

```text
SessionStart → UserPromptSubmit → PreToolUse → PostToolUse → Stop → SessionEnd
```

The [detailed hook sample](s2/samples/detailed-json-hooks.jsonl) preserves the payload shapes.
`UserPromptSubmit`, tool callbacks, and `Stop` shared `prompt_id`.
`PreToolUse` and `PostToolUse` shared `tool_use_id` and `tool_input`.
`PostToolUse` included `tool_response` and `duration_ms`.
`Stop` included `stop_hook_active=false` and `last_assistant_message`.

**The callbacks identify exact logical boundaries for the tested successful turn and tool.**
They do not contain an authoritative timestamp for each execution boundary.
The recorder timestamps callback receipt, which includes scheduling and process startup time.
For example, the detailed JSON tool callbacks were 92.79 ms apart,
while `PostToolUse.duration_ms` reported 50 ms.
Use agent durations or spans for execution timing. Treat hook receipt times as observations.

OTLP also contained `claude_code.hook_registered`, `claude_code.hook_execution_start`,
and `claude_code.hook_execution_complete` events.
The execution records included `hook_event`, `hook_name`, `num_hooks`, and policy/source fields.
Completion records included outcome counts and `total_duration_ms`.
They did not include the callback's tool-use ID.
Each run exported five start/complete pairs, with no SessionEnd pair in the captured logs.
Detailed tracing exported four hook spans for prompt submission, pre-tool, post-tool, and Stop.
The callback recorder nevertheless received SessionEnd in all runs.
Use the callback payloads for lifecycle identity rather than assuming OTLP hook records cover every callback.

Failed tools, denied permissions, asynchronous tools, resumed sessions, subagents,
interactive mode, and blocked Stop hooks were not tested.
The settings included `PostToolUseFailure`, but that hook did not fire.
These results establish the successful non-interactive path only.

### Evidence and reproduction

The [sample manifest](s2/samples/manifest.json) lists selected request bodies and their original hashes.
Samples preserve full selected payload structures after redaction.
JSON samples use readable formatting.
The sample transformation decoded, redacted, and serialized protobuf again. The samples differ from the original wire bytes.
Their `.pb.json` companions use the protobuf JSON mapping, with base64 byte fields.
HTTP JSON trace IDs use hexadecimal strings instead.
User and runtime identifiers use stable synthetic replacements.
The transformation removed paths, account data, email addresses, and context content.
Raw authenticated captures remain only in the temporary sandbox.

Start the sink with:

```sh
python3 -I docs/ws-v6/spikes/s2/capture-sink.py --output /tmp/s2-capture --port 0
```

The first stdout line reports the selected port as JSON.
In a sandbox project, apply the common environment above and the chosen mode's additions.
For hook evidence, use the recorder and settings below.
Set `S2_TMP` to a fresh temporary directory.
Set `S2_HOOK_OUTPUT` to a file inside that directory.

```sh
cat > "$S2_TMP/record-hook.py" <<'PY'
import json, os, sys, time
record = {"received_unix_ns": time.time_ns(), "payload": json.load(sys.stdin)}
with open(os.environ["S2_HOOK_OUTPUT"], "a") as target:
    target.write(json.dumps(record) + "\n")
PY
export S2_HOOK_RECORDER="$S2_TMP/record-hook.py"
python3 - <<'PY' > "$S2_TMP/hooks.json"
import json
events = ("SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse",
          "PostToolUseFailure", "Stop", "SessionEnd")
command = 'python3 -I "$S2_HOOK_RECORDER"'
print(json.dumps({"hooks": {event: [{"hooks": [
    {"type": "command", "command": command}
]}] for event in events}}))
PY
```
Then run this synthetic probe with the sandbox settings:

```sh
claude -p 'Run exactly one Bash tool call. Print a synthetic marker and run printenv TRACEPARENT. Then reply S2_DONE.' \
  --model haiku --tools Bash --allowedTools Bash \
  --permission-mode bypassPermissions --dangerously-skip-permissions \
  --setting-sources user --settings "$S2_TMP/hooks.json" \
  --strict-mcp-config --mcp-config '{"mcpServers":{}}' --output-format json
```

Run the sink's behavioral checks with:

```sh
python3 -I -m unittest discover -s docs/ws-v6/spikes/s2 -p 'test_*.py'
python3 -I -m py_compile docs/ws-v6/spikes/s2/capture-sink.py
```

The tests cover raw JSON, chunked binary bodies, compressed bodies, duplicate requests,
and omission of authorization headers from saved metadata.

## Verdict

**Partial.** Local metrics and logs work. Custom resource attributes survive on all three signals.
The hook callbacks identify the successful turn and tool boundaries.
Detailed beta tracing also honors the inbound parent and exports a coherent turn trace.
Enhanced-only tracing does not meet that requirement in this installation.

## Impact on the plan

Keep architecture §7's hook-based span synthesis as the reliable baseline.
Phase 18 should preserve both `prompt_id` and `tool_use_id` from callback JSON.
Phase 41 should support standard logs and metrics without requiring beta tracing.
Its receiver must accept JSON and protobuf on each OTLP route.

Phases that consume model events can use `api_request` durations and request IDs.
They should join tool results by `tool_use_id`, rather than time alone.
A detailed-tracing option can use the WS session trace ID through `TRACEPARENT`.
That option needs the detailed pair, explicit capability checks, and a decision about duplicate synthesized spans.
The detailed exporter exposes additional context and overrides logs/traces encoding in this test.
Keep it optional until its version-dependent behavior receives integration coverage.
