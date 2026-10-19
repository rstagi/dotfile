# WS v6 milestone 1 spikes

The [architecture](../architecture.md) defines the spike questions and fallbacks in §15.
The reports below record the results for milestone 1.

| Spike | Report path | Scope | Verdict at the milestone 1 gate |
|---|---|---|---|
| S1 | [s1-transcript-relocation.md](s1-transcript-relocation.md) | Claude Code and Codex transcript relocation | go (cross-machine run, macOS to Linux) |
| S2 | [s2-agent-otel.md](s2-agent-otel.md) | Agent OpenTelemetry export and hook boundaries | partial (wsd synthesizes spans from hooks) |
| S3 | [s3-runner-quota.md](s3-runner-quota.md) | Runner quota sources and estimates | partial (Codex weekly only, Claude headless event not reliable) |
| S5 | [s5-rust-skeleton.md](s5-rust-skeleton.md) | Rust daemon, tmux driver, and hook round trip | go |

S4 runs at the start of roadmap phase 6, outside this milestone.
The human recorded the verdicts at the milestone 1 merge gate on 2026-10-17.

The [S2 telemetry fixtures](../../../wsd/crates/ws-daemon/tests/fixtures/spikes/s2/)
and [S3 quota fixtures](../../../wsd/crates/ws-daemon/tests/fixtures/spikes/s3/) supply inputs for daemon tests.
The S1 scripts and observation files stay beside the reports.

## Report template

Use the following headings in each report.
Record the agent home mode in the method when a spike calls an agent.
Redact tokens, account ids, and email addresses from samples.

```markdown
## Question

State the question that the spike tests.

## Method

Describe the procedure, environment, and agent home mode.

## Findings

Record the results and evidence.

## Verdict

Choose one verdict: go | partial | no-go.
Explain the verdict and any limits.

## Impact on the plan

List the affected phases and required changes or fallbacks.
```
