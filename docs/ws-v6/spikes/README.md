# WS v6 milestone 1 spikes

The [architecture](../architecture.md) defines the spike questions and fallbacks in §15.
The reports below record the results for milestone 1.

| Spike | Report path | Scope |
|---|---|---|
| S1 | [s1-transcript-relocation.md](s1-transcript-relocation.md) | Claude Code and Codex transcript relocation |
| S2 | [s2-agent-otel.md](s2-agent-otel.md) | Agent OpenTelemetry export and hook boundaries |
| S3 | [s3-runner-quota.md](s3-runner-quota.md) | Runner quota sources and estimates |
| S5 | [s5-rust-skeleton.md](s5-rust-skeleton.md) | Rust daemon, tmux driver, and hook round trip |

S4 runs at the start of roadmap phase 6, outside this milestone.
The links identify planned reports. Each report appears when its spike phase completes.

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
