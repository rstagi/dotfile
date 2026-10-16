# S3 — Runner quota sources

## Question

Can ws read the remaining 5-hour and weekly quota for Claude Code, Codex, and Grok?
Which fields identify the account and the window?
How fresh are the readings?
What must ws estimate when a source omits a value?

## Method

Test date: **2026-10-16**. Host: macOS arm64.
Installed versions: Claude Code **2.1.302**, Codex **0.159.2**, Grok Build **1.0.52**.
The [manifest](s3/manifest.json) records all **10 CLI invocations**, including failed inspection calls.
Four calls completed synthetic inference turns. None executed an agent tool.

Agent-home mode: **sandbox**. No live-home fallback was necessary.
Every invocation used a temporary `HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GROK_HOME`, and `XDG_CONFIG_HOME`.
The project directory was empty and temporary.

The sandbox received copies of these authentication sources:

- Codex: `auth.json` and `models_cache.json`.
- Claude: OAuth credentials from a read-only Keychain lookup, plus selected onboarding and account metadata.
- Grok: `auth.json`.

Claude required its onboarding metadata at `$CLAUDE_CONFIG_DIR/.claude.json`.
The first interactive attempt lacked that file and stopped before inference.
The second attempt used the copied metadata and accepted trust for the temporary project.
Claude used subscription authentication. The probe removed the inherited `ANTHROPIC_API_KEY` from its environment.

The probes did not load live agent settings, change live configuration, or change existing sessions.
Codex wrote its new rollout inside the sandbox.
The interactive Claude probe inherited a child-session marker that disabled transcript persistence.
The recorder captured its status-line input and terminal output inside the sandbox.

The account and billing probes used read-only RPC methods and HTTP `GET` requests.
They did not log in, log out, redeem reset credits, or change billing settings.
The Codex account request set `refreshToken: false`.
Any automatic credential maintenance belonged to the copied sandbox home.

The four JSON evidence files contain field projections, not full credential or session files.
They preserve absent fields, `null`, field names, units, and observed numeric values.
They replace account identifiers and email addresses with redaction markers.
They omit credentials, signatures, prompts from other sessions, and unrelated billing data.

Official documentation supplied the documented interfaces.
Local captures supplied the version-specific behavior below.
No Context7 tool was available in this runner.
No reset, exhaustion, account-switch, or long-duration test ran.

## Findings

### Overview

A remaining ratio uses the interval 0–1. A remaining percentage uses 0–100.
A missing window means **unknown or unavailable**, not zero usage.

| Runner and source | 5-hour result | Weekly result | Account identity |
|---|---|---|---|
| Codex app-server and rollout | Absent for this account | 45% used, 55% remaining | RPC `result.accountId`, matching copied `tokens.account_id` |
| Claude status-line input | 0% used, 100% remaining | 14% used, 86% remaining | Separate profile `account.uuid` and `organization.uuid` |
| Claude headless JSON stream | Fraction 0 used, ratio 1 remaining | Fraction 0.14 used, ratio 0.86 remaining | Same launch-time account binding |
| Claude usage HTTP response | 0% used | 15% used | Same profile binding |
| Grok billing HTTP and ACP | No window exposed | Weekly dates exposed, usage percentage absent | Copied auth entry `user_id`, with principal and team context |

These rows are separate observations. They do not imply simultaneous, identical provider snapshots.
Claude's HTTP reading differed from its later response-derived reading by one percentage point.
The spike did not establish the cause or a maximum consistency delay.

### Codex

Evidence: [codex.json](s3/codex.json).

#### Sources and fields

Launch `codex app-server` with the sandbox environment.
The rejected global `--ignore-user-config` and `--ignore-rules` flags belong to `exec`, not this invocation.
The successful app-server used no sandbox `config.toml` and no thread or model request.

Send `initialize` with `clientInfo.name` and `clientInfo.version`.
Then send the `initialized` notification.
The subsequent quota request is:

```json
{"id":3,"method":"account/rateLimits/read"}
```

The official [app-server reference](https://learn.chatgpt.com/docs/app-server#6-rate-limits-chatgpt) documents this method and its update notification.
The installed version also returned `result.accountId`.
That value matched `tokens.account_id` in the copied `$CODEX_HOME/auth.json`.
`account/read` returned account type, email, and plan type, but no stable account identifier.

| App-server field | Rollout field under `payload.rate_limits` | Meaning |
|---|---|---|
| `result.rateLimits.limitId` | `limit_id` | Metered bucket, not account identity |
| `primary.usedPercent` or `secondary.usedPercent` within a bucket | `primary.used_percent` or `secondary.used_percent` | Used percentage, 0–100 |
| `primary.windowDurationMins` or `secondary.windowDurationMins` | `primary.window_minutes` or `secondary.window_minutes` | Window duration in minutes |
| `primary.resetsAt` or `secondary.resetsAt` | `primary.resets_at` or `secondary.resets_at` | Reset time, Unix seconds |
| `result.rateLimitsByLimitId` | Separate events may carry different `limit_id` values | Multiple metered buckets |
| `result.accountId` | Absent from the captured token-count payload | Account identifier |

The abbreviated app-server window paths refer to each `result.rateLimits` or `result.rateLimitsByLimitId[limitId]` bucket.
Do not assume that `primary` always means five hours.
Match **300 minutes** to five hours and **10080 minutes** to one week.
Preserve other durations as separate windows.

The observed `primary.windowDurationMins` was **10080**.
Its `usedPercent` was **45**, and `secondary` was `null`.
The observed response therefore supplied a weekly reading and no five-hour reading.
`planType` was `prolite`. Plan type does not identify the account.

A synthetic `codex exec` call also produced one new rollout under `$CODEX_HOME/sessions/`.
Its JSONL record had `type: "event_msg"` and `payload.type: "token_count"`.
Its `payload.rate_limits` contained the same weekly reading, with snake_case field names.
Its top-level `timestamp` was `2026-10-16T21:32:33.638Z`.

#### Freshness and account binding

Two quota requests completed at `21:31:57.783605Z` and `21:31:58.290556Z`.
Both returned the same values.
The responses contained no quota measurement timestamp or maximum cache age.
Record request receipt time as `observed_at`. Do not describe it as the provider's measurement time.

A rollout changes only when its runner writes an event.
An idle or stopped session can retain an old quota snapshot indefinitely.
Use the event timestamp for its reading age, not the time when a collector reads the file.
`account/rateLimits/updated` can supply pushed readings while app-server runs.
A timer can use `account/rateLimits/read` without launching an inference turn.

Prefer the quota response's account identifier when it exists.
For rollout-only collection, bind the session to its account at launch.
Do not assign an old rollout to whichever account currently appears in `auth.json`.
Keep readings separate by account, bucket, duration, and reset time.

### Claude Code

Evidence: [claude.json](s3/claude.json).

#### Status-line input

The official [status-line reference](https://code.claude.com/docs/en/statusline) documents both subscription windows.
Configure a command recorder through sandbox `--settings`.
The command receives JSON on stdin.

| Field | Unit |
|---|---|
| `rate_limits.five_hour.used_percentage` | Used percentage, 0–100 |
| `rate_limits.five_hour.resets_at` | Reset time, Unix seconds |
| `rate_limits.seven_day.used_percentage` | Used percentage, 0–100 |
| `rate_limits.seven_day.resets_at` | Reset time, Unix seconds |

Compute `remaining_ratio = 1 - used_percentage / 100` for a valid, unexpired window.
The first callback omitted `rate_limits`.
The next four callbacks contained both windows.
They reported five-hour usage **0** and weekly usage **14**.

The recorder requested `statusLine.refreshInterval: 1`.
Those repeated callbacks retained the same quota values after the response.
A callback refresh does not prove a new provider measurement.
The payload contained neither an account identifier nor a quota measurement timestamp.

The documentation limits subscription fields to eligible Pro and Max accounts after the first API response.
Each window can be independently absent.
Claude drops an expired window from the status-line input.
The captured account used Claude Max.
API-key, gateway, and other account types need separate handling.

#### Headless JSON stream

The successful headless call used these output flags:

```sh
claude -p --output-format stream-json --verbose <synthetic-prompt>
```

The full probe also supplied sandbox settings, empty tools, and an empty strict MCP configuration.
The configured status-line command received **zero callbacks** in this mode.
However, stdout emitted a `rate_limit_event` after the response.
Its exact window fields were:

| Field | Unit |
|---|---|
| `rate_limit_info.unifiedWindows.five_hour.utilization` | Used fraction, 0–1 |
| `rate_limit_info.unifiedWindows.five_hour.resetsAt` | Reset time, Unix seconds |
| `rate_limit_info.unifiedWindows.seven_day.utilization` | Used fraction, 0–1 |
| `rate_limit_info.unifiedWindows.seven_day.resetsAt` | Reset time, Unix seconds |

Compute `remaining_ratio = 1 - utilization` for this source.
The captured fractions were **0** and **0.14**.
Do not divide these fractions by 100 again.
`status: "allowed"` did not prevent the event from reporting both windows.
The event lacked a provider measurement timestamp and account identity.

Treat `unifiedWindows` as an observed wire interface for this installed version.
The status-line documentation does not establish this headless field as a stable contract.
Parse optional fields defensively and retain the token estimate when a field disappears.
Observe existing runner turns. Do not spend inference turns solely to poll quota.

#### Read-only usage and identity requests

A sandbox credential authenticated `GET https://api.anthropic.com/api/oauth/usage`.
The request used the `anthropic-beta: oauth-2025-04-20` header.
The response supplied `five_hour.utilization` and `seven_day.utilization` as **percentages**, not fractions.
Its `resets_at` fields were ISO 8601 timestamps, unlike the two CLI sources.
The sample reported **0** and **15**.
This HTTP source remains an internal interface, not a documented public polling contract.

A separate `GET https://api.anthropic.com/api/oauth/profile` returned `account.uuid` and `organization.uuid`.
Both matched the copied `oauthAccount.accountUuid` and `oauthAccount.organizationUuid` metadata.
Bind the runner session to these identifiers before accepting account-wide readings.
Email and session identifiers are not substitutes for the account identifier.

The usage response's HTTP `Date` was `21:31:58 GMT`.
That header identifies the HTTP response time, not the underlying quota measurement time.
For response-derived CLI readings, record event receipt time and reading source.
For status-line callbacks, retain the last response observation time when possible.
Repeated callbacks alone must not make an idle reading appear fresh.

### Grok Build

Evidence: [grok.json](s3/grok.json).

#### Billing sources and missing windows

`grok --help` distinguishes the persisted session-token command `grok usage` from interactive `/usage` billing.
An ACP process supports a read-only billing extension:

```json
{"jsonrpc":"2.0","id":2,"method":"_x.ai/billing","params":{}}
```

Initialize `grok agent stdio` with `protocolVersion: 1` first.
No session or inference turn is necessary for the billing request.
The ACP wire method requires the leading underscore.
The unprefixed `x.ai/billing` request returned `-32601`, `Method not found`.

The official [billing implementation](https://github.com/xai-org/grok-build/blob/2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8/crates/codegen/xai-grok-shell/src/extensions/billing.rs) defines the response fields and its HTTP source.
The same credentials also authenticated this direct read:

```text
GET https://cli-chat-proxy.grok.com/v1/billing?format=credits
```

The request supplied authorization in memory, plus `X-XAI-Token-Auth: xai-grok-cli` and `x-userid`.
No credential appeared in argv, output, or committed evidence.

| Response field | Meaning and observation |
|---|---|
| `config.creditUsagePercent` | Used percentage, 0–100. Absent from every captured response |
| `config.currentPeriod.type` | Reported `USAGE_PERIOD_TYPE_WEEKLY` |
| `config.currentPeriod.start` | ISO 8601 time at the lower window boundary |
| `config.currentPeriod.end` | ISO 8601 reset time |
| `config.monthlyLimit.val`, `config.used.val` | Legacy credit denominator and usage. Both absent |
| `config.billingPeriodStart`, `config.billingPeriodEnd` | Legacy dates. They matched the weekly period in this capture |
| `config.onDemandUsed.val`, `config.onDemandCap.val` | Extra-usage amounts. Both zero, not the included weekly quota |
| `subscription_tier` | ACP plan label. It reported `SuperGrok Heavy`, not an account identifier |

The reported window ran from `2026-10-14T10:09:21.510566+00:00` to `2026-10-21T10:09:21.510566+00:00`.
The source exposed **no five-hour window**.
It exposed a weekly period but **no usable weekly remaining percentage for this account**.
Do not convert missing `creditUsagePercent` into a claimed 100% remaining value.
This spike did not validate a contract that permits that default.

If a later reading supplies `creditUsagePercent`, compute `1 - creditUsagePercent / 100`.
Use `currentPeriod.type` and its dates to select the window.
A monthly period must remain monthly. Do not relabel it as weekly.
Use legacy amounts only when their denominator exists and exceeds zero.
Do not use on-demand or prepaid balances as the included subscription allowance.

#### Freshness and account binding

Two ACP billing reads completed at `21:33:24.245904Z` and `21:33:24.413334Z`.
They returned the same fields and values.
The direct HTTP response completed at `21:31:58.896374Z`.
Neither source returned a quota measurement timestamp or a maximum cache age.
Record local receipt time. Keep provider freshness unknown.

The billing payload contained no account identifier.
The copied `$GROK_HOME/auth.json` contained entries keyed by issuer and OAuth client.
Within the selected entry, `user_id` supplied the `x-userid` request header.
Retain `principal_type`, `principal_id`, and `team_id` as identity context when available.
The issuer/client key, plan label, email, and session id do not identify the quota owner alone.
A different authentication mode requires a verified binding before account aggregation.

The sandbox also recorded billing snapshots in `$GROK_HOME/logs/unified.jsonl`.
Their envelope used `ts`, `msg`, and `ctx.config` with `msg: "billing: fetched credits config"`.
Such log timestamps identify cached observations. They do not make old values current.
Prefer a direct read over scanning unrelated sessions or copying entire logs.

### Token-count fallback and aggregation

The [architecture](../architecture.md#12-runner-usage-and-quota) requires `reported|estimated` provenance.
Use reported quota when the account, window, value, and freshness are usable.
Otherwise sum observed token usage for that account and window and mark the result **estimated**.

The synthetic turns exposed these token sources:

| Runner | Exact fields | Counting rule |
|---|---|---|
| Codex | `payload.info.last_token_usage` and `payload.info.total_token_usage` in `token_count` events | Use per-call values or cumulative deltas, never repeated cumulative totals |
| Claude | Final stream `result.usage.input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`, `output_tokens` | These input categories are separate. Add them once |
| Grok | Stream `end.usage.total_tokens`, plus individual `input_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`, `output_tokens`, `reasoning_tokens` | Prefer the total. Do not also count the earlier `usage` event |

Codex reported **14232** total tokens. Its cached-input count was already inside its input total.
Claude reported **4247** tokens across its separate input categories and output.
Grok reported **13022** total tokens, including **1536** cached-input tokens.
Grok's **37** reasoning tokens were already inside its **44** output tokens.
A common formula that adds every field would count some tokens twice.

Deduplicate model calls before summing telemetry.
Keep account, runner, model, window boundaries, observed time, and coverage with each estimate.
Local telemetry misses other devices, sessions outside ws, and other products that share the account quota.
It therefore cannot establish the true account-wide remaining percentage by itself.

A token count also does not establish the provider's subscription denominator or model weights.
Show estimated tokens and coverage when no calibrated budget exists.
Leave the remaining ratio unknown in that case.
Do not invent a five-hour allowance for a provider that exposes no such window.
Do not present context-window usage as subscription quota.

For reported readings, retain the latest observation for each account, bucket, and window.
Do not add account-wide percentages from concurrent sessions.
Retain the source and reading age with every value.
After a reset passes, mark the old reading expired until a new report validates the next window.
An estimate must not silently replace a reported value without changing its provenance.

## Verdict

**partial**.

Codex provides a usable reported weekly reading through a read-only RPC and passive rollout events.
Its tested account exposes no five-hour window.
Claude provides both windows through status-line input and the observed headless JSON event.
Claude's headless field is version-specific and requires defensive parsing.
Grok provides a weekly billing period and a possible percentage field, but this account omits that percentage.
Its current remaining amount needs the token fallback or an unknown value.
No tested Grok source exposes a five-hour window.

## Impact on the plan

- **Phase 46:** Support Codex app-server reads and rollout parsing. Select windows by duration, not primary/secondary position.
- **Phase 46:** Capture the account binding at launch. Retain bucket identity, source timestamps, and missing windows.
- **Phase 47:** Read Claude's headless `unifiedWindows` alongside interactive status-line input. Normalize their different units explicitly.
- **Phase 47:** Try Grok's read-only billing extension before the estimator. Preserve missing percentages and validate any default separately.
- **Phase 47:** Deduplicate token records and retain incomplete telemetry coverage. Unknown denominators must produce unknown remaining ratios.
- **Phase 48:** Show each available window, its source, and its age. Show unavailable or unknown values explicitly.
- **Phase 49:** Keep preflight warnings advisory. Missing quota must not block execution or imply that an account has spare capacity.

These recommendations do not change the shared plan in this phase.
