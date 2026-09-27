# Loop Protocol

The shared contract between the `loop-execute` orchestrator skill, the `loop-*.sh` scripts
(`~/dotfile/loop-runner.sh`, `loop-merge.sh`, `loop-notify.sh`, `loop-state.sh`,
`loop-plan.sh`, `loop-repo.sh`, `loop-models.conf`), and the `--auto` modes of `loop-pickup` / `loop-handoff` /
`pr-review`. Everything machine-parsed lives here; change it in lockstep everywhere.

## Contents

- [Roles](#roles)
- [Editable plan reconciliation](#editable-plan-reconciliation)
- [Directory layout](#directory-layout)
- [Daemon and events](#daemon--events)
- [State schema](#statejson-schema-v2)
- [Repository bootstrap](#repository-registry-and-bootstrap)
- [Runner mechanics](#runner-spawn-how-the-orchestrator-launches-loop-runnersh)
- [Two-tier orchestration](#two-tier-orchestration-self-recycling-sub)
- [Runner contract](#statusjson-runner--orchestrator)
- [Model chains and invocations](#model-chains-loop-modelsconf)
- [Runner prompts](#runner-prompt-skeleton-orchestrator-generates-promptmd-per-attempt)
- [PR review phase pipeline](#pr-review-phase-pipeline)
- [Merge policy](#merge-policy-loop-mergesh-serialized--one-merge-at-a-time)
- [Escalation and HIL](#escalation-ladder-per-phase)
- [Backend mapping](#backend-mapping)
- [Crash-resume](#crash-resume)

## Roles

- **Orchestrator** — owns the plan **document** (sole `update_document` writer — parallel
  repushes are last-writer-wins), post-merge task status flips, PR creation/linking, all
  pushes to repository integration branches, scheduling, escalation, HIL. In the two-tier model this
  role is **split across two modes** (§ Two-tier orchestration): a THIN `supervise` main (the
  interactive Claude Code session the user talks to — preflight/init/lock, HIL answering,
  and review phases) and disposable headless `sub` instances that do the heavy scheduling/merge/
  escalation and self-recycle. A single-tier run (`supervise` doing everything itself) is
  still valid for a short effort.
- **Sub-orchestrator (SUB)** — a disposable headless orchestrator (`claude -p`,
  `CHAIN_ORCHESTRATE`) spawned by `loop-orchestrator.sh`, entered via `loop-execute resume`.
  Does the heavy orchestration under a self-measured token ceiling and recycles itself at a
  safe boundary (§ Two-tier orchestration). NEVER runs pr-review; NEVER `AskUserQuestion`
  (HIL → files only). Its final act is writing `sub/status.json`.
- **Merge-runner** — a detached runner (loop-runner.sh style) the SUB spawns on a
  `loop-merge.sh` exit-2 conflict: it resolves in the target repository integration worktree via
  `resolving-merge-conflicts` honoring both phases' *Done when*, then `loop-merge.sh
  --finish`, then writes `status.json`. Keeps the conflict diff out of the SUB's context
  (§ Merge policy — skeleton there).
- **Runner** — one headless agent process (`codex exec` or `claude -p`) working one phase
  attempt inside a lane worktree. Both hosts have the Kestral MCP (Claude: user plugin;
  Codex: `kestral@kestral-plugins` plugin) and the skills installed. A runner opens with
  `loop-pickup --auto` (claims its own task, loads context) and closes with
  `loop-handoff --auto` — **task-scoped ops only**: task status + progress comment via
  Kestral when **linked**, or a local `.loop/plan.md` copy marker update + `loop-plan.sh
  note` when **unlinked**. Runners NEVER: push, open PRs, call `update_document` or any
  plan-doc/PR-link operation, touch files outside their worktree, or ask the user. A
  runner's final act is writing `status.json`.
- **Adversarial review runner** — Astra 6 through Codex or Fable 5.1 through Claude Code.
  Each independently reviews a pinned PR head, saves a local report, and never posts to or
  mutates GitHub. Two run per round for three rounds.
- **Remediation coordinator** — Opus 5 through Claude Code after rounds 1 and 2. It reconciles
  the round's two reports through `/pr-review-fix-all`, which delegates accepted independent
  fix groups to Opus 5 subagents, then integrates, verifies, commits, and pushes.
- **Final review runner** — Opus 5 through Claude Code after round 3. It reconciles the latest
  two reports against current code and posts the pipeline's only GitHub review.
- **Scripts** — mechanism only (spawn/merge/notify/state; `loop-plan.sh` registers/pushes/
  notes the plan on the daemon — the local backend). They never decide; the orchestrator
  switches on their exit codes.

## Editable plan reconciliation

The plan is mutable until humans merge the PRs. Reconcile it at every safe scheduling and
review boundary. Phase numbers are stable identities used by state, notes, events, run dirs,
and linked tasks; document order is the displayed/executed order. Allocate the next unused
number and never renumber, reuse, move, or reopen a completed phase.

Every new-format executable plan ends in `[kind: pr-review]`:

- Pending last review: insert newly added work before it and extend its dependency closure.
- Completed last review: keep it in place, append new work after it, then append a new review.

Merge new phase entries into `state.phases` as `todo`; never replace existing entries.
Edits that conflict with a live runner wait for a safe boundary or raise HIL. Push the fully
reconciled document atomically to the daemon and, when linked, Kestral. Legacy no-kind plans
continue to use one synthetic terminal review.

## Directory layout

State lives in the launching coordinator checkout (add `.loop/` to
`.gitignore`):

```
.loop/
  state.json                 # orchestrator bookkeeping (loop-state.sh, atomic writes)
  plan.md                    # the plan document, beside state.json (register/push planText source)
  progress.md                # unlinked-mode progress stream (loop-plan.sh note / handoff --auto append)
  events.jsonl               # append-only journal: {ts, event, phase, repository?, detail}
  lock/                      # mkdir-lock; owner = run id (sessions have no stable pid);
                             # same owner re-locks freely, another owner needs --force
  runs/<phase-slug>-a<K>/    # one dir per attempt K of a phase
    prompt.md                # exact prompt sent
    transcript.jsonl         # full stream-json/JSONL (kept; mtime doubles as heartbeat)
    last.md                  # final assistant message
    status.json              # runner-written result (schema below)
    checkpoint.md            # observable worktree state if the 30m phase deadline fired
    stderr.log  verify.log   # wrapper-captured
    meta.json                # wrapper-written: engine, model, sessionId, exit, head shas
  runs/review-p<N>-<owner--repo>-a<1..9>/ # review phase N; see PR review phase pipeline
    report.md                     # reviewer/final report (a1,a2,a4,a5,a7,a8,a9)
    remediation-plan.md           # reconciled plan + disposition evidence (a3,a6)
  answers/<phase-slug>.md    # orchestrator guidance injected into a retry
  hil/<phase-slug>.md        # HIL request; answer arrives as hil/<phase-slug>.answer.md
  notes/<key>.md             # explicit phase number; legacy pr-review.* keys accepted
  sub/                       # two-tier self-recycling orchestrator runtime (§ Two-tier orchestration)
    transcript-<k>.jsonl     # SUB instance k's stream-json (mtime = heartbeat; occupancy source)
    status.json              # SUB → loop-orchestrator: recycle|review-ready|complete|fatal|blocked
    handoff.md               # single-consumption recycle handoff → renamed handoff.consumed-<k>.md
    current.pid              # live SUB pid (loop-orchestrator.sh writes; supervise polls it)
    saturation.json          # optional sidecar-watcher occupancy fallback (if stream-json buffers)
    REVIEW_READY             # explicit review barrier ready → supervise executes that phase
    PHASES_DONE              # legacy no-kind plan only: synthetic review ready
    control/                 # reserved: out-of-band control files (e.g. pause/abort)
```

Worktrees live outside the repo (ralph convention):

- Lane: `~/.loop/worktrees/<runId>/<owner--repo>/<lane-branch-slug>` on the phase's
  Suggested branch, cut from that repository's current integration tip. A lane repository
  hop removes the old worktree; reuse is same-repository consecutive phases only.
- Integration: `~/.loop/worktrees/<runId>/<owner--repo>/integration`. `loop-repo.sh
  prepare` fetches GitHub's default ref and creates it from the exact fetched SHA. All
  repository merges/pushes happen only here.

## Daemon & events

A perpetual **central daemon** (`LOOP_DAEMON_URL`, default `http://localhost:7717`) is the
authoritative, never-stale source for **UI status**; the `.loop/` files stay the
crash-resume **source of truth** for code + bookkeeping. A launchd LaunchAgent auto-starts it
on login (KeepAlive); `loop-emit.sh` (sourced by the four loop scripts) provides
`loop_ensure_daemon` as the fallback. Loops **register**, then push clean lifecycle events.
Emission is **best-effort** — a `curl` failure never fails the caller.
The daemon also reads size-capped `notes/*.md` content on each reconcile. NOTE badges are
file-derived but completion-aware: a numeric phase note is pending only while its phase is
not `done|merged`; legacy `pr-review.<owner--repo>` is pending only until that review
finishes. This makes missed best-effort file cleanup harmless.

Endpoints (POST bodies are JSON built injection-safely with `jq`):

- `POST /api/loops/:runId/register` `{ loopDir, planFile?, effort?, projectId?, integrationBranch?, startedAt?, planText? }` — server reads plan.md from `planFile` if `planText` omitted.
- `POST /api/loops/:runId/state` — the full state.json contents.
- `POST /api/loops/:runId/event` `{ event, phase?, repository?, detail?, ts?, outcome?, exitCode?, engine?, model?, prUrl?, tokens?, recycleIndex?, percent? }` — folded verbatim.
- `POST /api/loops/:runId/finish` accepts v2 `{ status?, finishedAt?, repositories:{slug:{prUrl?,review?}} }`; legacy scalar `prUrl/review` remains accepted. For explicit plans it is ignored until the terminal review phase is `done|merged`.
- `GET /api/loops/:runId/plan` → `{ runId, effort, status, integrationBranch, planText }` — how a fresh checkout fetches the plan with **no worktree needed**.
- `POST /api/loops/:runId/note` `{ key, markdown }` to write or `{ key, clear:true }` to delete a steering note; rejects archived loops.
- `GET /api/loops` (plural repository summaries) · `GET /events?runId=` ·
  `/api/loops/:runId/review?repository=<owner/repo>`; bare `/review` is the legacy alias ·
  `/api/loops/:runId/{snapshot,attempt/:slug/:k}` · `/api/health`.

A register-only record shows status **`planned`** (a plan on the daemon with no run yet):
`multiphase-plan` registers the plan (**`planned`**) before any run; the first state push or
event flips it to **`active`**.

Re-registering replaces `planText` without discarding phase overlays. If a finished record
receives a newly appended unfinished phase, it reopens as `active` under the same runId;
completed phase ranks remain monotone.

**Typed event vocabulary** (who emits what):

| event | emitted by | fields |
|-------|-----------|--------|
| `phase.attempt.start` | loop-runner.sh (after mkdir RUN_DIR) | phase, repository, engine, model |
| `phase.attempt.finish` | loop-runner.sh (EXIT trap) | phase, repository, outcome, exitCode, engine, model |
| `phase.merged` | loop-merge.sh (EXIT trap, rc==0) | phase, repository |
| `merge.conflict` | loop-merge.sh (conflict branch) | phase, repository, detail |
| `hil.raise` / `hil.resolve` | orchestrator (loop-state.sh log) | phase |
| `review.finish` | orchestrator | phase, repository, outcome, summary |
| `loop.finish` | orchestrator (loop-state.sh finish) | repositories |
| `sub.recycle` | loop-orchestrator.sh (on a SUB recycle) | tokens, recycleIndex |
| `sub.saturation` | loop-state.sh occupancy (periodic heartbeat) | tokens, percent |
| `progress.note` | loop-plan.sh note / loop-handoff --auto (unlinked) | phase, detail |

`progress.note` is the unlinked-mode progress narration — kept on the timeline, promotes
nothing in the lattice.

Unknown/legacy event names fall back to keyword matching, so today's free-form
`events.jsonl` still works. The two `sub.*` events are **loop-level** (no `phase`); the daemon
folds them idempotently — `subRecycles = max(recycleIndex)`, `occupancy = last {tokens, percent}`
— so re-folding `events.jsonl` on every reconcile never double-counts. They update loop state
only; they are NOT appended to the on-disk `events.jsonl` (heartbeats would flood the timeline),
so they ride the live daemon POST exclusively.

**Monotone promotion lattice.** The daemon materializes each loop through a monotone rank
`todo < claimed < running < done < merged`; a `phase.attempt.finish{outcome:done, exit 0}`
promotes a phase to **done even if state.json bookkeeping lags** (fixes "done phases stuck at
running/todo"). Ranks never regress — a phase never un-completes in the UI (v1).

## state.json schema v2

`runId` == the plan's `planId` (loop-execute adopts it). Crash-resume reads exactly these
fields — keep the shape:

```json
{
  "runId": "loop-<effort-slug>-<date>-<HHMMSS|rand>",
  "effort": "...", "projectId": "...", "workContextId": "...",
  "repositories": {
    "owner/api": {
      "sourceRoot": "<mapped checkout>", "defaultBranch": "main", "baseSha": "<sha>",
      "integrationBranch": "feat/<effort-slug>",
      "integrationWorktree": "<abs path>", "prUrl": null,
      "reviewPipeline": { "phase": 7, "stage": "round1",
                          "headSha": "<sha for current review round>" },
      "review": { "outcome": "done|question|blocked", "summary": "...",
                  "reportPath": "runs/review-p7-owner--api-a9/report.md", "commentUrl": "..." }
    }
  },
  "phases": {
    "3": { "kind": "work|pr-review", "slug": "<task-slug>", "taskId": "...", "lane": "A", "repository": "owner/api|all",
           "branch": "feat/api-client-token-refresh", "worktree": "<abs path or null>",
           "status": "todo|claimed|running|merged|blocked|done",
           "attempt": 2, "runDir": ".loop/runs/<phase-slug>-a2",
           "pid": 4242, "sessionId": "<from meta.json>", "questionRounds": 0 }
  },
  "linkedPrTasks": []
}
```

Legacy scalar `integrationBranch/integrationWorktree/prUrl/review` state migrates to the
synthetic `primary` repository. Singular daemon projections remain for legacy consumers.
Global scheduling follows the DAG/lane/concurrency cap; merge serialization is global, but
worktree, verify, PR, and review operations always target the phase repository.

## Repository registry and bootstrap

`~/.loop/repos.json` is an atomic JSON map from GitHub `owner/repo` to local checkout root.
It is reusable machine state, never plan content.

```sh
loop-repo.sh map <owner/repo> <checkout-root>
loop-repo.sh get <owner/repo>
loop-repo.sh list
loop-repo.sh unmap <owner/repo>
loop-repo.sh check --repo <owner/repo> --integration-branch <branch>
loop-repo.sh prepare --repo <owner/repo> --run-id <runId> --integration-branch <branch>
```

`check` validates mapping, SSH/HTTPS GitHub origin, GitHub default branch, and branch/worktree
collisions. `prepare` fetches the remote default ref, records its exact SHA, and creates the
run/repository-scoped integration worktree from that SHA.

## Runner spawn (how the orchestrator launches loop-runner.sh)

Detached, so runners survive orchestrator death:

```sh
nohup ~/dotfile/loop-runner.sh <args> > <runDir>/spawn.log 2>&1 &
echo $! > <runDir>/pid; disown
```

There is no completion notification — the orchestrator monitors by polling: `meta.json`
exists → the attempt ended (read `status.json` + runner exit from spawn.log tail);
otherwise `transcript.jsonl` mtime is the heartbeat. Phase attempts have their own 30-minute
watchdog and produce an exit-10 checkpoint; do not kill a quiet phase before that deadline.
If a runner is still live without `meta.json` 35 minutes after its attempt started, its
watchdog failed: kill the pid tree and treat as exit 124.

## Two-tier orchestration (self-recycling SUB)

Heavy orchestration accumulates context (diff-skimming, conflict resolution, runner Q&A). To
keep the *interactive* chat under a hard ~200k ceiling, `loop-execute` runs in one of two
**modes**, and the heavy work self-recycles:

- **`supervise`** (new default for a hands-off run) — the interactive session. THIN:
  preflight/init/lock, then spawn ONE detached `loop-orchestrator.sh` and poll **compact
  local** state (`loop-state.sh get '.phases|map(.status)'`, not the fat daemon snapshot) for
  a 2-3 line progress read. It answers HIL from the on-disk brief and, on
  `sub/REVIEW_READY`, executes that explicit review phase (bounded parallel repository
  review/remediation runners), then either respawns scheduling for later work or finishes when
  the review is terminal. It spawns only the orchestrator and review/remediation runners;
  it is designed never to need recycling (if it ever did, `state.json` is its backstop — a
  main auto-summary cannot lose the loop).
- **headless `sub`** — a disposable SUB instance entered via `loop-execute resume`. Does the
  heavy orchestration but NEVER runs pr-review and NEVER `AskUserQuestion`.

`loop-orchestrator.sh` (detached, spawned once by `supervise`) is a **dead-simple sequential
while-loop**, not a concurrent daemon. It runs SUB instance *k*, waits for it to exit, reads
`sub/status.json`, and:

- `recycle` → emit `sub.recycle`, `k++`, respawn SUB *k+1* with a `resume` prompt.
- `review-ready` → `touch sub/REVIEW_READY`, notify, break so supervise runs the barrier.
- `complete` → `touch sub/PHASES_DONE`, notify, break (legacy no-kind plans only).
- `fatal` / `blocked` → `loop-notify.sh`, break (supervise surfaces it).
- no valid `sub/status.json` → crash → respawn (`resume`), capped at `LOOP_ORCH_MAX_RESPAWN`
  consecutive crashes, then notify + exit.
- SUB transcript mtime older than `LOOP_ORCH_STALL_SEC` → kill the SUB pid tree
  (`descendants()`), treat as a crash-respawn.

**Why a sequential respawner, not "the SUB spawns its own successor":** instances run
sequentially, so the predecessor process has already exited before the successor starts. The
reentrant lock (`owner = runId`) gives **zero** mutual exclusion for the same run id, so a
predecessor↔successor overlap would be a silent double-orchestration; sequential respawn makes
that overlap **impossible by construction**. The SUB decides *when* to recycle (self-paced);
the loop only decides *that* it respawns. Phase runners (`loop-runner.sh`, detached) survive
every recycle untouched — `resume` re-attaches to their live pids and never double-spawns.

### Occupancy self-measurement

A model cannot introspect its context %, but it can read a file. `claude -p --output-format
stream-json --verbose` writes each turn's `assistant` event with `.message.usage` to the SUB's
redirected transcript (`sub/transcript-<k>.jsonl`). Each tick the SUB runs:

```sh
loop-state.sh occupancy --transcript .loop/sub/transcript-<k>.jsonl [--window <n>]
```

which takes the **last** usage-bearing `assistant` event and sums (verbatim jq):

```sh
jq -R 'fromjson? | select((.type == "assistant") and (.message.usage != null))
  | (.message.usage | (.input_tokens // 0) + (.cache_read_input_tokens // 0) + (.cache_creation_input_tokens // 0))' \
  "$TRANSCRIPT" | tail -n1
```

= current context occupancy (lag ≤ ~1 turn, absorbed by the 150k→200k margin). `fromjson?`
skips a malformed/partial last line (a mid-write transcript); `tail -n1` is the ≥1-productive-
tick guard (no assistant turn yet → empty → `0`). With `--window`
it also prints a percent (`<tokens> <percent>`); it **exits 10** when the sum ≥
`LOOP_ORCH_RECYCLE_TOKENS` (0 otherwise — the recycle signal is the exit code, NOT stdout), and
best-effort emits a `sub.saturation` event to the daemon. It requires ≥1 productive tick (a
usage-bearing event) — a fresh SUB with no assistant turn yet reports `0` and never fires. Spawn
the SUB through `stdbuf`/a pty so stream-json flushes per line; a sidecar `tail` writing
`sub/saturation.json` is the documented fallback if buffering proves too coarse (confirm the
per-turn flush during the forced-recycle rollout test; enable the sidecar if it is too coarse).

### Recycle trigger + handoff

**Trigger (locked with the user):** absolute tokens. Soft handoff at `LOOP_ORCH_RECYCLE_TOKENS`
(~150k); target ceiling `LOOP_ORCH_CEILING_TOKENS` (~200k, informational). **Gated on phase
quiescence** — if a phase op is mid-flight (a runner mid-attempt with no written `status.json`,
or a merge in progress), keep going, even past 200k, and recycle **asap** once every lane is at
a persisted checkpoint. **No token hard-kill.**

**Handoff = single-consumption.** At a safe boundary the SUB flushes durable state (state.json,
Kestral, git are already externalized), writes `sub/handoff.md`, and exits with `sub/status.json
{outcome:"recycle"}`. The successor reads `sub/handoff.md` **exactly once** at startup, then it
is archived to `sub/handoff.consumed-<k>.md`; `plan.md` + `state.json` stay freely re-readable.
This is a purpose-built recycle handoff — **NOT** the `loop-handoff` skill (that one is
plan-doc/task-status scoped and marks phases *done*: wrong semantics for a mid-run recycle).
`handoff.md` carries: open exit-10 question rounds + their `sessionId`s; pending merge-runner
promotions (runDir + lane-branch awaiting a `phase.merged`); per-lane notes; and why-stopped.
Everything else the successor rebuilds by reconciling **git > Kestral > state**.

### SUB `status.json` (sub-orchestrator → loop-orchestrator.sh)

The SUB's final act (like a runner's) is writing `sub/status.json`. It **extends** the runner
schema with orchestrator-role outcomes:

```json
{
  "outcome": "recycle | review-ready | complete | fatal | blocked",
  "summary": "1-3 lines: why it stopped / what is done",
  "tokens": 152341,
  "recycleIndex": 2
}
```

- `recycle` — occupancy hit the soft threshold at a safe boundary; `sub/handoff.md` written.
  `tokens`/`recycleIndex` populate the `sub.recycle` event.
- `review-ready` — the next ready document phase is `[kind: pr-review]`; the loop touches
  `sub/REVIEW_READY` and stops so `supervise` executes it.
- `complete` — legacy plan work is complete and its synthetic review is ready. The loop
  touches `sub/PHASES_DONE` and stops.
- `fatal` — unrecoverable orchestration error (bad config, the lock was taken by another owner).
- `blocked` — a HIL is open that only the human can answer AND no other lane can progress. (An
  open HIL with other lanes still running is NOT `blocked` — the SUB keeps going.)

A SUB that exits without a valid `sub/status.json` is a crash → `loop-orchestrator.sh` respawns
with `resume` (capped at `LOOP_ORCH_MAX_RESPAWN`).

## status.json (runner → orchestrator)

The runner's final act is writing `$RUN_DIR/status.json` (the absolute `RUN_DIR` path is
given in its prompt):

```json
{
  "outcome": "done | question | blocked",
  "summary": "1-3 lines: what happened / what was built",
  "question": "only when outcome=question — ONE concrete question, with options if useful",
  "checkpoint": "optional true for a 30-minute phase checkpoint",
  "details": "only when outcome=blocked — what was found, what was tried"
}
```

- `done` — the phase's *Done when* holds, work is committed, local verify passed.
- `question` — a decision is needed (ambiguity, unexpected finding with options). Stop
  immediately after writing; the orchestrator answers and resumes the session. A
  `checkpoint:true` question also occurs when a phase reaches its 30-minute work limit.
  The runner adds the deadline to the prompt; if the agent misses it, the wrapper stops the
  process and writes a fallback `status.json` plus `checkpoint.md` from worktree state.
- `blocked` — cannot proceed and no question would unblock (missing dep, broken base,
  contradiction in the plan).

No sentinel tags in prose. A process that exits without a valid `status.json` is a crash.

## loop-runner.sh exit codes (orchestrator switches on these)

| Code | Meaning | Orchestrator reaction |
|------|---------|----------------------|
| 0    | `done`, status.json valid, `--verify-cmd` passed (if given) | merge gate |
| 10   | `question`, including a phase checkpoint | answer + resume; checkpoints do not count toward the three decision-question rounds |
| 12   | claimed `done` but verify failed (verify.log has output) | retry with failure context; 2nd time → L3 |
| 20   | `blocked` | escalation L3 |
| 40   | whole model chain exhausted on API errors | backoff (60s·2ⁿ, cap 15m), retry from top of chain — max 3 per phase, then L3; two or more lanes hitting 40 → pause all scheduling + notify |
| 50   | crash: process exited without valid status.json | count as failed attempt → L3 after 2 |
| 124  | review watchdog timeout or failed phase watchdog | one retry, then L3 |
| 1    | usage/infra error (bad args, missing tools) | fix invocation, not the phase |

Every attempt restarts at the **top** of its chain (no sticky fallback). `meta.json` records
`headBefore`/`headAfter`. For phase implementation, exit 0 with no new commit is a stall.
Adversarial/final reviews are read-only; remediation may also keep the same head only when its
plan proves there were no accepted findings. Those cases are not stalls.

## Model chains (loop-models.conf)

```sh
CHAIN_TASK=("codex:gpt-6-sol" "claude:claude-sonnet-5")
CHAIN_ESCALATE=("claude:fable+opus")
CHAIN_REVIEW_FABLE=("claude:claude-fable-5-1")
CHAIN_REVIEW_ASTRA=("codex:gpt-6-astra")
CHAIN_REVIEW_FIX=("claude:claude-opus-5")
CHAIN_REVIEW_FINAL=("claude:claude-opus-5")
CHAIN_ORCHESTRATE=("claude:claude-opus-5-5")
CODEX_EXTRA_ARGS=(-c 'model_reasoning_effort="high"')
CLAUDE_EXTRA_ARGS=(--effort high)
LOOP_BUDGET_USD=15        # per attempt, claude legs only (codex has no budget flag)
LOOP_CHECKPOINT_SEC=1800  # hard ceiling across all legs/retries of one phase attempt
LOOP_TIMEOUT_TASK=1800    # 30m
LOOP_TIMEOUT_ESCALATE=1800
LOOP_TIMEOUT_REVIEW=1800
LOOP_TIMEOUT_REMEDIATE=1800
LOOP_MAX_PARALLEL=3
LOOP_ORCH_CTX_WINDOW=1000000
```

Leg grammar: `engine:model[+fallback[,fallback2]]`. The `+` list maps to Claude's native
`--fallback-model` (comma-separated; CLI retries the primary each turn) — so intra-Claude
fallback is one leg. Codex→Claude hops are the wrapper's job. Task/escalation legs may use
Claude aliases; review and orchestration roles use full model names to pin Fable 5.1,
Astra 6, Opus 5, and Opus 5.5. Each review chain contains exactly one leg: a failed stage stays failed
instead of silently becoming a duplicate of another model. The `review-fix` prompt invokes
`/pr-review-fix-all`; that skill owns Opus 5 subagent delegation.

Failure classing per leg (from structured error events first — `.is_error` result events in
Claude stream-json, `error` events in Codex JSONL — stderr regex last):

- transient (`5xx|overloaded|ECONNRESET|ETIMEDOUT|empty result`) → retry same leg, 3
  attempts total, backoff 10/30s;
- rate/usage (`429|rate.?limit|usage limit|quota`) → advance to next leg immediately;
- other nonzero → one retry, then next leg.

For every chain, `loop-runner.sh` caps even an explicit `--timeout` at
`LOOP_CHECKPOINT_SEC`. The deadline covers the entire attempt, including retries, backoff,
and fallback models. Each review stage has its own attempt and checkpoint.

## Engine invocations (inside loop-runner.sh)

Codex leg:

```sh
codex exec --json -C "$WORKTREE" -m "$MODEL" "${CODEX_EXTRA_ARGS[@]}" \
  --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check \
  -o "$RUN_DIR/last.md" - < "$RUN_DIR/prompt.md" > "$RUN_DIR/transcript.jsonl" 2> "$RUN_DIR/stderr.log"
```

Claude leg (run with `command claude` from a non-login shell to avoid zsh init noise):

```sh
command claude -p --model "$MODEL" ${FALLBACK:+--fallback-model "$FALLBACK"} \
  "${CLAUDE_EXTRA_ARGS[@]}" \
  --output-format stream-json --verbose \
  --allow-dangerously-skip-permissions --permission-mode bypassPermissions \
  --max-budget-usd "$LOOP_BUDGET_USD" \
  < "$RUN_DIR/prompt.md" > "$RUN_DIR/transcript.jsonl" 2> "$RUN_DIR/stderr.log"
```

(cwd = the worktree.) Session ids are parsed from the transcript (`.session_id` on Claude's
init event; `thread.started`/session field on Codex) into `meta.json`.

**Q&A resume** (the only channel to "answer a runner's question"): the orchestrator composes
an answer, then `loop-runner.sh --resume <sessionId> --engine <same engine>` with the answer
as the prompt — **in a fresh attempt dir** (`-a<K+1>`, answer as its `prompt.md`), never the
original run dir (rerunning there would clobber the transcript and session id). If resume
fails, fall back to a fresh attempt with the answer prepended via `answers/<phase-slug>.md`.

## Runner prompt skeleton (orchestrator generates prompt.md per attempt)

```
You are a loop-engineering task runner: fully non-interactive, nobody reads your output
live. You are in worktree <path> on branch <branch> (cut from <integration>). RUN_DIR is
<abs path>.

FIRST: run `$loop-pickup --auto <project> phase:<N>` (Claude: `/loop-pickup`) — it
claims your task and loads context. If it returns `REFUSED: <reason>`, write
RUN_DIR/status.json with outcome "blocked" and the reason as details, and stop.

RULES: never push; never open PRs; Kestral only through the pickup/handoff auto skills
(never update the plan document or link PRs); never ask the user in prose; stay inside
this worktree. Commit per logical step with descriptive messages. Follow the repo's
CLAUDE.md conventions (TDD where it applies).

PHASE (from the shared plan):
<verbatim phase block: title, Depends on, Touches, Done when, Verify, Notes>

CONTEXT: <goal + key decisions from plan header; prior-attempt failure summary;
steering notes: notes/<phaseNumber>.md — fold verbatim on EVERY attempt, including first;
answers/<phase>.md retry guidance; hil answer — whichever exist>

Read steering notes on every attempt, including the first. Read answers/ only on retries.

The runner appends the actual 30-minute deadline to this prompt at launch. At about 25
minutes, prepare a status. If unfinished, write `status.json` with outcome "question",
`checkpoint:true`, concise progress, and one question for the orchestrator, then exit.
The orchestrator answers in a fresh attempt without stopping other lanes.

WHEN FINISHED (Done when holds and `<verify cmd>` passes locally): run
`$loop-handoff --auto phase:<N> status:in-progress lane:<X> engine:<engine>` (comment:
implementation complete, awaiting merge — "done" is the orchestrator's post-merge call),
then write RUN_DIR/status.json exactly per this schema <schema> and end the session. If
you must stop early, skip the handoff and write status.json with outcome question/blocked.
```

## PR review phase pipeline

Every explicit `[kind: pr-review]` phase is a nine-run pipeline per repository. Stages within a
repository are strictly ordered; different repositories may advance concurrently under
`LOOP_MAX_PARALLEL`. Run artifacts use `review-p<N>-<owner--repo>-a<1..9>`, where `<N>` is the
review phase number.

| Run | Stage | Chain | Host/model | GitHub mutation |
|-----|-------|-------|------------|-----------------|
| `a1` | round 1 adversary | `review-astra` | Codex / Astra 6 | none |
| `a2` | round 1 adversary | `review-fable` | Claude Code / Fable 5.1 | none |
| `a3` | reconcile + fix | `review-fix` | Claude Code / Opus 5 + Opus 5 fixers | commit + push only |
| `a4` | round 2 adversary | `review-astra` | Codex / Astra 6 | none |
| `a5` | round 2 adversary | `review-fable` | Claude Code / Fable 5.1 | none |
| `a6` | reconcile + fix | `review-fix` | Claude Code / Opus 5 + Opus 5 fixers | commit + push only |
| `a7` | round 3 adversary | `review-astra` | Codex / Astra 6 | none |
| `a8` | round 3 adversary | `review-fable` | Claude Code / Fable 5.1 | none |
| `a9` | reconcile + final review | `review-final` | Claude Code / Opus 5 | one PR review |

Before each adversarial pair, fetch the PR and capture its current head SHA into
`repositories[slug].reviewPipeline.headSha`. Both prompts receive that exact SHA, the PR URL,
project instructions, verify commands, and the repository steering note. Launch the pair
concurrently when capacity allows. Each prompt says:

```
Perform the full pr-review inspection adversarially against <PR URL> at exactly <head SHA>.
Use its checkout/run/checklist methodology, but override its posting step: do not call any
GitHub mutation endpoint, do not post a review/comment, and do not push. Review independently;
do not read the paired review run, earlier review runs, their reports/transcripts/status files,
or existing GitHub reviews/comments. Save the complete findings to RUN_DIR/report.md, then
write RUN_DIR/status.json with {"outcome":"done|question|blocked","summary":"..."}.
```

Round 2 is local-only just like rounds 1 and 3. A reviewer failure blocks the repository after
its sibling finishes; never substitute a model. An exit-10 `checkpoint:true` is a timed status
request: the supervisor answers and resumes the same stage at the same PR head while other
repositories continue. Do not advance the stage until it finishes.

### Opus remediation (`a3`, `a6`)

Run `review-fix` in the repository integration worktree after both reports exist. Its prompt
passes only that round's reports plus the remediation execution inputs and says:

```
Invoke `/pr-review-fix-all` with:
- astra-report: <absolute a1-or-a4 report.md>
- fable-report: <absolute a2-or-a5 report.md>
- run-dir: <absolute a3-or-a6 RUN_DIR>
- worktree: <absolute repository integration worktree>
- branch: <integration branch>
- remote: origin
- verify: <repository verify command, verbatim>

Follow that skill through its final status.json. Do not implement remediation outside it.
```

The skill reconciles all findings into `remediation-plan.md`, groups non-overlapping work,
spawns at most three `pr-review-fixer` subagents pinned to Opus 5, and serializes overlaps or
dependencies. Subagents edit and test only; the coordinator owns integration, verification,
commits, and fast-forward push. It does not create an empty commit when no findings are accepted.
Confirm the remote PR head moved
to the pushed SHA before starting the next round. Any unresolved accepted finding or failed
verification yields `blocked` and stops this repository pipeline.

### Final Opus review (`a9`)

Run `review-final` only after `a7` and `a8` completed against the same current head. It reads
those two reports plus current code, independently verifies and deduplicates their findings,
and saves the reconciled review in `report.md`. Earlier-round reports are stale remediation
history and are not inputs to the final verdict.

Post exactly one review through `POST /repos/<owner>/<repo>/pulls/<n>/reviews`, pinning
`commit_id` to the reviewed head and selecting `event` from unresolved validated severities:

- one or more `[blocker]` findings → `REQUEST_CHANGES`;
- otherwise one or more `[major]` findings → `COMMENT`;
- otherwise → `APPROVE` (minor/nit/style findings may remain).

Put findings in inline review comments where the diff permits and keep the overall body
concise. End the body with `:)` and include
`<!-- loop-review:<runId>:p<N>:<owner--repo>:final -->`. Before posting, query existing PR reviews
for that marker; if it exists, reuse its URL instead of posting again. Save `html_url` as
`status.json.commentUrl`. Map the GitHub event to repository outcome:
`REQUEST_CHANGES → blocked`, `COMMENT → question`, `APPROVE → done`. Promote `a9/report.md`,
the final summary, and comment URL to `repositories[slug].review`, then emit one aggregate
`review.finish` event.

Persist `reviewPipeline.phase` plus the next stage to execute, initially `round1`; initialize a
fresh pipeline when the ready review phase number changes, then advance through `fix1`, `round2`,
`fix2`, `round3`, `final`, and `done` only after each prior stage succeeds. Crash-resume trusts
phase-scoped completed run artifacts plus git/GitHub evidence over a stale stage field; it never
reruns a live pid, repeats a completed remediation push, or duplicates that phase's final review.

## Merge policy (loop-merge.sh, serialized — one merge at a time)

`loop-merge.sh --worktree <repo-int-wt> --lane-branch <b> --repository <owner/repo> --verify-cmd '<repository verify>'`
(the script refuses to push unverified without an explicit `--no-verify`):

- **0** — merged, verified, pushed (also idempotent `{"already":true}` for a re-merge).
- **2** — textual conflict, merge left in progress, `{"conflicts":[files]}` on stdout.
  Resolution honors both phases' *Done when* with `resolving-merge-conflicts`, then
  `loop-merge.sh --worktree <repo-int-wt> --finish --repository <owner/repo> --verify-cmd '<repository verify>'`; not confident →
  `--abort` + HIL. **Who resolves depends on the mode:** a single-tier `supervise` does it
  inline; a **`sub`** spawns a **detached merge-runner** (skeleton below) so the conflict diff
  never enters the SUB's context — the SUB promotes on the next tick when `loop-merge`'s EXIT
  trap fires `phase.merged`.
- **4** — merge committed but Verify failed (semantic conflict). Orchestrator reverts the
  merge commit (`git revert -m1 HEAD`) and escalates L3.
- **3** — lane branch missing: reconcile from git (already merged + deleted → treat as
  done; otherwise escalate). Occurs after crash-resume repairs.
- **1** — merge failed to start (dirty integration worktree) — clean it, don't "resolve".

**First successful merge per repository → open its draft PR** (`gh pr create --draft` from
that integration branch); later repository merges just push. Only the orchestrator pushes
integration branches, with two orchestrator-delegate exceptions: the **merge-runner** finishes
a serialized merge, and the **remediation coordinator** pushes its verified review fixes.
Neither is a phase runner. Plain `git push` is fast-forward only, never force. Lane branches
never get pushed. Cleanup order: switch or
remove the lane worktree **first**, then `git branch -d <lane-branch>` (a branch checked out in
a worktree can't be deleted); a lane continuing to its next phase reuses its worktree via
`git -C <wt> switch -c <next-branch> <integration>`. Lanes launched later cut from the new
tip; already-running lanes pick up siblings' work at their own merge time.

### Merge-runner prompt skeleton (SUB generates on a `loop-merge` exit 2)

Spawned like any runner — detached, via `loop-runner.sh`, so the conflict diff lives in the
runner's context, never the SUB's. It runs on the **integration worktree** (not a lane), uses
`--chain escalate` (Fable+Opus — coherent single-model resolution), and carries **no
`--verify-cmd`** (loop-runner then gates only on the runner's `status.json`; the real verify is
`loop-merge --finish`'s own `--verify-cmd`, so verify runs exactly once):

```sh
nohup ~/dotfile/loop-runner.sh --chain escalate \
  --worktree <repo-int-wt> --run-dir <runs/merge-<phase-slug>-a1> \
  --prompt-file <runs/merge-<phase-slug>-a1/prompt.md> \
  --run-id <runId> --phase <N> --repository <owner/repo> > <runDir>/spawn.log 2>&1 &
echo $! > <runDir>/pid; disown
```

Prompt.md skeleton:

```
You are a loop-engineering MERGE runner: fully non-interactive. You are in the INTEGRATION
worktree <repo-int-wt> on branch <integration>, mid-merge of lane <lane-branch> (phase <N>) — a
textual conflict is in progress (`git status` shows unmerged paths). RUN_DIR is <abs path>.

TASK: resolve the conflict with the `resolving-merge-conflicts` skill, honoring BOTH phases'
*Done when* (the merging phase <N> and whatever is already on the integration branch — do not
regress either). Never widen scope beyond conflict resolution. Then finish + verify + push:

  ~/dotfile/loop-merge.sh --worktree <repo-int-wt> --finish --repository <owner/repo> \
    --verify-cmd '<repository verify>' --run-id <runId> --phase <N>

- exit 0 → the merge is committed, verified, pushed; `phase.merged` already fired.
- exit 2 → conflicts remain unresolved: keep resolving, or if not confident, abort
  (`git -C <repo-int-wt> merge --abort`) and stop with a question.
- exit 4 → committed but Verify failed (semantic conflict): stop with outcome "blocked".

RULES: never touch files outside the integration worktree; never open PRs or update the plan
doc; commit only via `loop-merge --finish`. Your final act is writing RUN_DIR/status.json:
outcome "done" (merged+verified+pushed), "question" (not confident — needs HIL), or "blocked"
(semantic conflict / verify failed). loop-runner spawned you with no --verify-cmd, so its exit
0 rides on your status.json alone.
```

## Escalation ladder (per phase)

| L | Trigger | Action | Cap |
|---|---------|--------|-----|
| L0 | transient API error (5xx/overload/network) | runner retries same leg, backoff | 3/leg |
| L1 | rate/usage limit | next chain leg (runner-internal) | chain length |
| L2 | `question`, or 1st verify-fail/stall | orchestrator answers from plan+code, resumes session (or retries with `answers/` context) | 3 rounds |
| L3 | `blocked`, rounds exhausted, 2nd verify-fail, crash ×2 | fresh attempt on `CHAIN_ESCALATE` (Fable takeover) with distilled post-mortem | 1 |
| L4 | L3 failed, unresolvable conflict, claim 409, plan contradiction | HIL pause (below) — only this lane pauses | — |

## HIL pause

1. Write `hil/<phase-slug>.md`: context, attempt table (engine → outcome), ONE blocking
   question, options A/B/C + recommendation, "reply in the orchestrator chat or write
   `hil/<phase-slug>.answer.md`".
2. Post the same content as a comment on the phase's Kestral task; flip the phase to
   `[status: blocked]` and repush the plan doc.
3. `loop-notify.sh --level question --run-id <runId> --event hil.raise --title "HIL: <phase>" --body "<question>"`.
4. Ask in-session (AskUserQuestion / plain question). Other lanes keep running. On answer:
   phase back to ready with the answer injected; status back to `in-progress`.

## Backend mapping

Two backends. The **central daemon is always the base**; **Kestral is opt-in (linked mode)**
— the default is local-only. Either way the split holds: **runners** own task-scoped ops via
the auto skills; the **orchestrator** owns the plan document and everything cross-task. Each
operation has an unlinked (local) and a linked (Kestral) form:

| Operation | Unlinked (local) | Linked (Kestral) |
|-----------|------------------|-------------------|
| Claim (runner, via `loop-pickup --auto`) | orchestrator pre-creates the lane branch; the phase flips to `claimed` in state.json (`loop-state.sh`) + a `[status: claimed]` marker in `.loop/plan.md`; lane collisions are caught by the branch/worktree conflict-check, not a task claim. | `claim_task_and_branch { taskId, branchName: <lane branch> }` on the pre-created branch — re-claiming your own branch is a no-op; a 409 means someone else owns it → `REFUSED` → status.json `blocked` → HIL, never steal. The claim performs no git action. |
| Runner handoff (via `loop-handoff --auto ... status:in-progress`) | `loop-plan.sh note --plan-id <runId> --phase <N> --body <progress>` (a `progress.note` event) + update the `[status: ...]` marker in the local `.loop/plan.md` copy; **task-scoped only** — no plan-doc rewrite, no PR links. | task progress comment + status kept task-scoped; **never** `update_document`, never PR links. |
| Post-merge status (orchestrator) | flip `[status: done]` in `.loop/plan.md` + append Progress log + `loop-plan.sh push` (re-register/overwrite the plan on the daemon) + a `progress.note`. | `update_task_status` (statusKey via `list_statuses`) + `post_progress_comment` (2–4 conversational lines, noting lane + engine) + flip `[status: done]` in the plan doc + append Progress log + `update_document`. |
| PR link (orchestrator, at effort completion) | record each `repositories[slug].prUrl`; no per-task link. | `link_pr_to_task` once per phase task using only its repository PR (dedup via state). |
| Completion (orchestrator) | one `loop-state.sh finish` with plural repository reviews after every staged pipeline reaches its final Opus review; plan remains `integrating`. | statuses → awaiting review; plan `integrating`; `trigger_brain_build`. |

"done" in loop mode = merged into the phase repository's integration branch. Humans merge
one PR per repository.

## Crash-resume

`loop-execute resume`: re-acquire the lock with the run id from state.json (a different
owner means another orchestrator claims this effort — stop and ask before `--force`), then
reconcile the sources — **git > plan/daemon > state.json (+ Kestral when linked)** (git is
truth for code, the plan/daemon for claims + phase status, state for attempt bookkeeping):

- attempt marked running: pid alive → re-attach (watch transcript mtime; stale >25m → kill,
  treat as 124); pid dead + status.json present → process it normally; pid dead + none →
  failed attempt.
- `MERGE_HEAD` in any repository integration worktree → finish or abort before anything else.
- Phase `done` in the plan/daemon (or Kestral when linked) but lane branch not merged (or
  vice versa) → repair from git.
- Review pipeline stage: live pid → re-attach; completed `status.json` → advance; remediation
  commit already on the remote integration branch → do not repeat it; final GitHub review marker
  already present → record its URL and mark `done` without posting again.
- Runners are spawned detached, so they survive orchestrator death; never double-spawn a
  phase whose run dir has a live pid.
