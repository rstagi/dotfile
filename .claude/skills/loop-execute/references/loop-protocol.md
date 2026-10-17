# Loop Protocol

The shared contract between the `loop-execute` orchestrator skill, the `loop-*.sh` scripts
(`~/dotfile/loop-runner.sh`, `loop-merge.sh`, `loop-notify.sh`, `loop-state.sh`,
`loop-plan.sh`, `loop-repo.sh`, `loop-review.sh`, `loop-ar.sh`, `loop-models.conf`), and the `--auto` modes of `loop-pickup` / `loop-handoff` /
`pr-review`. Everything machine-parsed lives here; change it in lockstep everywhere.

## Contents

- [Roles](#roles)
- [Editable plan reconciliation](#editable-plan-reconciliation)
- [Milestones](#milestones)
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

- **Orchestrator** — owns the plan **document** (sole `loop-plan.sh push` writer — parallel
  repushes are last-writer-wins), post-merge phase status flips, PR creation, all
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
  attempt inside a lane worktree. Both hosts have the skills installed. A runner opens with
  `loop-pickup --auto` (claims its own phase, loads context) and closes with
  `loop-handoff --auto` — **phase-scoped ops only**: a local `.loop/plan.md` copy marker
  update + `loop-plan.sh note`. Runners NEVER: push, open PRs, push the plan document,
  touch files outside their worktree, or ask the user. A
  runner's final act is writing `status.json`.
- **Reviewer R** — a fresh read-only runner producing numbered findings and a verdict for
  a frozen artifact. R also handles `respond`, with the prior exchange inlined in its prompt.
- **Critic C** — a fresh read-only runner from a different provider auditing R's review
  against the artifact and codebase. C references finding IDs and grounds disagreement in code.
- **Third provider T** — the provider outside R/C for this outer round. Its `confirm` run
  uses the critic prompt to audit an initially clean review; a veto makes T the active critic
  for the rest of that round's inner loop. R/C/T rotate (§ PR review phase pipeline).
- **Remediation coordinator** — Opus through Claude Code, only between outer rounds and
  only when the fix rule requires it. Through `/pr-review-fix-all` it verifies the converged
  review and disputed items, delegates independent fix groups to Opus subagents, then
  integrates, verifies, commits, and fast-forward pushes. It alone edits the PR artifact.
- **Final review runner** — Opus through Claude Code after the terminal outer round or a
  clean early stop. It verifies that round's converged review and disputed items against
  current code, then posts the pipeline's only GitHub review.
- **Scripts** — mechanism only (spawn/merge/notify/state; `loop-plan.sh` registers/pushes/
  notes the plan on the daemon — the local backend). They never decide; the orchestrator
  switches on their exit codes.

## Editable plan reconciliation

The plan is mutable until humans merge the PRs. Reconcile it at every safe scheduling and
review boundary. Phase numbers are stable identities used by state, notes, events, run dirs,
and the plan; document order is the displayed/executed order. Allocate the next unused
number and never renumber, reuse, move, or reopen a completed phase.

Every new-format executable plan ends in `[kind: pr-review]`:

- Pending last review: insert newly added work before it and extend its dependency closure.
- Completed last review: keep it in place, append new work after it, then append a new review.

Merge new phase entries into `state.phases` as `todo`; never replace existing entries.
Edits that conflict with a live runner wait for a safe boundary or raise HIL. Push the fully
reconciled document atomically to the daemon. Legacy no-kind plans
continue to use one synthetic terminal review.

## Milestones

A plan may carry several milestones: `## Milestone N — <outcome>` sections replace `## Phases`
(a plain `## Phases` plan is one implicit milestone 1 — everything below is then inert and the
single-milestone path is unchanged). Phase numbers stay global and stable. Each milestone ships
as its own PR per participating repository (the repositories of its work phases) and, unless
final, ends with a `[kind: pr-review]` `[gate: merge]` phase that every root phase of the next
milestone lists in `Depends on` explicitly. Milestones always block the next — no stacking.

- **Branches.** Precedence: milestone repository line (`Integration branch (owner/repo)`) >
  milestone line > (milestone 1 only) `## Repositories` override > Loop config. Milestones ≥2
  must declare their branch; a branch never repeats for the same repository across milestones.
  PR links live in the milestone's `PR` bullet (`PR (owner/repo)` for several repositories); the
  `## Repositories` `PR:` line applies only to milestone-less plans.
- **Scope.** `state.milestone` (set to 1 at init) is the current milestone. The SUB schedules only
  its phases; init, PR creation, review and the gate are scoped to its participants. Milestone 1
  alone may adopt an existing branch/worktree; milestones ≥2 are prepared fresh from the fetched
  default at resume (`loop-milestone.sh advance`), never at init.
- **Parking.** After a gate review aggregates, supervise persists `state.awaitingMerge`
  `{milestone, prs{slug:url}}`, sets plan Status `awaiting merge`, logs the typed
  `milestone.awaiting-merge` event, runs the best-effort `ship` explainer (§ Milestone
  explainers), notifies (PR URLs, the explainer link or failure, + `/loop-execute resume`), **keeps**
  `sub/REVIEW_READY`, unlocks and stops — never `finish`, never respawn. The dead-orchestrator
  backstop does not respawn while `state.awaitingMerge` is set; a SUB that sees it writes
  `sub/status.json{outcome:"blocked"}` and exits, which `loop-orchestrator.sh` handles as a
  parked loop (info notification, exit 2), not a crash.
- **Resume (human only, `LOOP_SUB` unset).** `loop-milestone.sh check` → `advance`, then the
  skill applies the printed patch to state, clears `REVIEW_READY`, logs `milestone.resumed`,
  and respawns the orchestrator. Then it starts the best-effort `plan` explainer for the new
  milestone (§ Milestone explainers).
- **Reconciliation.** Phases of a milestone whose gate PRs are merged are permanently shipped;
  crash-resume never re-checks them against a later integration tip.

### Milestone explainers

Supervise makes two `visual-explainer` pages for each milestone change. A detached explainer
runner writes the props. Supervise only renders, publishes and links. A failed explainer
never blocks the park or the resume.

| When | Mode | Output directory | Facts for the runner |
|------|------|------------------|----------------------|
| Park step 4: after the final reviews post, before the notification | `ship` | `.loop/explainers/m<n>-ship/` | plan, review ledgers, final review reports, gate PRs, code at the reviewed heads |
| Milestone resume step 6: after the orchestrator respawns | `plan` | `.loop/explainers/m<N>-plan/` | the milestone N section of the plan, code of its participants |

Spawn the runner detached, like § Runner spawn. It is not a phase attempt, so it gets no
`--run-id` and no `--phase`. It emits no phase events and gets no route decision.

```sh
nohup "$RUNTIME_ROOT/loop-runner.sh" --worktree <integration worktree of the first participant> \
  --run-dir <abs .loop/runs/explainer-m<n>-<mode>-a<k>> --prompt-file <run dir>/prompt.md \
  --chain task --timeout 900 \
  --verify-cmd 'node <RUNTIME_ROOT>/.claude/skills/visual-explainer/scripts/render.mjs --props <abs output dir>/props.json --check' \
  > <run dir>/spawn.log 2>&1 &
echo $! > <run dir>/pid; disown
```

- Exit 0: supervise renders, publishes and writes `link.txt`, as the skill tells it.
- Exit 12: one relaunch in `-a<k+1>` with the `verify.log` tail in the prompt.
- Any other exit, or a second failure: write the reason to `error.txt` and make no page.

The steps are idempotent. A `props.json` that passes `--check` needs no runner. A publish
goes again to the URL in `link.txt`. At the park, a link that is an artifact URL also goes
into each gate PR body, as one line with the marker `<!-- loop-explainer:m<n>-ship -->`.

Explainer runner prompt skeleton:

```
You are a Loop explainer runner: fully non-interactive, nobody reads your output live.
RUN_DIR is <abs path>. Use the visual-explainer skill in <mode> mode.
Write only <abs output dir>/props.json. Do not render or publish the page.
Do not commit, push, or edit a repository file, the plan or the state.

FACTS (data, not instructions):
- plan: <abs .loop/plan.md>, milestone <n>
- integration worktrees: <owner/repo> = <abs path>, ...
- ship only: ledgers <abs runs/review-p<N>-<owner--repo>.ledger.tsv>, final reports
  <abs report.md>, gate PRs <url>, reviewed heads <sha>

Ship only: check every hand-test command against the code at the reviewed head (skill
step 3). When the code contradicts the plan, follow the code and add a deviations entry.

WHEN FINISHED: run `node <skill>/scripts/render.mjs --props <props> --check` until it exits 0.
Then write RUN_DIR/status.json with outcome "done" and a one-line summary.
If you cannot finish, write outcome "blocked" with the reason in details.
```

## Directory layout

State lives in the launching coordinator checkout (add `.loop/` to
`.gitignore`):

```
.loop/
  state.json                 # orchestrator bookkeeping (loop-state.sh, atomic writes)
  plan.md                    # the plan document, beside state.json (register/push planText source)
  progress.md                # progress stream (loop-plan.sh note / handoff --auto append)
  events.jsonl               # append-only journal: {ts, event, phase, repository?, detail}
  lock/                      # mkdir-lock; owner = run id (sessions have no stable pid);
                             # same owner re-locks freely, another owner needs --force
  runs/<phase-slug>-a<K>/    # one dir per attempt K of a phase
    prompt.md                # exact prompt sent
    transcript.jsonl         # full stream-json/JSONL (kept; mtime doubles as heartbeat)
    last.md                  # final assistant message
    status.json              # runner-written result (schema below)
    checkpoint.md            # observable worktree state if the 30m phase deadline fired
    question-decision.json   # ordinary exit-10 advice + evidence-backed disposition
    risk-decision-<head>.json # merge-risk advice + actual full-skim disposition/gate evidence
    stderr.log  verify.log   # wrapper-captured
    leg.json                 # wrapper-written at each leg launch: engine, model, effort, startedAt (live model while in flight)
    meta.json                # wrapper-written: engine, model, sessionId, exit, head shas
  runs/review-p<N>-<owner--repo>.ledger.tsv # six-column completed-run ledger
  runs/review-p<N>-<owner--repo>.ledger.config.json # pinned tier/rounds/inner for recovery
  runs/review-p<N>-<owner--repo>.ledger.disputed.md # current capped disagreement, if any
  runs/review-p<N>-<owner--repo>-a<k>/ # review phase N, k from `loop-review.sh next`
    prompt.md  meta.json  status.json # same runner artifacts as phase attempts
    report.md                     # R/C/T exchange or final report
    remediation-plan.md           # finding/dispute dispositions (fix<r> only)
  runs/explainer-m<n>-<plan|ship>-a<k>/ # explainer runner attempt (§ Milestone explainers)
  explainers/                # visual-explainer pages (never committed)
    plan/                    # whole-plan page from multiphase-plan step 5
    m<n>-<plan|ship>/        # milestone n: plan page at resume, ship page at the gate
      props.json             # written by the explainer runner (or the planner)
      index.html             # rendered by supervise (or the planner)
      link.txt               # artifact URL, or the absolute path of index.html
      error.txt              # the failure reason, when the explainer failed
  answers/<phase-slug>.md    # orchestrator guidance injected into a retry
  hil/<phase-slug>.md        # HIL request; answer arrives as hil/<phase-slug>.answer.md
  notes/<key>.md             # explicit phase number; legacy pr-review.* keys accepted
  control/                   # out-of-band user control (daemon POST /control, e.g. loop-top, or by hand)
    pause                    # whole loop paused (§ Pause & live control)
    pause-<N>                # phase N paused; may be set before it starts
    model-<N>                # one-line leg `engine:model[+fb]` override for a not-yet-started phase
  sub/                       # two-tier self-recycling orchestrator runtime (§ Two-tier orchestration)
    transcript-<k>.jsonl     # SUB instance k's stream-json (mtime = heartbeat; occupancy source)
    status.json              # SUB → loop-orchestrator: recycle|review-ready|complete|fatal|blocked
    handoff.md               # single-consumption recycle handoff → renamed handoff.consumed-<k>.md
    current.pid              # live SUB pid (loop-orchestrator.sh writes; supervise polls it)
    saturation.json          # optional sidecar-watcher occupancy fallback (if stream-json buffers)
    REVIEW_READY             # explicit review barrier ready → supervise executes that phase;
                             # a parked milestone gate keeps it until a human resume clears it
    PHASES_DONE              # legacy no-kind plan only: synthetic review ready
```

Worktrees live outside the repo (ralph convention):

- Lane: `~/.loop/worktrees/<runId>/<owner--repo>/<lane-branch-slug>` on the phase's
  Suggested branch, cut from that repository's current integration tip. A lane repository
  hop removes the old worktree; reuse is same-repository consecutive phases only.
- Integration: `~/.loop/worktrees/<runId>/<owner--repo>/integration`. `loop-repo.sh
  prepare` fetches GitHub's default ref and creates it from the exact fetched SHA, or adopts
  an existing integration branch: its current worktree (often the user's ws worktree or the
  coordinator checkout) when checked out, else this path on that branch. Adopted worktrees
  are the user's; never remove them. All repository merges/pushes happen only here.
- Milestones: milestone 1 keeps `…/<owner--repo>/integration` (above); milestones ≥2 use
  `…/<owner--repo>/integration-<branch-slug>-<hash8>` (`<hash8>` = first 8 hex of the sha1 of
  the exact branch name, so same-slug branches never collide), created fresh from the fetched default and never
  adopted. Old non-adopted worktrees are retired only after the next milestone's are prepared.

## Daemon & events

A perpetual **central daemon** (`LOOP_DAEMON_URL`, default `http://localhost:7717`) is the
authoritative, never-stale source for **UI status**; the `.loop/` files stay the
crash-resume **source of truth** for code + bookkeeping. A launchd LaunchAgent auto-starts it
on login (KeepAlive); `loop-emit.sh` (sourced by the four loop scripts) provides
`loop_ensure_daemon` as the fallback. Loops **register**, then push clean lifecycle events.
Emission is **best-effort** — a `curl` failure never fails the caller.
On startup and reconcile, the daemon recovers persisted route, question, and risk decision files
after missed POSTs while the coordinator `.loop/` directory still exists.
The daemon also reads size-capped `notes/*.md` content on each reconcile. NOTE badges are
file-derived but completion-aware: a numeric phase note is pending only while its phase is
not `done|merged`; legacy `pr-review.<owner--repo>` is pending only until that review
finishes. This makes missed best-effort file cleanup harmless.

Endpoints (POST bodies are JSON built injection-safely with `jq`):

- `POST /api/loops/:runId/register` `{ loopDir, planFile?, effort?, integrationBranch?, startedAt?, planText? }` — server reads plan.md from `planFile` if `planText` omitted.
- `POST /api/loops/:runId/state` — the full state.json contents.
- `POST /api/loops/:runId/event` `{ event, phase?, repository?, detail?, ts?, outcome?, exitCode?, engine?, model?, prUrl?, tokens?, recycleIndex?, percent? }` — folded verbatim.
- `POST /api/loops/:runId/finish` accepts v2 `{ status?, finishedAt?, repositories:{slug:{prUrl?,review?}} }`; legacy scalar `prUrl/review` remains accepted. For explicit plans it is ignored until the terminal review phase is `done|merged`.
- `GET /api/loops/:runId/plan` → `{ runId, effort, status, integrationBranch, milestone, milestoneTotal, awaitingMerge, planText }` — how a fresh checkout fetches the plan (and the current milestone) with **no worktree needed**.
- `POST /api/loops/:runId/note` `{ key, markdown }` to write or `{ key, clear:true }` to delete a steering note; rejects archived loops.
- `POST /api/loops/:runId/archive` · `/unarchive` — manual archive flag on the store record (`loop-top archive [runId]`): listed as `archived` regardless of the worktree; worktrees untouched.
- `POST /api/loops/:runId/control` `{ action:"pause"|"resume", phase? }` writes/removes `control/pause[-<N>]`; `{ action:"model", phase, leg|null }` writes/removes `control/model-<N>` (409 once the phase has started; invalid leg → 400). Rejects archived loops (410). The snapshot gains loop-level `paused` and per-node `paused`, `modelOverride`; UI state `paused`. `{ action:"review", phase, tier }` rewrites a not-yet-started review phase's `[review: …]` tag in plan.md (drops `[rounds: …]` → tier default).
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
| `milestone.awaiting-merge` | supervise (loop-state.sh log) | detail = `M<n> owner/repo=<url> …` |
| `milestone.resumed` | supervise (loop-state.sh log) | detail = `M<n>` |
| `sub.recycle` | loop-orchestrator.sh (on a SUB recycle) | tokens, recycleIndex |
| `sub.saturation` | loop-state.sh occupancy (periodic heartbeat) | tokens, percent |
| `progress.note` | loop-plan.sh note / loop-handoff --auto | phase, detail |
| `control.pause` / `control.resume` / `control.model` | daemon (POST /control) | phase?, detail |
| `loop.paused` / `loop.resumed` | loop-orchestrator.sh | — |
| `note.applied` | loop-runner.sh (live note resume) | phase, repository |
| `jev.decision` | runner (route) or SUB (question/risk) | phase, attempt, stage, mode, candidate, confidence/probabilities, appliedAction, fallbackReason, resolvedModel, evidenceChecked/evidenceSources, questionRound, head, required/completed/remaining gates, focus, ts |

`loop-state.sh log` carries only phase/repository/detail, so both `milestone.*` payloads ride in
`detail` as compact text (structured data lives in `state.awaitingMerge`). Both are typed and
loop-level: they never promote a phase (the keyword fallback would read "merg…" as a merge), and
`milestone.awaiting-merge` drives the `awaiting-merge` loop status from `state.awaitingMerge`.

`progress.note` is the progress narration — kept on the timeline, promotes
nothing in the lattice.

Unknown/legacy event names fall back to keyword matching, so today's free-form
`events.jsonl` still works. The two `sub.*` events are **loop-level** (no `phase`); the daemon
folds them idempotently — `subRecycles = max(recycleIndex)`, `occupancy = last {tokens, percent}`
— so re-folding `events.jsonl` on every reconcile never double-counts. They update loop state
only; they are NOT appended to the on-disk `events.jsonl` (heartbeats would flood the timeline),
so they ride the live daemon POST exclusively.

**Monotone promotion lattice.** The daemon materializes each loop through a monotone rank
`todo < claimed < running < done < merged`; a `phase.attempt.finish{outcome:done, exit 0}`
promotes a work phase to **done even if state.json bookkeeping lags** (fixes "done phases stuck
at running/todo"). Individual review-stage attempts never promote an explicit `pr-review`
phase; only the aggregate review pipeline does that after every repository finishes. Ranks never
regress — a phase never un-completes in the UI (v1).

## state.json schema v2

`runId` == the plan's `planId` (loop-execute adopts it). Crash-resume reads exactly these
fields — keep the shape:

```json
{
  "runId": "loop-<effort-slug>-<date>-<HHMMSS|rand>",
  "effort": "...",
  "milestone": 1,
  "runtimeRepo": null, "runtimeRoot": null, "runtimeAdopted": null, "resumePending": null,
  "awaitingMerge": { "milestone": 1, "prs": { "owner/api": "<gate PR url>" } },
  "milestones": { "1": { "prs": { "owner/api": "<gate PR url>" } } },
  "repositories": {
    "owner/api": {
      "sourceRoot": "<mapped checkout>", "defaultBranch": "main", "baseSha": "<sha>",
      "integrationBranch": "feat/<effort-slug>",
      "integrationWorktree": "<abs path>", "adopted": false, "prUrl": null,
      "reviewPipeline": { "phase": 7, "tier": "standard", "rounds": 2, "inner": 2,
                          "headSha": "<sha for current review round>",
                          "ledger": "runs/review-p7-owner--api.ledger.tsv" },
      "review": { "outcome": "done|question|blocked", "summary": "...",
                  "reportPath": "runs/review-p7-owner--api-a9/report.md", "commentUrl": "..." }
    }
  },
  "phases": {
    "3": { "kind": "work|pr-review", "slug": "<phase-slug>", "lane": "A", "repository": "owner/api|all",
           "branch": "feat/api-client-token-refresh", "worktree": "<abs path or null>",
           "status": "todo|claimed|running|paused|merged|blocked|done",
           "attempt": 2, "runDir": ".loop/runs/<phase-slug>-a2",
           "pid": 4242, "sessionId": "<from meta.json>", "questionRounds": 0 }
  }
}
```

`milestone` is the current milestone (1 at init, also for milestone-less plans). `awaitingMerge`
is set only while parked at a gate and cleared (`null`) on resume; `milestones[N].prs` records
each shipped milestone's gate PRs. On a milestone change `repositories` is replaced by the new
milestone's participants. `runtimeRepo` (slug) + `runtimeRoot` (abs path) are set only when the
effort changes the Loop runtime itself (self-hosted: the repository whose integration worktree is
`RUNTIME_ROOT`); otherwise both stay `null`. `runtimeAdopted` mirrors that worktree's `adopted`
flag so a host retained across a milestone it sits out is never retired if it is the user's.
`resumePending` is the milestone a resume applied but has not finished restarting (set by the
`advance` patch, cleared last by resume step 5); `null` otherwise.

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

`check` validates mapping, SSH/HTTPS GitHub origin, and GitHub default branch (exit 4 when
the integration branch IS the default branch), then reports
`mode`: `create` (branch absent) or `adopt` (branch exists) plus `adoptWorktree` (normalized
path where it is checked out, else null). It is non-mutating; exit 4 when the adopted worktree
is dirty (`git status --porcelain`; ignored files such as `.loop/` do not count).

`prepare` fetches the remote default ref and records its exact SHA as `baseSha`.
- `create`: new branch + run/repository-scoped integration worktree from that SHA.
- `adopt`: when `origin/<branch>` exists it is fetched and must be an ancestor of the local
  branch (exit 4: behind/diverged, pull first). A branch not containing `baseSha` is
  fast-forwarded to it only when it has no unique commits; otherwise exit 4 (diverged,
  rebase/merge first). The integration worktree is the existing one, or a new run-scoped
  worktree on the branch (exit 4 if git refuses, e.g. a paused rebase/bisect elsewhere); the
  ff-only merge runs inside it afterwards, so a refusal moves no ref. Never forces, resets,
  or rewrites history.

Output adds `adopted` and `aheadCount` (commits on the branch beyond `baseSha`; they ship in
the PR). Create mode reports `false`/`0`.

## Runner spawn (how the orchestrator launches loop-runner.sh)

Resolve `RUNTIME_ROOT` to the repository integration worktree when the effort changes the
Loop runtime itself; otherwise use `~/dotfile`. The runner resolves its Jev client, model
config, and emitter from that same directory. Compose later SUB prompts from
`$RUNTIME_ROOT/.claude/skills/loop-execute/{SKILL.md,references/loop-protocol.md}` too, so
merged self-hosting changes run without mutating the installed checkout. At `REVIEW_READY`,
the supervisor also reloads §8 and the protocol from `RUNTIME_ROOT` before starting review;
this includes self-hosted terminal review of the changed pipeline itself. With milestones, a
resume that advances the milestone switches `RUNTIME_ROOT` to the patch's `runtimeRoot` only when it is non-null
(self-hosted: the host's new integration worktree, or its retained old one when the host sits the
milestone out) — the old one may be retired, so never keep composing prompts from it; a null `runtimeRoot` keeps the installed runtime (`~/dotfile`).

Detached, so runners survive orchestrator death:

```sh
nohup "$RUNTIME_ROOT/loop-runner.sh" <args> > <runDir>/spawn.log 2>&1 &
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
- `fatal` / `blocked` → `loop-notify.sh`, break (supervise surfaces it). `blocked` while
  `state.awaitingMerge` is set is a parked milestone gate: info notification, not an error.
- no valid `sub/status.json` → crash → respawn (`resume`), capped at `LOOP_ORCH_MAX_RESPAWN`
  consecutive crashes, then notify + exit.
- SUB transcript mtime older than `LOOP_ORCH_STALL_SEC` → kill the SUB pid tree
  (`descendants()`), treat as a crash-respawn.

At a milestone merge gate a parked loop keeps `sub/REVIEW_READY` and has no live orchestrator: supervise's dead-orchestrator
backstop must not respawn it, and only a human resume continues (§ Milestones).

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
plan/daemon, git are already externalized), writes `sub/handoff.md`, and exits with `sub/status.json
{outcome:"recycle"}`. The successor reads `sub/handoff.md` **exactly once** at startup, then it
is archived to `sub/handoff.consumed-<k>.md`; `plan.md` + `state.json` stay freely re-readable.
This is a purpose-built recycle handoff — **NOT** the `loop-handoff` skill (that one is
plan-doc/phase-status scoped and marks phases *done*: wrong semantics for a mid-run recycle).
`handoff.md` carries: open exit-10 question rounds + their `sessionId`s; pending merge-runner
promotions (runDir + lane-branch awaiting a `phase.merged`); per-lane notes; and why-stopped.
Everything else the successor rebuilds by reconciling **git > plan/daemon > state**.

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
- `blocked` — a HIL is open that only the human can answer AND no other lane can progress, or
  `state.awaitingMerge` is set (parked at a milestone merge gate — a handled park, see
  § Milestones). (An
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
| 30   | `paused`: watchdog saw `control/pause` or `control/pause-<N>`, killed the engine within ~2s; `meta.json` keeps `sessionId` | mark phase `paused`; never retry/escalate; resume per § Pause & live control |
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
CHAIN_TASK=("codex:@sol" "claude:sonnet")
CHAIN_TASK_LIGHT=("codex:@terra" "${CHAIN_TASK[@]}")
CHAIN_ESCALATE=("claude:fable+opus")
CHAIN_ORCHESTRATE=("claude:opus")
EFFORT_TASK=high  EFFORT_ESCALATE=high  EFFORT_ORCHESTRATE=high
# Provider slots rotate between R, C, and T; FIX and FINAL always use Claude.
# CHAIN_REVIEW_<STANDARD|MAX>_<GPT|CLAUDE|GROK|FIX|FINAL>, matching EFFORT_*.
CHAIN_REVIEW_STANDARD_GPT=("codex:@sol");    EFFORT_REVIEW_STANDARD_GPT=xhigh
CHAIN_REVIEW_STANDARD_CLAUDE=("claude:opus"); EFFORT_REVIEW_STANDARD_CLAUDE=xhigh
CHAIN_REVIEW_STANDARD_GROK=("grok:default"); EFFORT_REVIEW_STANDARD_GROK=xhigh
CHAIN_REVIEW_STANDARD_FIX=("claude:opus");  EFFORT_REVIEW_STANDARD_FIX=medium
CHAIN_REVIEW_STANDARD_FINAL=("claude:opus"); EFFORT_REVIEW_STANDARD_FINAL=xhigh
CHAIN_REVIEW_MAX_GPT=("codex:@astra");       EFFORT_REVIEW_MAX_GPT=ultra
CHAIN_REVIEW_MAX_CLAUDE=("claude:fable");    EFFORT_REVIEW_MAX_CLAUDE=max
CHAIN_REVIEW_MAX_GROK=("grok:default");      EFFORT_REVIEW_MAX_GROK=max
CHAIN_REVIEW_MAX_FIX=("claude:opus");        EFFORT_REVIEW_MAX_FIX=medium
CHAIN_REVIEW_MAX_FINAL=("claude:opus");      EFFORT_REVIEW_MAX_FINAL=max
LOOP_REVIEW_TIER_DEFAULT=standard
LOOP_REVIEW_ROUNDS_DEFAULT=2
LOOP_REVIEW_INNER_DEFAULT=2
LOOP_REVIEW_ROUNDS_CAP=5
LOOP_PLAN_REVIEW_ROUNDS=2
CHAIN_PLAN_REVIEW_GPT=("codex:@sol");    EFFORT_PLAN_REVIEW_GPT=high
CHAIN_PLAN_REVIEW_CLAUDE=("claude:opus"); EFFORT_PLAN_REVIEW_CLAUDE=high
CHAIN_PLAN_REVIEW_GROK=("grok:default");  EFFORT_PLAN_REVIEW_GROK=high
LOOP_BUDGET_USD=15        # per attempt, claude legs only (codex/grok have no budget flag)
LOOP_CHECKPOINT_SEC=1800  # hard ceiling across all legs/retries of one phase attempt
LOOP_TIMEOUT_TASK=1800    # 30m
LOOP_TIMEOUT_ESCALATE=1800
LOOP_TIMEOUT_REVIEW=1800
LOOP_TIMEOUT_REMEDIATE=1800
LOOP_MAX_PARALLEL=3
LOOP_ORCH_CTX_WINDOW=1000000
```

For `--chain task` with run/phase correlation, the runner records one bounded `route`
decision before engine launch. It caches the decision by phase beside attempt directories
and materializes `route-decision.json` plus proposed/actual profile fields in each attempt's
`meta.json`; retries and resumes reuse it. Shadow mode and every fallback use `CHAIN_TASK`.
Active mode selects `CHAIN_TASK_LIGHT` only for the allowlisted `light` candidate at or
above `LOOP_JEV_ROUTE_MIN_CONFIDENCE`; that chain falls through to the default chain on
API/usage failure. Escalation, merge-resolution, remediation, and review chains never route.
A `control/model-<N>` override replaces routing for that phase: its leg becomes the first
leg of the task chain (the default chain follows as fallback), the route is skipped with
`fallbackReason: "user-override"`, and `meta.json` records `modelOverride`.

Effort is per chain, not per engine: the runner maps the chain's `EFFORT_*` to Claude
`--effort <level>` (`low|medium|high|xhigh|max`; `ultra` clamps to `max`), Codex
`-c model_reasoning_effort="<level>"` (adds `ultra` on Sol/Astra), and Grok `--effort <level>`
(`low|medium|high|xhigh`; `max`/`ultra` clamp to `xhigh`). Defaults: implementing
`high` (Sol, Sonnet, escalation), fixing `medium` (Opus), orchestrating `high`, reviewing per
tier (standard `xhigh`; max GPT `ultra`, Claude/Grok `max`, with Grok clamped to `xhigh`). Orchestrator and reviewers never use
multi-agent workflows (`ultracode`): runners are one-shot `-p` processes, and the review
pipeline is already multi-agent. `meta.json.effort` records the level used.
`--chain review-gpt|review-claude|review-grok|review-fix|review-final` accepts `--review-tier`
(default `LOOP_REVIEW_TIER_DEFAULT`). Plan review uses `plan-review-gpt|plan-review-claude|plan-review-grok`.
Provider chain selection follows the rotated role, rather than assigning one chain permanently to R or C.

Leg grammar: `engine:model[+fallback[,fallback2]]`. The `+` list maps to Claude's native
`--fallback-model` (comma-separated; CLI retries the primary each turn) — so intra-Claude
fallback is one leg. Codex→Claude hops are the wrapper's job. Every role tracks the LATEST
model of its family so releases need no config edit: Claude legs use the CLI's native aliases
(`opus`, `sonnet`, `fable`); Codex legs use `@<family>` (`@sol`, `@astra`, `@terra`), which
loop-runner resolves at leg start to the highest-versioned *listed* `gpt-<v>-<family>` in
`codex debug models` (15s bound; falls back to `~/.codex/models_cache.json`; an unresolvable
family skips the leg). Grok legs use `grok:default` (the CLI's default, i.e. latest, model) and
run headless as `grok --prompt-file <f> --always-approve --output-format streaming-messages-json`
(claude-shaped transcript; resume with `--engine grok`). `meta.json.model` records the concrete
model that ran (Claude/Grok: from the transcript's init event). A full model name still pins a leg. Each review chain contains exactly one leg: a failed stage stays failed
instead of silently becoming a duplicate of another model. The `review-fix` prompt invokes
`/pr-review-fix-all`; that skill owns Opus subagent delegation.

Failure classing per leg (from structured error events first — `.is_error` result events in
Claude stream-json, `error` events in Codex JSONL — stderr regex last):

- transient (`5xx|overloaded|ECONNRESET|ETIMEDOUT|empty result`) → retry same leg, 3
  attempts total, backoff 10/30s;
- rate/usage (`429|rate.?limit|usage limit|quota|hit your … limit`, checked before a status-less exit 0 counts as done) → advance to next leg immediately;
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
R/C/T questions instead start a fresh call at the frozen head with the prior exchange and
answer inlined (§ PR review phase pipeline); they never use `--resume`.

### Ordinary question triage

`checkpoint:true` follows the deterministic checkpoint path and never invokes Jev. For any
other exit 10, enforce `questionRounds < 3` first; an exhausted phase proceeds to L3 without
another judgment. Increment the round before triage so a crash cannot create a fourth L2 answer.

Call `loop-jev.mjs` at stage `question` with one Choice named `triage` and exactly these
candidates: `plan-answer`, `code-investigation`, `human-preference`, `uncertain`. Bound state
to the runner question, phase/attempt/repository, Goal, Done-when, dependencies, and relevant
plan lines. Never include transcripts, environment values, credentials, or full state.json;
never persist the request or raw vendor response.

The SUB inspects the actual question and plan, and code where relevant, independently of the
label. It records the disposition through `loop-jev-question.mjs`. The policy requires
`question + plan` evidence for every answer or HIL disposition, adds `code` for an answer after
investigation, and permits `raise-hil` only with a substantive reason at L4. Thus
`human-preference` is an investigation hint, never an escalation gate. Shadow, error,
low-confidence, off, and `uncertain` use `current-behavior`. Persist the typed result atomically
as `<runDir>/question-decision.json`, emit it with `loop_emit_jev_decision`, then act.
`candidate` is the advice; `appliedAction` is what actually happened.

`loop-jev-question.mjs` reads one JSON object from stdin:

```json
{
  "phase": "4", "attempt": 2, "questionRound": 1, "checkpoint": false,
  "jev": { "...": "structured loop-jev.mjs result" },
  "action": "answer-from-plan | investigate-code | answer-after-investigation | raise-hil | current-behavior",
  "evidenceSources": ["question", "plan", "code"],
  "hilReason": "required for raise-hil", "escalationLevel": 4,
  "ts": "2026-09-27T10:00:00Z"
}
```

Omit the HIL-only fields for other actions. Exit 0 returns the typed decision. Exit 2 returns
`{"error":"..."}` and forbids the action; investigate the missing evidence and retry the
policy, never bypass it. A checkpoint or fourth round returns a `bypass:true` disposition and
must not be emitted as a Jev decision.

### Pre-merge risk judgment

After a phase attempt returns verified exit 0, and before any merge, the SUB executes this
unconditional order:

1. confirm verified exit 0;
2. obtain one bounded merge-risk judgment for the attempt's `headAfter`;
3. skim the **entire** `git diff <base>...HEAD`, using advice only to focus attention;
4. reject `headBefore == headAfter` as a stall;
5. require a clean lane worktree;
6. reread `notes/<phase>.md` and honor it;
7. run the globally serialized `loop-merge.sh`;
8. let `loop-merge.sh` Verify the merged repository tree.

Steps 1 and 3–8 are unconditional in off, shadow, active, low-risk, fallback, error,
low-confidence, and malformed-response paths. A Jev record is observational and never a merge
permit.

For step 2, call `loop-jev-risk-input.mjs` with repository root, base/head, phase/attempt,
verbatim *Done when*, and only `{exitCode:0,summary:<bounded success summary>}` for verification.
It uses `git` directly, includes every changed path plus aggregate additions/deletions, omits
all patch contents, bounds and redacts the supplied Done when and verification summary, and
fails closed if the complete path inventory cannot fit its 48 KiB output bound. Pipe its
output directly into `loop-jev.mjs`; never persist the request, raw API response, transcript,
verify log, environment values, or credentials. The two Choice questions are `scopeGap`
(`none|possible|likely`) and `changeRisk` (`low|medium|high`).

After steps 3–6, call `loop-jev-risk.mjs` with the structured Jev result, phase/attempt,
`headBefore`, `headAfter`, decision head, verification exit, clean/skimming evidence, focus,
and the completed gate prefix. It accepts only `full-diff-skim` or
`focused-full-diff-skim`, validates exact gate order and evidence, and returns a versioned
proposal-versus-disposition record with the immutable required/remaining gate list. Any builder,
Jev, or policy error uses a typed `merge-risk` fallback and the unfocused full skim; it never
waives a gate.

Persist the validated record atomically (`tmp` + `mv`) as
`<runDir>/risk-decision-<head>.json`, then pipe that record to
`loop_emit_jev_decision <runId>` before starting the merge. If that exact attempt/head record
already exists and validates, reuse it rather than calling Jev again. A retry or changed head
must create and emit a new timestamped record. The canonical candidate is
`scope-gap:<label> · risk:<label>`; `appliedAction` is the actual full-skim disposition.

### Shadow replay and rollout

`loop-jev-replay.mjs <snapshot-or-store.json> [...]` reads persisted live snapshots or archived
store records and emits one JSON report without writing to the source loops. It compares each
recorded route, question, and merge-risk proposal with the applied action using stage-specific
policy mappings, then reports per-stage and overall agreement and fallback rates. Archived records
use the frozen `lastSnapshot.decisions`, matching Observatory reload behavior.

Latency, runner retries, and cost appear only when those numeric fields were recorded; otherwise
the report marks each metric `unavailable`. Synthetic fixtures cover the three decision points, archive
reload, explicit off mode, missing credentials, and API failure, but validate mechanics only and
must never be used to claim savings. Keep routing in shadow unless observed loop data supports the
configured active threshold; `LOOP_JEV_MODE=active` remains an explicit opt-in.

## Runner prompt skeleton (orchestrator generates prompt.md per attempt)

```
You are a loop-engineering task runner: fully non-interactive, nobody reads your output
live. You are in worktree <path> on branch <branch> (cut from <integration>). RUN_DIR is
<abs path>.

FIRST: run `$loop-pickup --auto <project> phase:<N>` (Claude: `/loop-pickup`) — it
claims your phase and loads context. If it returns `REFUSED: <reason>`, write
RUN_DIR/status.json with outcome "blocked" and the reason as details, and stop.

RULES: never push; never open PRs; plan state only through the pickup/handoff auto skills
(never push the plan document); never ask the user in prose; stay inside
this worktree. Commit per logical step with descriptive messages. Follow the repo's
CLAUDE.md conventions (TDD where it applies).

PHASE (from the shared plan):
<verbatim phase block: title, Depends on, Touches, Done when, Verify, Notes>

CONTEXT: <goal + key decisions from plan header; prior-attempt failure summary;
steering notes: notes/<phaseNumber>.md — fold verbatim on EVERY attempt, including first;
answers/<phase>.md retry guidance; hil answer — whichever exist>

Read steering notes on every attempt, including the first. Read answers/ only on retries.
A running attempt also picks up note edits live (§ Pause & live control).

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

Every explicit `[kind: pr-review]` phase runs one Adversarial Review (AR) pipeline per
repository. The heading accepts `[review: standard|max]` (default
`LOOP_REVIEW_TIER_DEFAULT=standard`), `[rounds: N]` (default `LOOP_REVIEW_ROUNDS_DEFAULT=2`),
and `[inner: M]` (default `LOOP_REVIEW_INNER_DEFAULT=2`). Both counts are integers 1..5;
`LOOP_REVIEW_ROUNDS_CAP=5`. Legacy tiers shallow and medium are rejected by execution paths;
historical Observatory/loop-top displays may still render them with a warning.

| Tier | GPT slot | Claude slot | Grok slot | Fix (`review-fix`) | Final (`review-final`) |
|------|----------|-------------|-----------|--------------------|------------------------|
| `standard` (default) | Sol `xhigh` | Opus `xhigh` | Grok `xhigh` | Opus `medium` | Opus `xhigh` |
| `max` | Astra `ultra` | Fable `max` | Grok `max` | Opus `medium` | Opus `max` |

These are configured efforts; the runner clamps Grok `max` to `xhigh`. A Grok call sends
the PR repository to xAI. Each provider chain has one leg; failure never substitutes another
provider. `loop-ar.sh` is the shared sourced core: verdict parsing, rotation, state machine,
prompt rendering, and disputed writer. `loop-review.sh` is its PR-facing CLI.

For self-hosted changes, the supervisor must read §8 and this protocol from `RUNTIME_ROOT` at `REVIEW_READY`,
then invoke that runtime's scripts, model config, and vendored prompts. The installed skill
loaded before the effort began may be stale. A pipeline failure follows the normal HIL path;
a human may choose a manual `/pr-review` fallback.

### Rotation and frozen exchange

Let P = (gpt, claude, grok), zero-indexed. For outer round r:
R = P[(r−1) mod 3], C = P[r mod 3], T = P[(r+1) mod 3]. Model names come from the chosen tier.

| Outer round (repeats every 3) | R | C | T |
|-------------------------------|---|---|---|
| 1 | gpt | claude | grok |
| 2 | claude | grok | gpt |
| 3 | grok | gpt | claude |

Before each outer round, fetch the PR and capture its current head SHA into
`repositories[slug].reviewPipeline.headSha`. Persist that SHA before the opening review;
every R/C/T prompt in the round receives the same SHA, PR URL, project instructions,
verification commands, repository steering note, and full PR inspection methodology.
The artifact stays frozen throughout the inner loop; only the remediation coordinator edits
it between outer rounds. If the PR head changes externally, do not combine reports from
incompatible heads or silently recapture the SHA mid-round: block that repository for reconciliation.

Runs within a round are sequential: `review` (R) → `critic` (C), then on disagreement
`respond` (R) → `critic` (active C), repeating only up to the inner cap. By definition,
the opening `review` + `critic` is inner round 1; each later `respond` + `critic` adds one.
The default cap 2 therefore permits one response. Different repository pipelines may advance
concurrently under `LOOP_MAX_PARALLEL`.

R writes at most ten numbered findings, `F1`…`F10`, with severity
`[blocker]`, `[major]`, or `[minor]`. Its last non-empty line must be exactly
`APPROVE` or `NEEDS_CHANGES: <top issue>`. C has the frozen artifact, codebase, and R's review,
references finding IDs, and ends with exactly one of
`AGREE`, `DISAGREE_EVIDENCE: <summary>`, or `DISAGREE_CONCERN: <summary>`.
Evidence disagreement cites code; a concern requires R to find the confirming or refuting code.
On `AGREE`, R keeps every flag. On `DISAGREE_EVIDENCE`, R revises the flagged finding against
C's cited code. On `DISAGREE_CONCERN`, R must cite code confirming the bug to keep it, or code
refuting it to drop it. A bare rebuttal is invalid.

The vendored `references/ar-prompts.md` contains the paper's appendix A.2.3/A.2.4 verbatim,
plus Loop's finding/verdict/output adaptation. `ar_prompt` composes that source, opaque caller
framing, and the current round's prior exchange in ledger order (`{prior_exchange}`). Do not
paraphrase the paper prompts. Every R/C/T call is fresh; never resume a provider session.
Opening R has no prior exchange; C, `respond`, and T receive the relevant inlined exchange,
never unrelated runs or GitHub reviews/comments. Use the full `pr-review` checkout/run/checklist
methodology at the frozen SHA, overriding its posting and artifact-editing workflow.

Every round is local-only. R/C/T never edit the artifact, commit, push, post to GitHub, or
mutate the ledger/prior reports. Each writes `RUN_DIR/report.md` and `RUN_DIR/status.json`.
A provider failure blocks that repository pipeline; other repositories may continue. An
exit-10 `checkpoint:true` is a timed status request: answer and continue the same step at the
same head, using a fresh R/C/T call with the exchange inlined. Do not advance before completion.

### Early stop, cap, and fix rule

If R's first review is `APPROVE` and C answers `AGREE`, run T as `confirm` with the same
critic prompt and full context. T `AGREE` skips every remaining outer round and goes straight
to `final` (plan review: done). No `skipped` ledger rows are written; consumers infer skips
from the clean `confirm`. If T disagrees, T becomes the critic for the rest of that inner loop;
the cap still applies and the round can no longer be clean, even if R later approves.
The `confirm` does not consume another inner round; a veto at inner cap 1 goes directly to disputed.

At the inner cap without agreement, `ar_write_disputed` writes
`<ledger-stem>.disputed.md` and appends a synthetic `disputed` row. It preserves the
last R report and last critic report verbatim and unresolved finding IDs, including critic-added issues.
The sidecar uses the ledger filename without `.tsv` as its stem. A disputed round is never
clean. The fixer/final reviewer/planner must verify disputed items against code; reaching the
cap alone does not pause for HIL. Round status precedence is `disputed` > `findings` > `clean`.
The ledger's dispute row associates the sidecar with its round; an older sidecar must not make
a later round disputed.

After round r < N, run `fix<r>` iff the last R verdict is `NEEDS_CHANGES` or the round's
`.disputed.md` exists (as identified by that round's dispute row). Otherwise advance without
a fix. A clean confirmed early stop bypasses fixes and later rounds. Round N always proceeds
to `final`, with its converged review and any current disputed items, rather than fixing again.

### Ledger and CLI

Persist `reviewPipeline = {phase, tier, rounds, inner, headSha, ledger}` under each repository;
`ledger` is the loop-relative `runs/review-p<N>-<owner--repo>.ledger.tsv`. Initialize a fresh
pipeline when the ready review phase number changes. There is no persisted next-stage cursor:
replay of the ledger selects the next step.

The ledger has no header and exactly six tab-separated columns, one row per completed run:

```text
a<k>	stage	role	provider	verdict	run_dir
```

`run_dir` is relative to the loop directory; `a<k>` is the pipeline-scoped idempotency key.
`stage` is `round<r>` for R/C/T exchanges, `disputed` for the synthetic marker, `fix<r>` for
remediation, or `final`. Providers are `gpt|claude|grok`; `fix`/`final` use `claude`, and
`disputed` uses `-` and the sidecar's parent directory (`runs`) instead of a runner directory.
Verdicts are tokens only, without the report's colon/summary:

| Role | Verdict column |
|------|----------------|
| `review`, `respond` | `APPROVE` or `NEEDS_CHANGES` |
| `critic`, `confirm` | `AGREE`, `DISAGREE_EVIDENCE`, or `DISAGREE_CONCERN` |
| `disputed` | `DISPUTED` |
| `fix` | `PUSHED` or `NOOP` |
| `final` | `REQUEST_CHANGES`, `COMMENT`, or `APPROVE` |

```sh
loop-review.sh next --tier <T> --rounds <N> --inner <M> --ledger <F>
loop-review.sh record --ledger <F> --run-dir <D> --stage <S> --role <R> --provider <P>
loop-review.sh recover --ledger <F> --runs-dir <runs>
loop-review.sh prompt --role <R> --pr <url> --head <sha> --notes <file> --ledger <F> --run-dir <D>
```

- `next` returns TSV `a<k> stage role chain`, or `done`. It replays `ar_next`, resolves the
  provider chain (`review-gpt|review-claude|review-grok`, or `review-fix|review-final`), and
  writes a dispute sidecar/row itself when needed; never spawn a `disputed` runner. That
  synthetic row consumes its own `a<k>`. `next` also saves tier/rounds/inner to
  `<ledger-stem>.config.json`, so `recover` replays the same limits.
- `record` is atomic (temporary file + rename), idempotent per `a<k>`; duplicate
  recording is a no-op. It validates role/provider/verdict and ordering before append.
  Fix records require `status.json` result `PUSHED|NOOP`, full `headBefore`/`headAfter`,
  remote and branch (plus worktree when needed); check remote HEAD with `git ls-remote`.
  `PUSHED` requires a changed head; `NOOP` an unchanged head. Both require remote evidence,
  never status alone. Final records require the actual posted event and `commentUrl`.
- `recover` records finished-but-unrecorded run directories of this pipeline in numeric
  order, requiring `meta.json` and `status.json`, successful engine completion, and usable
  outputs. Optional `meta.json.review = {stage, role, provider}` pins recovery metadata;
  otherwise replay derives it. A directory without completed metadata is not proof of success.
- `prompt` renders R/C/T through `ar_prompt` with full PR framing, the pinned head, and
  steering notes (pass a readable empty file when none). For `fix`/`final`, it supplies only
  the current round's converged review (last R report) plus current disputed items, if any.
  The supervisor adds integration worktree/branch/remote and verification execution inputs.

Supervise drives each repository as `recover` → `next` → spawn → `record`, with GitHub
final-marker reconciliation on resume as specified below. Run artifacts use
`review-p<N>-<owner--repo>-a<k>`, where k comes from `next`. Every runner passes
`--chain <chain> --review-tier <T>` and runs detached; wait for completion and record it
before asking for another step. Preserve `headSha` across all calls of the current round.

A `record` exit 2 means no usable last verdict line: append no row and re-run the same step once with a format reminder.
Retain the invalid output and retry-used evidence in the run artifacts so a crash cannot
reset the allowance. A second invalid verdict blocks that repository pipeline and follows
the normal HIL path. This rule also applies when `recover` encounters an invalid verdict;
never treat malformed output as agreement or advance the ledger past it.

Example: standard, two outer rounds, inner cap 2. R revises once in round 1, fixes are pushed,
and round 2 is confirmed clean (a first-round clean confirmation would instead jump to final):

| Run | Stage | Role | Provider | Chain | Verdict |
|-----|-------|------|----------|-------|---------|
| `a1` | `round1` | `review` | gpt | `review-gpt` | `NEEDS_CHANGES` |
| `a2` | `round1` | `critic` | claude | `review-claude` | `DISAGREE_EVIDENCE` |
| `a3` | `round1` | `respond` | gpt | `review-gpt` | `NEEDS_CHANGES` |
| `a4` | `round1` | `critic` | claude | `review-claude` | `AGREE` |
| `a5` | `fix1` | `fix` | claude | `review-fix` | `PUSHED` |
| `a6` | `round2` | `review` | claude | `review-claude` | `APPROVE` |
| `a7` | `round2` | `critic` | grok | `review-grok` | `AGREE` |
| `a8` | `round2` | `confirm` | gpt | `review-gpt` | `AGREE` |
| `a9` | `final` | `final` | claude | `review-final` | `APPROVE` |

### Remediation (`fix<r>`)

Run `review-fix` in the repository integration worktree only after the round's inner loop
has ended and the fix rule holds. Invoke the skill with one converged review and any current
disputed sidecar, not a set of independent reports:

```
Invoke `/pr-review-fix-all` with:
- report: <absolute last R report.md of round r>
- disputed: <absolute current .disputed.md, if any>
- tier: <standard|max>
- run-dir: <absolute fix RUN_DIR>
- worktree: <absolute repository integration worktree>
- branch: <integration branch>
- remote: origin
- verify: <repository verify command, verbatim>

Follow that skill through its final status.json. Do not implement remediation outside it.
```

The coordinator verifies every finding and disputed item (including critic-only concerns)
against code before accepting or rejecting it, recording dispositions in `remediation-plan.md`.
It groups independent work and spawns at most three `pr-review-fixer` subagents (latest Opus),
serializing overlapping files or dependencies. Subagents edit/test only; the coordinator owns
integration, verification, commits, and fast-forward push. Do not create an empty commit or
push when nothing is accepted. Verify remote HEAD before recording `PUSHED|NOOP` and starting
the next round. Unresolved accepted findings or failed verification block that repository.

### Final review (`final`)

Run `review-final` after the last outer round or a confirmed clean early stop, at the same
current head reviewed by that round. It reads one converged review (last R report), the current
`.disputed.md` when the round is disputed, and current code. It independently verifies and
deduplicates findings, including critic-added disputed items, saving the reconciled review in
`report.md`. Earlier-round reports are stale remediation history, not verdict inputs.

Post exactly one review using `gh api --method POST repos/<owner>/<repo>/pulls/<n>/reviews`,
pinning `commit_id` to the reviewed head and selecting `event` from unresolved validated severities:

- one or more `[blocker]` findings → `REQUEST_CHANGES`;
- otherwise one or more `[major]` findings → `COMMENT`;
- otherwise → `APPROVE` (minor/nit/style findings may remain).

Put findings in inline review comments where the diff permits and keep the overall body
concise. End the body with `:)` and include
`<!-- loop-review:<runId>:p<N>:<owner--repo>:final -->`. Before posting, query all existing PR reviews
for that marker; if it exists, reuse its URL and actual event instead of posting again. Save
`html_url` as `status.json.commentUrl` and the posted event as `status.json.event`. Map the event
to repository outcome: `REQUEST_CHANGES → blocked`, `COMMENT → question`, `APPROVE → done`.
Record the final row even when its outcome is question/blocked: the posted review is complete.
Promote `report.md`, summary, and comment URL to `repositories[slug].review`, then emit one
aggregate `review.finish` event. Do not equate a completed ledger with an approving verdict.

### Review recovery

Resume order is recover → GitHub final-marker reconcile → `next`. First inspect pipeline
run-directory pids and reattach to any live call without spawning a duplicate. Run `recover`
for completed metadata/results that have no row; a live process is never a completed run.
Then query all PR reviews on GitHub for the phase-scoped final marker. If present, recover its actual event/URL
into the final run's status, record it idempotently, and promote the repository result; do not
post again even if the process crashed before writing status. Only then call `next`.

Trust the phase-scoped ledger, completed artifacts, and git/GitHub evidence over stale
bookkeeping: never rerun a live pid, repeat a remediation push, or duplicate the final review.
If remediation pushed before its status/row was written, reconcile its recorded head evidence
with local and remote integration HEAD and recover `PUSHED|NOOP` before continuing, without
invoking the fixer again. If the evidence cannot prove completion, block for reconciliation.
Keep the round's persisted head on inner-loop resume; capture a new head only at the next
outer-round boundary after any verified fix. Replay, rather than a cached stage, drives recovery.

### Plan review uses the same AR core

`loop-plan-review.sh run --round r [--inner M]` drives one independent outer round synchronously
with its own ledger and `ar_next --start-round r --single-round`. It prints TSV
`status <converged review path> <ledger>`, status precedence `disputed` > `findings` > `clean`;
exit 2 means no usable review. Plan review uses standard provider slots at effort `high`.
The planner verifies the converged review and disputed items against code, edits the plan
between outer rounds, and runs up to `LOOP_PLAN_REVIEW_ROUNDS=2`, stopping early on `clean`.
The reviewer/critic never edit the plan; only the planner does. Provider failure is reported,
never substituted. The PR final-post workflow does not apply to plan review.

## Merge policy (loop-merge.sh, serialized — one merge at a time)

Entry requires the persisted merge-risk disposition described above. This adds focus and
observability only: serialized merge and merged-tree Verify remain unconditional.

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
  An `adopted` integration worktree is the user's: never discard its changes; HIL pause.

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

## Pause & live control

User control arrives as files under `.loop/control/` (written by the daemon's
`POST /control`, e.g. from `loop-top`, or by hand); runners and loop-orchestrator.sh poll them
on their ~2s watchdog tick.

- **Phase pause** (`control/pause-<N>`): the runner kills the engine tree and exits 30;
  `meta.json` keeps `sessionId`. The SUB marks the phase `paused`, never retries/escalates,
  and never schedules a phase whose `control/pause-<N>` exists. Each tick, a `paused` phase
  whose pause file is gone (and the loop is not paused) resumes with `loop-runner.sh --resume
  <sessionId> --engine <engine>` in a new attempt dir (fresh start if no `sessionId`). R/C/T
  review calls instead restart fresh at the pinned head with their prior exchange inlined.
- **Loop pause** (`control/pause`): loop-orchestrator.sh kills the SUB tree — neither a crash
  nor a recycle — emits `loop.paused`, waits for the file to clear, emits `loop.resumed`, and
  respawns a SUB with the `resume` prompt. Every runner (review runners included) halts itself
  via the same file (exit 30). Supervise spawns no review/remediation runners while it exists
  and reports "paused" in its progress read.
- **Live notes:** when `notes/<N>.md` changes during a running attempt, the runner kills the
  engine and resumes the same session with "Steering note updated…" plus the note — same
  attempt, no retry consumed, deadline unchanged — and emits `note.applied`. For R/C/T,
  restart a fresh call with the updated note and inlined exchange, never resume the session;
  preserve the frozen head and original deadline.
- **Model override** (`control/model-<N>`): honored only before the phase starts; see § Model
  chains.

## HIL pause

1. Write `hil/<phase-slug>.md`: context, attempt table (engine → outcome), ONE blocking
   question, options A/B/C + recommendation, "reply in the orchestrator chat or write
   `hil/<phase-slug>.answer.md`".
2. Flip the phase to `[status: blocked]` and repush the plan doc.
3. `loop-notify.sh --level question --run-id <runId> --event hil.raise --title "HIL: <phase>" --body "<question>"`.
4. Ask in-session (AskUserQuestion / plain question). Other lanes keep running. On answer:
   phase back to ready with the answer injected; status back to `in-progress`.

## Backend mapping

One backend: the **central daemon** + `.loop/plan.md`. **Runners** own phase-scoped ops via
the auto skills; the **orchestrator** owns the plan document and everything cross-phase:

| Operation | Behaviour |
|-----------|-----------|
| Claim (runner, via `loop-pickup --auto`) | orchestrator pre-creates the lane branch; the phase flips to `claimed` in state.json (`loop-state.sh`) + a `[status: claimed]` marker in `.loop/plan.md`; lane collisions are caught by the branch/worktree conflict-check. |
| Runner handoff (via `loop-handoff --auto ... status:in-progress`) | `loop-plan.sh note --plan-id <runId> --phase <N> --body <progress>` (a `progress.note` event) + update the `[status: ...]` marker in the local `.loop/plan.md` copy; **phase-scoped only** — no plan-doc rewrite. |
| Post-merge status (orchestrator) | flip `[status: done]` in `.loop/plan.md` + append Progress log + `loop-plan.sh push` (re-register/overwrite the plan on the daemon) + a `progress.note`. |
| PR link (orchestrator, at effort completion) | record each `repositories[slug].prUrl`; with milestones the PR is per milestone per participant and its URL also goes into that milestone's `PR` bullet. |
| Milestone gate (supervise, after the gate review aggregates) | `state.awaitingMerge` + plan Status `awaiting merge` + `loop-plan.sh push` + `milestone.awaiting-merge` event + best-effort `ship` explainer + notify. Keep `sub/REVIEW_READY`, unlock, stop (no `finish`). |
| Completion (orchestrator) | one `loop-state.sh finish` with plural repository reviews after every AR pipeline reaches its final Opus review; plan remains `integrating`. |

"done" in loop mode = merged into the phase repository's integration branch. Humans merge
one PR per repository.

## Crash-resume

`loop-execute resume`: re-acquire the lock with the run id from state.json (a different
owner means another orchestrator claims this effort — stop and ask before `--force`), then
reconcile the sources — **git > plan/daemon > state.json** (git is
truth for code, the plan/daemon for claims + phase status, state for attempt bookkeeping):

- attempt marked running: pid alive → re-attach (watch transcript mtime; stale >25m → kill,
  treat as 124); pid dead + status.json present → process it normally; pid dead + none →
  failed attempt.
- `MERGE_HEAD` in any repository integration worktree → finish or abort before anything else.
- Phase `done` in the plan/daemon but lane branch not merged (or
  vice versa) → repair from git.
- Review pipeline: follow § Review recovery in order: recover → GitHub final-marker reconcile → `next`.
  Reattach to live pids; record completed-but-unrecorded metadata before selecting work.
  Reconcile a completed remediation push from git/remote evidence without invoking it again.
  Recover a posted final marker's event/URL and repository outcome without posting again.
  R/C/T use fresh calls with inlined exchange when continuation is needed, never session resume.
- `state.awaitingMerge` set → parked at a milestone merge gate: do not respawn the orchestrator
  and do not re-run the gate review; a SUB writes `blocked` and exits, only a human resume leaves
  the park. Applies while `state.awaitingMerge` is set, whatever the plan or daemon status says.
- Gate review `done` for the current milestone but `state.awaitingMerge` unset (crash before
  parking finished) → recover `awaitingMerge` from the gate review's recorded PR URLs, persist it
  (`check` reads it from disk) and redo the park's Status push, event, `ship` explainer and
  notification (idempotent). Never re-run the gate review.
- `state.resumePending` equals `state.milestone` (crash after the advance patch was applied, before restart)
  → run resume step 5 only (plan Status push, `milestone.resumed`, clear the marker, respawn), then
  clear `resumePending`. Milestone history and `sub/REVIEW_READY` are not evidence: both persist
  through every later milestone's own review barrier.
- Phases of a milestone whose gate PRs are merged are permanently shipped: never re-check them
  against a later integration tip (squash/rebase merges rewrite it) and never re-run them.
  Reconcile git against the current milestone's integration branches only.
- `loop-milestone.sh advance` interrupted (state still has `awaitingMerge`, the next milestone
  not yet applied) → re-run `check` then `advance`; prepare and retire are idempotent, state is
  written once by the final `loop-state.sh set`.
- Runners are spawned detached, so they survive orchestrator death; never double-spawn a
  phase whose run dir has a live pid.
