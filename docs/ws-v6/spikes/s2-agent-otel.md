# S2 — Agent OTel export: Claude Code and Codex

## Question

Which signals can Claude Code and Codex export to a local OTLP receiver?
Do custom resource attributes and an environment `TRACEPARENT` survive?
Can hooks identify turn and tool boundaries?

## Method

### Claude Code

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

### Codex CLI

Test date: 2026-10-16. Agent: **codex-cli 0.159.2**.
Every probe selected `gpt-6.1-sol` with reasoning effort `low`.

Agent-home mode: **sandbox**. The probes used a temporary `HOME`, `CODEX_HOME`,
`CLAUDE_CONFIG_DIR`, and `XDG_CONFIG_HOME`.
The sandbox received copies of `auth.json` and `models_cache.json` from the live Codex home.
The copied ChatGPT credentials authenticated all eight probes.
No live fallback was necessary.
Every run used `--ignore-user-config --ignore-rules` in an empty temporary project.
No sandbox `config.toml` existed.
The live Codex configuration retained its original SHA-256.
All sessions and rollouts belonged to the sandbox.

Eight synthetic inference calls and two inspection calls (`--version`, `exec --help`) used the entire **10-call allowance**.
Each inference call created a new session and returned `S2_CODEX_DONE`.
Each executed one shell command through `exec_command` inside the `functions.exec` wrapper.
The wrapper produced a second OTel tool-result record without a second shell command.
The `failure` probe executed `exit 7`.
The `no-parent` probe omitted `TRACEPARENT`, so its `printenv TRACEPARENT` exited 1.
The other six shell commands exited 0.

Every inference call passed **all `[otel]` settings through `-c` flags**.
No command edited `~/.codex/config.toml`.
The baseline configured all three exporters:

```sh
-c 'otel.environment="s2-spike"'
-c 'otel.log_user_prompt=false'
-c 'analytics.enabled=true'
-c 'otel.exporter={otlp-http={endpoint="http://127.0.0.1:<port>/v1/logs",protocol="json"}}'
-c 'otel.trace_exporter={otlp-http={endpoint="http://127.0.0.1:<port>/v1/traces",protocol="json"}}'
-c 'otel.metrics_exporter={otlp-http={endpoint="http://127.0.0.1:<port>/v1/metrics",protocol="json"}}'
```

Replace `<port>` with the capture sink's selected port.
Codex uses `json` or `binary` as the protocol value.
These values differ from Claude's environment protocol names.
Each endpoint included its full signal path.
The probes reused the Phase 6 sink without changes.
They set `OTEL_METRIC_EXPORT_INTERVAL=1000`.
They supplied these resource attributes and trace context:

```sh
OTEL_RESOURCE_ATTRIBUTES=ws.session.id=s2-codex-session,ws.parent_session.id=s2-codex-parent,ws.device.id=s2-codex-device,ws.project=s2-codex-project
TRACEPARENT=00-11111111111111111111111111111111-2222222222222222-01
```

The recorder used the same stdin JSON and receipt-time format as the Claude recorder.
Each run configured exactly the six Codex hook events in `ws/lib/tabs.sh:135-148`.
The recorder replaced `ws hook <state>` and returned no output.
Thus the probes exercised Codex callbacks without starting ws or tmux.
The invocation included `--dangerously-bypass-hook-trust`, as the ws invocation does.
It also included `--dangerously-bypass-approvals-and-sandbox` for the synthetic command.
Permission prompts therefore did not occur.
The sink stopped two seconds after each agent exited.

The official [advanced configuration](https://learn.chatgpt.com/docs/config-file/config-advanced#observability-and-telemetry)
and [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
supplied the exporter settings and the analytics dependency.
No Context7 tool was available in this runner.
The results below describe captured requests from this installed version.

## Findings

### Claude Code

#### Signals and switches

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

#### Resource attributes and trace context

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

#### Model and tool records

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

#### Hook boundaries

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

#### Evidence and reproduction

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

### Codex CLI

#### Signals and switches

Counts below are HTTP requests, not records or model calls.
All rows used JSON except `protobuf`, which used `protocol="binary"` for all exporters.

| Probe | Change from baseline | Metrics / logs / traces |
|---|---|---|
| baseline | All three exporters enabled | 6 / 7 / 3 |
| protobuf | Binary protobuf protocol | 6 / 7 / 3 |
| off | All three exporter values `"none"` | 0 / 0 / 0 |
| analytics-off | `analytics.enabled=false` | 0 / 6 / 3 |
| prompt-enabled | `otel.log_user_prompt=true` | 5 / 4 / 3 |
| trace-off | `otel.trace_exporter="none"` | 6 / 7 / 0 |
| failure | Shell command `exit 7` | 6 / 4 / 3 |
| no-parent | No environment `TRACEPARENT` | 7 / 5 / 3 |

The three signal routes were `/v1/metrics`, `/v1/logs`, and `/v1/traces`.
JSON requests used `application/json`.
Binary requests used `application/x-protobuf`.
The `binary` setting controlled all three signal encodings.
`analytics.enabled=false` disabled custom OTLP metrics while logs and traces continued.
A receiver endpoint alone does not enable exporters whose value is `"none"`.
The probes did not test gRPC or environment-only endpoint configuration.

Selected metrics included `codex.tool.call`, `codex.tool.call.duration_ms`,
`codex.hooks.run`, `codex.hooks.run.duration_ms`, `codex.turn.e2e_duration_ms`,
`codex.turn.token_usage`, `codex.websocket.request`, and `codex.websocket.request.duration_ms`.
The exports contained additional startup, filesystem, plugin, and MCP activity.
Metrics describe aggregated activity and do not supply call identifiers.

#### Resource attributes and trace context

**All four supplied `ws.*` values survived on every captured resource for every enabled signal.**
The built-in resource identified `service.name=codex_exec` and `service.version=0.159.2`.
The log records also carried `conversation.id`, which matched the hook `session_id`.
Use the resource's `ws.session.id` as the WS identity.

**Codex honored the environment parent for the main execution trace.**
In both baseline protocols, `codex.exec` used trace ID `11111111111111111111111111111111`
and parent span ID `2222222222222222`.
The main `session_task.turn` and its sampling spans used that trace.
The JSON baseline exported **1,307 spans across 29 traces**.
Of those spans, 1,012 used the supplied trace ID.
The binary baseline exported 1,305 spans across 28 traces.
Of those spans, 1,011 used the supplied trace ID.
The no-parent control exported 1,299 spans across 33 traces and none used the supplied trace ID.

The main trace included this execution path:

```text
supplied parent → codex.exec → … → session_task.turn → session_task.run → run_turn
  run_turn → run_sampling_request → try_run_sampling_request → stream_request
```

The ellipsis omits captured infrastructure ancestors.
There were two `run_sampling_request` spans for the shell-command turn.
`session_task.turn` carried `turn.id`, `model`, `codex.turn.reasoning_effort`,
and `codex.turn.token_usage.*` fields.
The token fields covered input, cached input, cache-write input, non-cached input,
output, reasoning output, and total tokens.

The nested shell execution exported `exec_command` and `unified_exec.exec_command` in a **different trace**.
Their `call_id` matched the shell hook's `tool_use_id`.
`unified_exec.exec_command` also carried `conversation.id` and `turn_id`.
Background work produced further traces.
Raw native traces therefore do not implement WS's one-trace-per-session contract.

The shell process inherited the original environment `TRACEPARENT` unchanged.
Codex did not replace its parent span ID with the current tool span ID.
With trace export disabled, all six model transport/completion records lacked OTLP trace and span IDs.
Even with tracing enabled, the baseline completion logs lacked those IDs.
Resource identity survives independently of native trace parenting.

#### Model and tool records

Codex identifies each named event through its `event.name` attribute.
The OTLP body was null on the selected structured events.
Common session records carried `event.timestamp`, `app.version`, `auth_mode`,
`originator`, `terminal.type`, `model`, `slug`, and `conversation.id`.
Authenticated records also contained account identity fields, which the committed samples redact.

| Event | Observed fields beyond common attributes |
|---|---|
| `codex.conversation_starts` | `approval_policy`, `sandbox_policy`, `provider_name`, `reasoning_effort`, `reasoning_summary`, `mcp_servers`, authentication-presence flags |
| `codex.api_request` | `attempt`, `duration_ms`, `http.response.status_code`, `success`, `endpoint`, authentication-presence flags |
| `codex.websocket_request` | `duration_ms`, `success`, `auth.connection_reused`, authentication-presence flags |
| `codex.sse_event` with `event.kind=response.completed` | `input_token_count`, `output_token_count`, `cached_token_count`, `cache_write_token_count`, `reasoning_token_count`, `tool_token_count`, optional `ttft_ms`, `model_reasoning_effort` |
| `codex.user_prompt` | `prompt_length`, `prompt` |
| `codex.tool_decision` | `tool_name`, `tool_namespace`, `call_id`, `decision`, `source` |
| `codex.tool_result` | `tool_name`, `tool_namespace`, `call_id`, `duration_ms`, `success`, `arguments`, `output`, `output_truncated`, `tool_result_seq`, `agent_name`, MCP fields |

**The baseline `codex.api_request` described `/models`, not model inference.**
The model used WebSockets.
Three `codex.sse_event` completion records included one prewarm completion with zero output tokens.
The other two completions represented the tool request and final answer.
Do not count every completion as a user-visible model step without classifying prewarm activity.

The baseline WebSocket request durations were 1, 0, and 0 milliseconds.
The two `responses_websocket.stream_request` spans lasted about 2065 and 1248 milliseconds.
The sampling spans lasted about 2197 and 1254 milliseconds and included tool dispatch.
Do not equate the whole sampling interval with model latency.
Thus `codex.websocket_request.duration_ms` did not measure full inference latency in this probe.
The completion logs did not contain a full inference duration or a request identifier.
**Logs alone did not establish exact model-call start times.**
Use native streaming spans to measure request intervals with native tracing.
Otherwise record timing as estimated or retain completion events without claiming exact model spans.

`otel.log_user_prompt=false` replaced prompt content with `[REDACTED]`.
The prompt-enabled probe exported the synthetic prompt.
The baseline still exported tool arguments and output.
Prompt redaction does not remove tool content or account identity fields.

Each run with logs enabled exported two tool-result records.
One described `functions.exec` and the other described its nested `functions.exec_command`.
Only the nested shell call had the tested pre/post hook pair.
The hooks therefore did not cover every OTel tool operation.
Wrapper operations need their own tool-result policy if WS represents them as spans.
Match its `call_id` to `tool_use_id` rather than counting both results as shell executions.

The [failure excerpt](s2/samples/codex-failure-logs.json) reports `success="true"` for the shell tool.
The [exec output](s2/samples/codex-failure-exec.jsonl) records command exit code 7 and status `failed`.
The PostToolUse callback still fired and supplied an empty output string.
This result distinguishes tool transport success from shell command success.
Neither that callback nor its OTel tool-result record supplied the command exit code.

#### Hook boundaries and current ws mapping

Every probe produced this callback sequence, including the failed shell command:

```text
UserPromptSubmit → PreToolUse → PostToolUse → Stop
```

The [baseline hook sample](s2/samples/codex-baseline-hooks.jsonl) preserves all callback fields.
Every callback carried `session_id`, `turn_id`, `hook_event_name`, `model`, and `permission_mode`.
Prompt submission included `prompt`.
Both tool callbacks included `tool_name=Bash`, `tool_input.command`, and the same `tool_use_id`.
PostToolUse supplied `tool_response` as a string, without duration or structured exit status.
Stop supplied `stop_hook_active=false` and `last_assistant_message`.

| Codex event | Current injected command | Required WS interpretation |
|---|---|---|
| `UserPromptSubmit` | `ws hook working` | Start the turn identified by `turn_id`. |
| `PreToolUse` | `ws hook working` | Start a child tool span identified by `tool_use_id`. |
| `PostToolUse` | `ws hook working` | End that tool span. Preserve its output and resolve command status separately. |
| `PermissionRequest` | `ws hook waiting` | Record a wait within the active turn or tool. Do not end the turn. |
| `Stop` | `ws hook stop` | End the turn if ws accepts Stop. A blocked Stop continues the turn. |
| `Interrupt` | `ws hook cancel` | End the matching turn as interrupted. Reject stale turn IDs. |

The injection loop assigns the first three callbacks the same `working` state.
`ws/lib/hooks.sh` reads `hook_event_name` and `turn_id` before it changes that state.
It emits notifications only when the state changes.
Consequently, its current notifications cannot reconstruct every pre/post boundary.
Phase 18 must retain each callback's original event name, IDs, and payload before state reduction.
The existing state mapping can continue to drive the tab display.

The callbacks establish logical boundaries for these completed shell turns.
They do not supply execution timestamps.
The baseline recorder received the tool callbacks 84.29 ms apart.
The corresponding OTel tool result reported 40 ms.
Receipt time includes hook startup and scheduling.
PermissionRequest and Interrupt did not fire, so their mapping above comes from the injection code.
Interactive sessions, resumed turns, denied tools, asynchronous tools, blocked Stop hooks,
and process termination remain untested.

#### Evidence and reproduction

The [Codex manifest](s2/samples/codex-manifest.json) records each run's flags, counts, and trace totals.
It records source request hashes for each selected signal.
The Codex OTLP samples merge decoded requests and retain selected records.
Trace excerpts include execution spans and their captured ancestors.
Metric excerpts retain selected execution metrics.
The samples do not represent complete original wire requests.
The transformation redacted identity, paths, and unrelated context values.
It used stable synthetic identifiers across logs, spans, and callbacks.
The `.pb` samples contain redacted protobuf, with `.pb.json` companions.
Their companion trace and span IDs use base64, as the protobuf JSON mapping requires.
The HTTP JSON samples retain hexadecimal trace and span IDs.
Raw captures remain in the temporary sandbox.

To reproduce the baseline:

1. Create a temporary agent home and an empty project.
2. Copy Codex authentication and the model cache into the temporary `CODEX_HOME`.
3. Start the capture sink and use its port in the three exporter flags above.
4. Apply the resource attributes and trace context above.
5. Create the recorder from the Claude reproduction section.
6. Construct the six hook flags with this zsh loop:

```sh
hook_flags=()
for event in UserPromptSubmit PreToolUse PostToolUse PermissionRequest Stop Interrupt; do
  command_json=$(jq -rn --arg c "python3 -I \"$S2_HOOK_RECORDER\"" '$c | @json')
  hook_flags+=(-c "hooks.$event=[{hooks=[{type=\"command\",command=$command_json}]}]")
done
```

Run `codex exec` with those hook flags and the six baseline configuration flags above.
Add these arguments:

```sh
--skip-git-repo-check --ignore-user-config --ignore-rules --json
--dangerously-bypass-approvals-and-sandbox --dangerously-bypass-hook-trust
-c 'features.shell_snapshot=false' -c 'model_reasoning_effort="low"'
-C "$S2_PROJECT" -m gpt-6.1-sol
```

Use the synthetic prompt:

```text
Run exactly one exec_command tool call with this command: printf S2_CODEX_TOOL; printenv TRACEPARENT. Then reply S2_CODEX_DONE. Do not read files or use other tools.
```

Check sink requests after the process exits and the two-second drain interval ends.
For binary probes, replace every exporter protocol value with `binary`.
The settings table defines the remaining probe variants.

## Verdict

**Partial for both agents.** Both agents export local logs and metrics with custom resource attributes.
Both agents also export native traces under the tested settings.
The tested callbacks identify completed turn and shell-tool boundaries.
They do not provide precise execution timestamps.

Claude requires the detailed beta pair for the coherent parented turn trace observed here.
Codex's main trace honors `TRACEPARENT`, but nested tools and background work create additional traces.
Neither agent's tested native export provides the complete WS trace contract without normalization.

**`wsd` must synthesize these canonical spans for both agents:**

- One turn segment from prompt submission to an accepted Stop, interruption, or process exit.
- One child tool span per pre/post callback pair, matched by tool-use ID.
- One phase-attempt segment when a Loop runner executes a phase.
- Model-call spans where agent records establish their timing and identity.
  Codex's default completion logs support token attribution but require estimated timing or optional native streaming spans.

Use the WS session ULID to derive the canonical trace ID.
Add span links for parent sessions, dependencies, and Loop relations.
Keep native traces optional and prevent duplicate canonical model/tool spans.
Do not create a session span that remains open across turns.

## Impact on the plan

Keep architecture §7's hook-based turn/tool synthesis as the baseline.
Phase 18 must preserve every callback before state reduction.
Preserve Claude's `prompt_id`, Codex's `turn_id`, and both agents' tool-use identifiers.
A display-state notification stream loses boundaries when consecutive callbacks all map to `working`.
Use callback receipt times as observations and retain agent durations separately.
A blocked Stop must continue the same turn.

Phase 41 must inject three independent Codex exporter configurations through `-c` flags.
Codex metrics also require analytics to remain enabled.
The receiver must accept JSON and protobuf on every OTLP route.
Resource `ws.session.id` supplies correlation even when native trace context is absent.
Keep beta Claude tracing and Codex native tracing optional.
Native tool traces can differ from the WS session trace.

Phase 42 can implement successful shell boundaries from these samples.
It must also close interrupted or abandoned spans and test the unobserved lifecycle paths.
Do not infer a failed command from Codex's OTel `success` field alone.

**Revise Phase 43 at the milestone gate.**
Claude model events provide request IDs, token counts, and durations.
Codex WebSocket inference uses completion logs rather than the captured `/models` API event.
Its completion logs provide tokens but no full inference duration or request ID.
Pair transport/completion observations only with explicit uncertainty and prewarm classification.
Use native streaming spans to measure Codex request intervals.
Otherwise retain completion events or mark synthesized timing as estimated.
Match tool results by tool-use ID rather than time alone.

The detailed Claude exporter exposes additional context and overrides logs/traces encoding in this installation.
Codex exports tool content even with prompt logging disabled.
Retain version checks, content controls, and integration coverage for optional native tracing.
