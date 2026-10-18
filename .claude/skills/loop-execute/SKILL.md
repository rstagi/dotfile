---
name: loop-execute
description: >-
  Loop engineering: autonomously execute an editable multi-phase plan registered on the Loop
  daemon end-to-end — spawn a fresh headless runner per phase (Codex
  or Claude, parallel lanes in separate worktrees, cap 3), verify and merge each lane into its
  repository integration branch, escalate stuck work (retry → stronger model → HIL pause),
  open one effort PR per repository, execute each explicit review phase as a tiered adversarial
  pipeline (standard/max, rotating reviewer ↔ critic, N outer rounds) with remediation between
  rounds, post one final Opus
  verdict, then stop for humans to merge.
  By default the heavy orchestration runs in a detached, self-recycling sub-orchestrator so
  the interactive chat stays thin. Use when asked to
  "run the loop", "execute the multi-phase plan", "run the published plan autonomously",
  "loop-engineer this", or after multiphase-plan when the user wants the phases executed
  hands-off. NOT for implementing an in-chat plan yourself — it needs a registered plan with a
  Loop config.
argument-hint: "<project / plan doc> | resume | status | abort"
---

# Loop Execute

The execution engine for `multiphase-plan`. The plan backend is the **Loop daemon** (+
`.loop/plan.md`). The human plans once and may keep editing;
this skill runs work phases through pickup → implement → handoff and treats explicit review
phases as orchestrator-owned barriers. Each headless runner runs the pickup/handoff auto modes itself;
you — the orchestrator session
— are the sole writer of the plan document and the integration branch, and the policy brain.
Scripts do the mechanics. The full contract (dir layout,
status.json, exit codes, chains, prompts) is `references/loop-protocol.md` — read it
before starting and follow it exactly.

**Division of labor:** scripts (`~/dotfile/loop-runner.sh`, `loop-orchestrator.sh`,
`loop-merge.sh`, `loop-notify.sh`, `loop-state.sh`) are mechanism; never shell raw
`claude`/`codex`/`git merge` yourself. You are policy: what to schedule, whether a result is
acceptable, how to answer a runner's question, when to escalate, when to wake the human.

## Modes

Heavy orchestration accumulates context; on a long plan a single interactive session blows past
its safe ceiling. So `loop-execute` runs in one of three shapes (contract:
`references/loop-protocol.md` § Two-tier orchestration):

- **`supervise`** (default for a hands-off run) — the interactive session you're in stays
  **THIN**. It does preflight / init / lock / the confirm gate, spawns ONE detached
  `loop-orchestrator.sh`, then only: polls **compact local** state for a 2-3 line progress read,
  answers HIL from on-disk briefs, and handles `sub/REVIEW_READY` barriers. It spawns **only** the
  orchestrator, ledger-driven AR review/remediation runners (§8) and milestone explainer
  runners (§8, §9) — the heavy scheduling/merge/escalation runs in
  disposable **`sub`** instances that self-recycle, so this chat never fills up.
- **headless `sub`** — a disposable orchestrator instance (`claude -p`, `CHAIN_ORCHESTRATE`)
  spawned by `loop-orchestrator.sh`, entered via `loop-execute resume` (signalled by env
  `LOOP_SUB=1`). It runs steps 4-7 (schedule / handle exit / merge / escalate), self-measures
  its own context each tick, and recycles at a safe boundary (§ Sub recycle below). It NEVER
  runs pr-review and NEVER `AskUserQuestion`.
- **single-tier** (`<plan> --inline`, or when `loop-orchestrator.sh` is absent) — the
  interactive session does everything itself (steps 1-9 inline, no orchestrator, no recycle).
  Fine only for a plan small enough that the chat won't approach its ceiling.

**Argument → mode:** `<plan>` → `supervise`. `<plan> --inline` → single-tier. `resume` → `sub`
when `LOOP_SUB=1` (loop-orchestrator's child), else a human re-attach in `supervise`/single-tier.
`status` / `abort` are mode-agnostic. Below, steps **4-7** are "run by the `sub` instance (or
inline in single-tier)"; steps **1-3, 3b, 8** are supervise/single-tier.

## Prerequisites

`gh auth status` OK; `jq`; the loop scripts (including `loop-repo.sh`) present; a plan
(registered on the Loop daemon or in `.loop/plan.md`) authored by `multiphase-plan` with
Loop config + repository blocks: shared integration branch, concurrency, and mandatory
Verify per repository. Missing repository verify → ask; never invent one. Legacy scalar
plans normalize to the synthetic `primary` repository. Long runs: remind the user once to `caffeinate`
the Mac; do not manage power yourself.

## Workflow

### 1. Preflight

Resolve the plan LOCAL → daemon: `.loop/plan.md` header → the daemon
(`loop-plan.sh get --plan-id <id>`, else `loop-plan.sh list`; argument → ask, like
`loop-pickup` step 1). Parse phases, kinds, lanes, repositories, `Depends on` edges,
statuses, and per-phase `Verify`. Validate: DAG acyclic;
every phase has *Done when*; every work phase has exactly one known repository; the final
document phase is `[kind: pr-review]`. Legacy plans without explicit kinds retain their
synthetic terminal review. A phase that an `[archived]` stub lists is an existing phase with status `done`.
It needs no *Done when* and no repository. A `Depends on` edge to it is satisfied. Do not
read the archive files (`.loop/milestones/m<N>.md`) for validation. Resolve each `owner/repo` through
`loop-repo.sh get`; before autonomy ask once for missing checkout paths and persist them
with `loop-repo.sh map`. Run `loop-repo.sh check` for origin/default-branch validation; it
reports `mode` `create` or `adopt` (branch already exists, e.g. the ws worktree you run from —
`adoptWorktree` is where it is checked out) and refuses a dirty adopted worktree or the
default branch as integration branch (exit 4).
**Milestones.** A plan with `## Milestone N — <outcome>` sections carries several milestones
(a plain `## Phases` plan is one implicit milestone 1: skip this paragraph, nothing changes).
A milestone's participants are the repositories of its work phases. Validate: every
non-final milestone ends with one `[kind: pr-review]` `[gate: merge]` phase; every root phase
of milestone N+1 must list that gate in `Depends on` **explicitly** — missing → reject, never
infer it; milestones ≥2 declare their integration branch (milestone line or per-repository
`(owner/repo)` line; no global fallback); a branch never repeats for the same repository
across milestones; `[gate: merge]` on the final milestone → warn (nothing to park for; it runs
as an ordinary terminal review). Do not apply the gate and root-phase checks inside an
archived milestone. Its stub names its gate (plan-format → Archived milestones). Branch precedence: milestone repository line > milestone line >
(milestone 1 only) `## Repositories` override > Loop config. Only milestone-1 participants
go through `loop-repo.sh check`/adoption; M≥2 branch names are checked by name only — no
`check`, no `prepare` (their branches and worktrees are created fresh at resume).
Ensure coordinator `.loop/` is gitignored and repository worktrees are clean.
**Adopt the plan's
`planId` as the run id** — the planId IS the runId, so loop-execute drives the same daemon
record through planned → active → finished (one selector entry). If it is already `finished`,
re-read the plan: newly appended unfinished phases reopen that same record; only an unchanged
intentional re-run mints `<planId>-r<K>`. Take the lock now —
`loop-state.sh lock --owner <run-id>`: a foreign owner means a live orchestrator already runs
this effort; surface it and stop (never `--force` silently). This happens before the confirm
gate so the gate's no-further-contact promise holds.

### 2. One confirm gate, then autonomy

Show the user: the plan id, repository → checkout/default
branch/integration branch/create-vs-adopt/verify table (adopt: the adopted worktree path and
its commits ahead of default — they become part of the PR), phase/kind/lane/repository table, chains +
budget/timeouts from `loop-models.conf`, concurrency (the plan's **Concurrency** line
overrides `LOOP_MAX_PARALLEL`; default 3). With milestones also show a milestone → gate /
participants / integration-branch table and that the loop **parks** at every merge gate for a
human merge + `/loop-execute resume`. After their go, do not contact them again
except through the HIL path or completion. `AskUserQuestion` is reserved for those two
moments.

### 3. Init

`loop-state.sh init` with runtime state v2 from the protocol; set `state.milestone = 1` (also
for milestone-less plans). Record `state.runtimeRepo` + `state.runtimeRoot` only when the effort changes the Loop runtime
(self-hosted: the repository whose integration worktree is `RUNTIME_ROOT`, and that path, once
prepared); otherwise leave both `null`. For every repository — with milestones, the milestone-1 participants only — run
`loop-repo.sh prepare --repo <owner/repo> --run-id <runId> --integration-branch <branch>`.
It fetches GitHub's default ref and either creates the integration branch/worktree from that
exact fetched SHA under `~/.loop/worktrees/<runId>/<owner--repo>/integration`, or adopts the
existing branch/worktree (ff-only to the fetched default when it has no unique commits;
refuses divergence from default or from its remote); never use stale local `main`. Push each
initial integration branch and persist the returned repository fields (including `adopted`)
in `state.repositories[slug]`. Adoption of an existing branch/worktree is allowed only for
milestone 1; milestones ≥2 are never prepared here. When the adopted worktree is the coordinator checkout itself,
`.loop/` lives inside it (gitignored) and the human must not edit that worktree while the
loop runs. Later phase merges go through loop-merge; only a review-phase remediation
coordinator may also fast-forward-push its verified fixes. Set plan
**Status: in progress**, then repush once.

**The observer runs itself (on by default).** `loop-state.sh init` sources `loop-emit.sh`,
which runs `loop_ensure_daemon` (starts the central Loop daemon if it isn't already up) then
POSTs `/api/loops/<runId>/register` and seeds state — flipping this loop's daemon record
from `planned` to `active` (authoritative, never-stale) the moment `.loop/` exists. Print `http://localhost:7717` once so
the human can open the **Loop Observatory** and pick this loop from the selector. It's an
optional read-only dashboard that never blocks the run; printing a URL is not a "contact"
that breaks the confirm gate's promise.

### 3b. Supervise: launch the orchestrator, then poll thin

**(supervise mode only — single-tier falls straight through to step 4 inline.)** The interactive
session hands the heavy work to a detached orchestrator and stays thin:

1. **Seed the SUB prompt.** Write `.loop/sub/prompt-seed.md`: the headless-`sub`
   instructions (invoke this skill in `sub` mode → `loop-execute resume`; run steps 4-7; recycle
   per § Sub recycle; never pr-review; never `AskUserQuestion`) plus the plan Goal + key decisions.
2. **Spawn once, detached** (mirror the protocol's detach idiom):
   `nohup ~/dotfile/loop-orchestrator.sh --run-id <runId> --dir .loop --prompt-file
   .loop/sub/prompt-seed.md > .loop/sub/orch.log 2>&1 & echo $! >
   .loop/sub/orch.pid; disown`. It runs SUB instances sequentially, respawning on
   recycle/crash (exports `LOOP_SUB=1` + `LOOP_SUB_DIR`).
3. **Poll thin.** Every ~60-120s read **compact local** state only — `loop-state.sh get
   '.phases|map(.status)'` + a couple of `events.jsonl` tail lines — and summarize to 2-3 lines.
   Do NOT skim diffs, resolve conflicts, or pull the fat daemon snapshot; that heavy work is the
   SUB's, and reading it here defeats the purpose. If `sub/orch.pid` is dead and there is no
   `sub/REVIEW_READY` or legacy `sub/PHASES_DONE`, re-spawn the orchestrator (it `resume`s cleanly — the dead-orchestrator
   backstop). It does not respawn while `state.awaitingMerge` is set: the loop is parked at a
   milestone merge gate and only a human `/loop-execute resume` continues it. While `.loop/control/pause` exists, report "paused" and spawn no
   review/remediation runners (protocol § Pause & live control).
4. **HIL asker.** Each poll, scan `hil/*.md` lacking a sibling `.answer.md`. For each: read the
   brief, `AskUserQuestion` (the ONLY user contact besides completion), write the reply to
   `hil/<slug>.answer.md`. The `sub` picks it up and requeues that lane — the human-facing HIL
   lives HERE in supervise, never in the SUB.
5. On `sub/REVIEW_READY` → reconcile the plan once more, then go to **step 8**. A late edit
   that makes the review no longer ready clears the marker and respawns the orchestrator.
   Legacy `sub/PHASES_DONE` uses the legacy final-review path. On `fatal`/`blocked` with no
   path forward, surface it and stop.

### 4. Schedule *(run by the `sub` instance — or inline in single-tier)*

At every safe scheduling boundary, re-read the newest plan (daemon)
and reconcile it into `state.phases` without replacing existing entries. Phase numbers are
stable identities; document order is the sequence. Add each new phase as `todo`, then repush. Preserve completed and running state.
Never drop, reopen or renumber an archived phase. Keep its `state.phases` entry. If
`state.phases` has no entry for an archived phase, add it as `done`, never as `todo`.

Enforce review topology as one coherent plan edit:

- Last review pending → place new work immediately before it and extend that review's
  dependency closure. Reuse the review.
- Last review done → keep it where it ran, place new work after it, and append a fresh
  terminal `[kind: pr-review]` phase using the next unused phase number.

Never renumber, move, or reopen a completed phase. If an edit races a live runner or changes
its body/repository/branch, preserve the running snapshot and apply it at the next safe
boundary; incompatible edits raise HIL. A review phase follows normal readiness rules, but
the SUB yields it to supervise with `status.json{outcome:"review-ready"}`.

With milestones the SUB schedules only phases of `state.milestone`: the gate's explicit
`Depends on` edges keep milestone N+1 blocked until the gate is `done`, and nothing of N+1 is
prepared or started while the loop is parked. A SUB that finds `state.awaitingMerge` set
schedules nothing: it writes `status.json{outcome:"blocked"}` and exits —
`loop-orchestrator.sh` treats that as a handled park, not a crash.

A work phase is READY when `[status: todo]`, all its `Depends on` phases are done, its lane has
no phase running, and fewer runners are live than the concurrency cap (plan's
**Concurrency**, else `LOOP_MAX_PARALLEL`). For each READY phase:

1. Lane worktree on the Suggested branch, cut from the phase repository's **integration
   tip**. Reuse it only when the prior lane phase used the same repository. On a repository
   hop, remove the old worktree and create the next from the target repository tip.
2. Generate `runs/<phase-slug>-a<K>/prompt.md` from the protocol's skeleton — it opens
   with `loop-pickup --auto` (the runner claims its own phase) and closes with `loop-handoff --auto ... status:in-progress` +
   status.json. Include the verbatim phase block + plan Goal/decisions +
   `notes/<phaseNumber>.md` verbatim on **every** attempt, including the first. Retry-only
   context (`answers/`, verify.log tails, HIL answers) remains separate.
3. Spawn detached per the protocol's Runner-spawn section (nohup + pid file):
   `<runtime-root>/loop-runner.sh --worktree <wt> --run-dir <abs> --prompt-file <p> --run-id
   <runId> --phase <N> --repository <owner/repo> --chain task --verify-cmd
   '<phase-or-repository verify>'` (it inherits
   `LOOP_DAEMON_URL` via env, so its EXIT-trap `phase.attempt.*` events reach the daemon).
   Record pid + run dir in state; journal the event. A pickup `REFUSED` surfaces as the
   runner's `blocked` status → ladder, keep scheduling other lanes.
   Never schedule a phase whose `.loop/control/pause-<N>` exists. A `control/model-<N>`
   override is applied by the runner itself (no extra flag).

   Resolve `<runtime-root>` once during preflight. Normally it is `~/dotfile`. When the
   effort modifies `rstagi/dotfile` Loop runtime files, use that repository's integration
   worktree after each lane merge. Read this skill and `references/loop-protocol.md` from
   the same root when composing later SUB/runner prompts. The runner then resolves
   `loop-models.conf`, `loop-jev.mjs`, and `loop-emit.sh` beside itself, exercising the
   merged runtime without copying into or editing the installed checkout.

There are no completion notifications from detached runners — monitor by polling per the
protocol: `meta.json` appearing means the attempt ended; a `transcript.jsonl` staler than
the protocol's threshold means a hung runner (kill the pid tree, treat as 124). Do not
preempt the runner's 30-minute phase checkpoint.

### 5. Handle a runner exit

Switch on the exit code (protocol table). The extra checks only you can do:

- **exit 0** — run the pre-merge gates in this exact order; none is conditional on Jev:
  verified exit 0 → one merge-risk judgment for this attempt/head → **full** diff skim →
  `headBefore/headAfter` stall check → clean-worktree check → reread steering notes →
  serialized merge → merged-tree Verify. Build the bounded judgment input with
  `loop-jev-risk-input.mjs` from *Done when*, every changed path, aggregate diff statistics,
  and the successful verification summary. It omits all patch contents; pipe it directly to
  `loop-jev.mjs` without persisting the request or raw vendor response. Ask only `scopeGap`
  (`none|possible|likely`) and `changeRisk` (`low|medium|high`). In active mode, use valid
  advice only to focus the mandatory full `git diff <base>...HEAD` skim. Shadow, off, error,
  malformed, and low-confidence results use the unfocused full skim. After that full skim,
  the stall/clean checks, and rereading `notes/<N>.md`, validate the evidence and actual
  disposition with `loop-jev-risk.mjs`. It rejects missing/reordered gates, dirty or stalled
  attempts, head mismatches, and unverified results. Atomically persist its output as
  `<runDir>/risk-decision-<head>.json`, then emit it with `loop_emit_jev_decision`. Reuse an
  existing valid record only for the same attempt and head; a retry or new head gets a new
  judgment and record. A builder/policy/Jev failure falls back to a typed `full-diff-skim`
  disposition and never blocks these deterministic checks. Never send transcripts, logs,
  environment values, credentials, or patch contents to TypeSafe.
- **exit 10** — read `status.json`'s question. For `checkpoint:true`, read `checkpoint.md`
  when present and inspect the worktree; give a concrete next step from the plan and code.
  If the phase made no meaningful progress over two work blocks, escalate instead of
  resuming indefinitely. Checkpoints bypass Jev and do not consume the three decision-question
  rounds. For other questions, enforce the three-round cap first, increment the round, then
  ask `loop-jev.mjs` one bounded `question` choice: `plan-answer`, `code-investigation`,
  `human-preference`, or `uncertain`. Inspect the actual question and relevant plan before
  acting, and inspect relevant code before a code-backed answer. A `human-preference` label
  never raises HIL by itself. Shadow, error, low-confidence, and `uncertain` advice use the
  existing plan/code path. Validate the disposition with
  `loop-jev-question.mjs`; it rejects evidence-free answers and HIL outside L4. Persist its
  typed suggestion-versus-action record atomically as `<runDir>/question-decision.json`, emit
  it with `loop_emit_jev_decision`, and only then act. Resume either kind **in a fresh attempt dir**
  (protocol Q&A-resume): `loop-runner.sh --resume <sessionId> --engine <meta.engine>
  --run-dir <new a<K+1>> ...` with your answer as the prompt. If the session cannot
  resume, start fresh with the answer prepended.
- **exit 12** — relaunch once with the verify.log tail in the prompt; second failure →
  L3.
- **exit 20 / 50×2 / 124×2** — escalate per the ladder. **exit 40** — follow the
  protocol's exit-40 rule (bounded backoff, then L3; multi-lane 40s pause scheduling).
- **exit 30** — user pause: mark the phase `paused`, never retry/escalate. Each tick, once
  `control/pause-<N>` (and loop-level `control/pause`) is gone, resume with
  `loop-runner.sh --resume <sessionId> --engine <meta.engine>` in a new attempt dir (fresh
  if no `sessionId`).

### 6. Merge and sync

Immediately before merging phase `<N>`, re-read `notes/<N>.md` and honor it (for example,
rebase the lane onto its repository integration tip first). This catches notes dropped after the
runner started. After the merge is complete and the phase is promoted `done|merged`, run
`loop-state.sh note --dir .loop --clear <N>` as best-effort housekeeping.

The merge-risk record is advisory proof of the completed pre-merge inspection, not a merge
permit. Low risk, fallback, policy error, or low confidence cannot skip merge serialization or
`loop-merge.sh`'s repository Verify. Do not call `loop-merge.sh` until the risk record exists.

Serialize merges globally (one at a time), targeting the phase repository.
`loop-merge.sh --worktree <repo-int-wt> --lane-branch <b> --run-id <runId> --phase <N>
--repository <owner/repo> --verify-cmd '<repository verify>'` (inherits `LOOP_DAEMON_URL`;
its EXIT trap emits `phase.merged` on rc 0, `merge.conflict` on a conflict):

- **0** → run `loop-handoff --auto phase:<N> status:done lane:<X> engine:<engine>`
  inline — the orchestrator context of that skill (you are the plan's sole writer; runners
  already posted their phase-scoped notes). It flips the plan's `[status: done]` marker +
  `loop-plan.sh push`, appends `.loop/progress.md`, and `loop-plan.sh note` noting lane + engine.
  Cleanup removes the worktree on a repository hop; otherwise it may switch to the next lane
  branch. First merge in each repository → `gh pr create --draft` from that repository's
  integration branch, PR URL into `state.repositories[slug]` + its plan block. Later merges
  update only that PR. With milestones: one PR per milestone per participant, created on that
  milestone's first merge in the repository; body = Goal + that milestone's phases only; the
  URL goes into `state.repositories[slug]` and the milestone's `PR` bullet (`- **PR (owner/repo):**`
  when several repositories participate), never the `## Repositories` `PR:` line. Schedule globally by DAG/lane capacity.
- **2** → conflict. **`sub` mode: spawn a detached merge-runner** (protocol's
  Merge-runner skeleton — `loop-runner.sh --chain escalate --worktree <int-wt>` with NO
  `--verify-cmd`), so the diff never enters your context; promote on the next tick when its
  `phase.merged` fires. **single-tier:** resolve inline (`resolving-merge-conflicts`, honoring
  both phases' *Done when*), then `loop-merge.sh --worktree <repo-int-wt> --finish
  --repository <owner/repo> --verify-cmd '<repository verify>'`. Either way, not confident
  it's semantically right → `--abort` + HIL.
- **4** → semantic conflict: `git revert -m1 HEAD` in the integration worktree, then L3
  for this phase with the verify output.
- **3 / 1** → reconcile from git per the protocol (already-merged → done; dirty
  integration worktree → clean it, except an `adopted` one: never discard its changes, HIL
  pause instead), don't treat as a conflict.

### 7. Escalate

Ladder per protocol: L0/L1 live inside loop-runner. Yours: **L2** answer-and-resume (step
5); **L3** Fable takeover — fresh attempt, `--chain escalate`, prompt carries a distilled
post-mortem of prior attempts (last.md + verify tails, never raw transcripts), one shot;
**L4** HIL — write `hil/<phase-slug>.md` per protocol, post it as a task comment, flip
`[status: blocked]` + repush, `loop-notify.sh --level question --run-id <runId> --event
hil.raise`. **Only that lane pauses; other lanes keep running.** Then, to get the answer:
**`sub` mode NEVER `AskUserQuestion`** — it poll-waits for `hil/<slug>.answer.md` (which the
supervise main writes via its HIL asker, step 3b), staying on other lanes meanwhile; if HIL is
the *only* thing left and no lane can progress, write `status.json{outcome:"blocked"}` and let
supervise carry it. **single-tier:** ask the user in-session (`AskUserQuestion`). On answer,
requeue the phase with it in context.

### 8. Execute the review phase *(supervise / single-tier — never the `sub`)*

In supervise mode the `sub` stops at the next ready explicit review phase with
`status.json{outcome:"review-ready"}`; `loop-orchestrator.sh` touches `sub/REVIEW_READY`, and
the supervise main runs this step. The `sub` never runs pr-review.

Re-fetch and reconcile the plan before spawning reviewers. If new work was inserted before
this pending review, clear `REVIEW_READY` and resume scheduling. Otherwise mark every
repository PR ready on the first review round; later rounds reuse those PRs. Each PR body
contains the Goal and only
that repository's phases (with milestones: that milestone's phases in that repository) plus a progress digest. Then record completion: flip plan
**Status: integrating** + `loop-plan.sh push`, append `.loop/progress.md` — the daemon learns
completion from `loop-state.sh finish`'s `loop.finish` below.

Sweep remaining lane worktrees. Re-read `notes/<reviewPhase>.md` plus legacy repository
review-note aliases, honor them against each integration worktree throughout this review phase,
and fold them verbatim into every reviewer/remediator prompt.

**Self-hosted review:** when this effort changes Loop itself, read §8 and the protocol from `RUNTIME_ROOT` at `REVIEW_READY`
(`.claude/skills/loop-execute/SKILL.md` and
`.claude/skills/loop-execute/references/loop-protocol.md` there), even if you
loaded an installed skill earlier. Use the merged runtime's `loop-review.sh`, `loop-runner.sh`,
configuration, and prompt references. A pipeline failure follows HIL; do not silently fall back
to the old workflow.

Read the review phase's tags: `[review: standard|max]` (default `LOOP_REVIEW_TIER_DEFAULT`,
standard), `[rounds: N]` (default `LOOP_REVIEW_ROUNDS_DEFAULT` (2)), and `[inner: N]` (default
`LOOP_REVIEW_INNER_DEFAULT` (2)). Both limits are 1..5, with rounds capped by
`LOOP_REVIEW_ROUNDS_CAP`. Legacy tiers shallow and medium are rejected, never translated.
For a new phase persist each `repositories[slug].reviewPipeline` using
`reviewPipeline = {phase, tier, rounds, inner, headSha, ledger}`, with ledger
`runs/review-p<N>-<owner--repo>.ledger.tsv` relative to the loop directory. Resume uses these
pinned inputs, not re-read tags or a cached stage cursor. The six-column ledger is authoritative.

Drive each repository as `recover` → `next` → spawn → `record`:

1. Inspect existing pipeline pids first; reattach to live calls without spawning duplicates.
   Run `loop-review.sh recover --ledger <F> --runs-dir <runs>` to record completed,
   unrecorded artifacts (`meta.json` + `status.json`, successful engine completion).
   On resume reconcile the phase-scoped final marker on GitHub before calling `next`
   (§9 and protocol § Review recovery). Incomplete metadata is not proof of success.
2. Call `loop-review.sh next --tier <T> --rounds <N> --inner <M> --ledger <F>`.
   It returns TSV `a<k> stage role chain`, or `done`; derive provider from
   `review-gpt|review-claude|review-grok`, using `claude` for `review-fix|review-final`.
   It writes capped disagreements and their synthetic ledger row itself: never spawn a `disputed` runner.
   A synthetic row consumes an attempt number. Use the returned `a<k>`, never a precomputed list.
3. For a new `round<r>` opening `review`, capture the current PR head and persist `headSha`;
   keep that SHA frozen throughout the inner loop, including responses and confirmation.
   Refresh it only at the next outer-round boundary after any verified fix. If the artifact
   moves unexpectedly within a round, block for reconciliation rather than mixing heads.
   Render `loop-review.sh prompt --role <R> --pr <url> --head <sha> --notes <file> --ledger <F> --run-dir <D>`
   into the run's prompt (a readable empty notes file when none). Add execution inputs for
   fix/final: integration worktree, branch, remote, and repository verify command verbatim.
4. Spawn a detached `loop-runner.sh` in `runs/review-p<N>-<owner--repo>-a<k>/`, passing
   `--chain <chain> --review-tier <T> --phase <N> --repository <owner/repo>`.
   Within a repository all R/C/T runs are sequential; different repositories may progress in
   parallel under Loop Concurrency. Every R/C/T call is fresh, with the prior exchange inlined
   by `ar_prompt`; never use provider-session resume. Wait for completion before continuing.
5. Record with `loop-review.sh record --ledger <F> --run-dir <D> --stage <S> --role <R> --provider <P>`.
   Only after successful record call `next` again. Fix recording verifies remote HEAD through
   `git ls-remote`; final recording saves the actual posted event + URL even for a question or
   blocked verdict. Duplicate recording is idempotent. `done` ends execution, not verdict assessment.

A `record` (or `recover`) exit 2 means an invalid last verdict line: append no row and
re-run the same step once with a format reminder. Before retrying, preserve the invalid output and retry-used evidence
in that run's artifacts (outside files the runner overwrites), so a crash cannot reset the
allowance. Reuse the same `a<k>`/stage/role/provider/head, archive the old metadata/status, and
start a fresh call. A second invalid verdict blocks that repository pipeline and takes the
normal HIL path; never interpret malformed output as agreement or advance past it.
Other infrastructure failures block that repository without cancelling siblings or substituting
models. For exit 10 with `checkpoint:true`, read the checkpoint report and give one concrete
answer; continue the same logical step/head without advancing the ledger. R/C/T use a fresh
call with prior exchange and answer inlined, never `--resume`; retain the original artifacts.

The CLI owns rotation (round 1 gpt→claude, round 2 claude→grok, round 3 grok→gpt), inner caps,
and the fix rule. R reviews/responds, C audits against code, and T confirms a candidate clean
opening pair (the first R review is `APPROVE` and C answers `AGREE`). R/C/T
**never post or mutate GitHub** or edit the artifact. T `AGREE` skips remaining outer rounds
and jumps to `final`; no skipped ledger rows are written. A T veto makes T the active critic
for the remaining inner loop. At the inner cap the disputed sidecar preserves both positions,
including critic-added issues; disputed > findings > clean. Only the fixer edits code between
outer rounds. For r < N, run `fix<r>` iff the last R verdict is `NEEDS_CHANGES` or the round is disputed;
otherwise proceed without a fix. The last round always goes to final, not another fix.

- `fix<r>` (`review-fix`): its prompt invokes `/pr-review-fix-all` with exactly one converged
  review, the current disputed sidecar only when that round has a dispute row, and these inputs:

  ```text
  - report: <absolute last R report.md of round r>
  - disputed: <absolute current .disputed.md, if any>
  - tier: <standard|max>
  - run-dir: <absolute fix RUN_DIR>
  - worktree: <absolute repository integration worktree>
  - branch: <integration branch>
  - remote: origin
  - verify: <repository verify command, verbatim>
  ```

  Do not implement remediation outside that skill. The Opus coordinator verifies every finding
  and disputed item against current code before accepting/rejecting it, delegates accepted fix
  groups to Opus fixers (max 3), integrates, verifies, commits, and fast-forward pushes.
  Dispositions require evidence; no accepted finding may be silently skipped. Status must
  include `result:PUSHED|NOOP`, full `headBefore`/`headAfter`, remote, branch, and worktree so
  `record` can verify remote HEAD. A no-op makes no empty commit or push.
- `final` (`review-final`): receives the last converged review + current disputed items and
  current code at that round's pinned head. Earlier-round reports are stale history. It
  independently verifies each finding and disputed item, then posts the pipeline's **only**
  GitHub review: `REQUEST_CHANGES` for unresolved `[blocker]`; otherwise `COMMENT` for
  unresolved `[major]`; otherwise `APPROVE`. Minor/nit/style findings do not prevent approval.
  Include the protocol's phase-scoped idempotency marker; query all PR reviews for it before
  posting, reusing an existing marked review. Save the API response URL in
  `status.json.commentUrl` and actual event in `status.json.event`.

After `final`, promote its outcome (`REQUEST_CHANGES → blocked`, `COMMENT → question`,
`APPROVE → done`), report path, and comment URL into `state.repositories[slug].review`, clear
that repository's review note, and emit one aggregate `review.finish` after aggregation.
A complete ledger does not imply approval.

Wait for every participating repository of this milestone. Aggregate verdict precedence is `blocked > question > done`. If any
repository is blocked, mark the explicit review phase `blocked` and use the normal HIL/retry path;
otherwise promote it to `done`. Repush after promotion. On a `[gate: merge]` review, park instead of respawning
(never the "later phases" path below) — park only when a later milestone exists; a gate on the
final milestone is an ordinary terminal review (finish below):

1. Persist `state.awaitingMerge {milestone, prs{slug:url}}` with one gate PR URL per participating
   repository (`loop-state.sh set`), and record the verdicts in `state.repositories[slug].review`.
2. Flip plan Status `awaiting merge` + `loop-plan.sh push`.
3. `loop-state.sh log milestone.awaiting-merge "" "M<n> <slug>=<url> ..."` — the daemon's typed
   `milestone.awaiting-merge` event. `log` only carries phase/repository/detail, so the milestone
   and gate PRs go in `detail` as one compact string (e.g. `M1 owner/api=<url> owner/web=<url>`).
4. **Ship explainer (best-effort).** Explain the milestone in `visual-explainer` `ship` mode
   (protocol § Milestone explainers). The output directory is `.loop/explainers/m<n>-ship/`.
   - Collect the retro facts (protocol § Milestone retrospective):
     `node "$RUNTIME_ROOT/loop-retro.mjs" --dir .loop --milestone <n> --json > .loop/explainers/m<n>-ship/retro-facts.json`.
     When the command fails, remove that file and go on without retro facts.
   - Supervise does not write the props. Reuse a `props.json` that passes the skill's
     `--check` when `.loop/milestones/m<n>.retro.md` exists. Otherwise spawn one detached
     explainer runner and wait for its `meta.json`. It writes the props and
     `.loop/milestones/m<n>.retro.md` from the same facts.
     Exit 12 gets one relaunch with the `verify.log` tail.
   - Check that `.loop/milestones/m<n>.retro.md` exists. When it is missing, write the reason
     to `retro-error.txt` in the output directory. A missing retro never blocks the park.
   - Then supervise renders, publishes and writes `link.txt`, as the skill tells it.
   - When the link is an artifact URL, add or replace one line in each gate PR body:
     `Explainer: <url> <!-- loop-explainer:m<n>-ship -->`.
   - On any failure, write the reason to `error.txt` in the output directory and go on
     with the park. A failed explainer never blocks the park.
5. Notify once (`loop-notify.sh --level info --event milestone.awaiting-merge`): PR URLs, the
   explainer link (`--url`) or the explainer failure, and the
   instruction to merge them and run `/loop-execute resume`.
6. **keep** `sub/REVIEW_READY` (it is what stops the step-3b backstop), unlock, stop — never `finish`, never respawn. A SUB or backstop that sees `awaitingMerge` does nothing.

Resume is human-only (`LOOP_SUB` unset) and starts the next milestone — step 9. Otherwise, if later
document phases exist, set plan Status back to `in progress`, clear `REVIEW_READY`, and respawn
the orchestrator for the next segment.

If this review is terminal, do the finish. A terminal review is the final review of the last
milestone of a plan, or the final review of a plan with no milestones.

1. Emit exactly one `loop-state.sh finish --json '{status:"integrating",repositories:{...}}'`.
   The daemon rejects the finish until the terminal explicit review is `done`. Keep plan Status
   `integrating` until humans merge every PR.
2. **Ship explainer (best-effort).** Explain the work since the last merge gate in
   `visual-explainer` `ship` mode (protocol § Milestone explainers). For a plan with no
   milestones, this work is the whole plan. The output directory is `.loop/explainers/m<n>-ship/`,
   where n is `state.milestone`.
   - Collect the retro facts (protocol § Milestone retrospective):
     `node "$RUNTIME_ROOT/loop-retro.mjs" --dir .loop --milestone <n> --json > .loop/explainers/m<n>-ship/retro-facts.json`.
     When the command fails, remove that file and go on without retro facts.
   - Supervise does not write the props. Reuse a `props.json` that passes the skill's
     `--check` when `.loop/milestones/m<n>.retro.md` exists. Otherwise spawn one detached
     explainer runner and wait for its `meta.json`. It writes the props and
     `.loop/milestones/m<n>.retro.md` from the same facts.
     Exit 12 gets one relaunch with the `verify.log` tail.
   - Check that `.loop/milestones/m<n>.retro.md` exists. When it is missing, write the reason
     to `retro-error.txt` in the output directory. A missing retro never blocks the finish.
   - Then supervise renders, publishes and writes `link.txt`, as the skill tells it.
   - When the link is an artifact URL, add or replace one line in the PR body of each
     participating repository: `Explainer: <url> <!-- loop-explainer:m<n>-ship -->`.
   - On any failure, write the reason to `error.txt` in the output directory and go on
     with the finish. A failed explainer never blocks the finish.
3. Notify once (`loop-notify.sh --level info`): the final PRs and verdicts, and the
   explainer link (`--url`) or the explainer failure.
4. Unlock, stop.

### 9. `resume` / `status` / `abort`

- **resume** — re-lock with the state's run id, then reconcile per protocol (git >
  plan/daemon > state): finish/abort any in-progress merge first;
  re-attach or fail dead attempts;
  repair status drift. Sweep `notes/<N>.md` for every phase already `done|merged`; this is
  idempotent housekeeping, not badge correctness. Never double-spawn a phase with a live pid. Call `loop-state.sh
  register` (idempotent ensure-daemon + re-register) so the loop reappears in the observer —
  no per-loop observer to relaunch. **If `state.awaitingMerge` is set** the loop is parked at a
  milestone merge gate (step 8): a SUB (`LOOP_SUB=1`) writes `status.json{outcome:"blocked"}` and exits;
  only a human resume (`LOOP_SUB` unset) leaves the park, via `loop-milestone.sh check`,
  `loop-plan.sh archive` and `loop-milestone.sh advance` (protocol § Milestones). **If `LOOP_SUB=1` (you are a SUB instance):** FIRST, if
  `sub/handoff.md` exists, read it **once** and rename it to `sub/handoff.consumed-<k>.md`
  (single-consumption); then reconcile as above and run steps 4-7 under the recycle loop below.

**Review resume** *(supervise / single-tier only)*. Reload §8/protocol from `RUNTIME_ROOT`
for self-hosted review, then replay each repository's pinned pipeline in this order:
recover → GitHub final-marker reconcile → `next`. First inspect live run pids and reattach;
never rerun a live pid, repeat a remediation push, or duplicate the final review.

- Run `recover` for finished-but-unrecorded artifacts. Handle invalid verdict exit 2 with the
  same persisted single-retry allowance from §8, never a new allowance after a crash.
- If remediation may have committed/pushed before its status or row was written,
  reconcile local and remote HEAD with the recorded starting/final SHAs and verification
  evidence. Recover `PUSHED|NOOP` and record it without invoking remediation again. If evidence
  cannot prove completion, block that repository for reconciliation instead of repeating a push.
- Query all PR reviews for the protocol's exact phase-scoped final marker. If found,
  recover its actual event/URL into the final run status and record idempotently, then promote
  the repository result. Do this even when the process crashed before writing status; never
  repost. The actual GitHub event determines outcome, not a stale process exit or stage flag.
- Only then call `next` with persisted tier/rounds/inner/ledger. Keep the current round's
  artifact: keep the persisted `headSha` within the inner loop, refreshing only for a new
  outer round after any verified fix. Preserve prior reports and retry evidence. The ledger
  derives the next step; do not reconstruct a static run list or trust a stage cursor.

**Milestone resume** *(human only, `LOOP_SUB` unset; the loop is parked — `state.awaitingMerge`
set, or unset after a crash and recovered in step 1)*. Never auto-resume or poll for merges.

1. **Recover the park.** If `awaitingMerge` is unset but the current milestone's gate review is
   `done` with every participant's gate PR recorded (crash between aggregation and step 8.1),
   rebuild `awaitingMerge {milestone, prs{slug:url}}` from `state.repositories[slug].prUrl`,
   persist it with `loop-state.sh set '.awaitingMerge = …'` (`check` reads it from disk), then
   re-run step-8 park items 2-5 (idempotent): plan Status `awaiting merge` push, the
   `milestone.awaiting-merge` event, the ship explainer, the notification. Continue to check. Never re-run the gate
   review. **Crash after apply:** `state.resumePending` equals `state.milestone` → the patch is
   already applied but the restart never finished: run resume step 5 only. (Recorded milestone
   history or a present `sub/REVIEW_READY` alone prove nothing — both persist through every later
   milestone's own review.)
2. **Check.** `loop-milestone.sh check --state .loop/state.json`: exit 3 (open) → report each open
   PR, stop, change nothing; exit 4 (closed unmerged) → open a HIL for the human (reopen or
   re-plan), stop; exit 1 (gh/state error) → report it, stop; exit 0 (all merged) → archive, then
   continue.

   **Archive.** Archive the shipped milestones before the next milestone starts. P is the parked
   milestone. For each M from 1 to P, in order, run
   `loop-plan.sh archive --milestone <M> --dir .loop`.
   - The command checks that every phase of M is `done` and that every gate PR of M is merged.
   - It moves the phase blocks and the progress-log entries of M to `.loop/milestones/m<M>.md`.
   - It writes the `[archived]` stub into `.loop/plan.md` and pushes the plan.
   - Exit 0 means archived or already archived. Go to the next M.
   - Any other exit: report the command output and stop. Do not edit the plan to force the archive.
     Do not advance. The human fixes the cause and runs `/loop-execute resume` again.
   - Do not change `.loop/state.json`. It keeps the entries of the archived phases.
3. **Advance.** Resolve the next milestone's branch per participant (precedence in preflight) and
   run `loop-milestone.sh advance --state .loop/state.json --next <N> --branches '<json slug→branch>'`.
   It prepares every next participant before retiring old non-adopted worktrees, so a failed
   advance leaves the runtime intact; re-run it as-is after an interruption (idempotent). A
   non-zero exit is reported and stops; nothing is applied.
4. **Apply.** Apply the printed patch with exactly one `loop-state.sh set` as a top-level merge:
   `PATCH="$(loop-milestone.sh advance ...)"; loop-state.sh set ". + $PATCH"` (the patch JSON is a
   valid jq literal). `. + ` replaces each top-level key wholesale — `repositories` becomes exactly
   the next participants and `prUrl`/`review`/`reviewPipeline`/`awaitingMerge` reset to `null`, with
   new `milestone`, `integrationWorktree`, `integrationBranch`, `runtimeRepo`, `runtimeRoot`,
   `runtimeAdopted`, `resumePending` (= the new milestone), `milestones[N].prs` history. Never deep-merge with `*` (keeps stale repositories) or `set` the bare patch (drops the
   rest of state). When the effort changes the Loop
   runtime itself (`runtimeRepo` set), `runtimeRoot` is the host repository's new integration
   worktree (or its retained old one when the host sits the milestone out):
   switch `RUNTIME_ROOT` to it before composing any later prompt; patch `runtimeRoot` null (no host) → keep the installed runtime.
5. **Restart.** Set plan Status back to `in progress` + `loop-plan.sh push`; remove
   `sub/REVIEW_READY`; `loop-state.sh log milestone.resumed` with empty phase and `detail` `M<n>` (the new milestone); seed the lessons (below); respawn the
   orchestrator for the new milestone (the SUB then schedules only its phases); last,
   `loop-state.sh set '.resumePending = null'` (every restart item above is idempotent).
   **Lessons in the seed.** Before the respawn, add the `### Lessons` of the most recent
   archive file `.loop/milestones/m<M>.md` to `.loop/sub/prompt-seed.md`, at most 15 lines.
   M is the highest archived milestone.
   - Put the lessons between the lines `<!-- loop-lessons -->` and `<!-- /loop-lessons -->`.
     Replace that block when it exists. Otherwise, add it at the end of the seed.
   - Start the block with `Lessons from milestone <M>. Apply each lesson that fits a phase.`
   - When the archive file has no `### Lessons` section, remove the block.
6. **Plan explainer (best-effort).** Explain the new milestone in `visual-explainer` `plan`
   mode (protocol § Milestone explainers). The output directory is
   `.loop/explainers/m<N>-plan/`, where N is the new milestone.
   - Skip this step when `link.txt` already exists there.
   - Spawn one detached explainer runner after the respawn. Do not wait for it here.
   - At each later poll or tick, look for its `meta.json`. Then render, publish and write
     `link.txt`, as the skill tells you.
   - Notify once (`loop-notify.sh --level info --title "M<N> plan explainer"`) with the link
     (`--url`) or the failure.
   - A failed explainer never stops the resume or the orchestrator.

### Sub recycle *(mode: `sub` only)*

You are a disposable instance — recycle before you fill up so a fresh successor continues.
Each tick, after handling any runner exits / merges, sweep notes for phases already
`done|merged`, then self-measure:

```sh
loop-state.sh occupancy --dir .loop \
  --transcript .loop/sub/transcript-<k>.jsonl --window <LOOP_ORCH_CTX_WINDOW>
```

(the `<k>` and the exact command are in your spawn prompt). When it reports **≥
`LOOP_ORCH_RECYCLE_TOKENS`** (exit 10) **AND you are at a safe phase boundary** — no runner
mid-attempt without a written `status.json`, no merge in progress — then: flush durable state
(state.json via `loop-state.sh set`, plan/daemon, git are already externalized), write
`sub/handoff.md` (open exit-10 rounds + their `sessionId`s, pending merge-runner promotions =
runDir + lane-branch awaiting a `phase.merged`, per-lane notes, why-stopped), then write
`sub/status.json {outcome:"recycle", summary, tokens:<n>, recycleIndex:<k>}` and **exit**.
`loop-orchestrator.sh` respawns your successor, which consumes the handoff once and picks up
via `resume`. Gated on quiescence: if a phase op is mid-flight, keep going (even past the
ceiling) and recycle asap once every lane is at a checkpoint — **never** hard-kill a runner to
recycle. When the next ready phase is a review, write `{outcome:"review-ready"}`. Use
`{outcome:"complete"}` only for a legacy plan without explicit review phases; on an unrecoverable
error `{outcome:"fatal"}` (bad config / lost lock) or `{outcome:"blocked"}` (HIL is the only
thing left and no lane can progress).
- **status** — print the lane table from state + live pids + last events; read-only.
- **abort** — kill live runners, `--abort` any merge, flip in-progress phases back to
  todo, repush plan, notify, unlock. Leave worktrees for autopsy; tell the user the
  cleanup commands — never for an adopted integration worktree (it is the user's).

## Steering notes

Users may steer work without pausing the loop by writing `.loop/notes/<key>.md`.
A running attempt picks up edits live: the runner resumes the same session with the updated
note (same attempt, no retry consumed). Pause and model overrides live in `.loop/control/`
(protocol § Pause & live control).
All explicit phase keys, including review phases, are plan numbers (`notes/2.md`). Legacy
review keys `notes/pr-review.<owner--repo>.md` and `notes/pr-review.md` remain accepted.
A note persists until that phase or review completes. The `sub` reads phase notes for runner
prompts and again before lane merges; the thin supervise main reads the review note before
spawning pr-review. Any note consumed by a runner appears verbatim in that attempt's
`prompt.md`, which is the durable proof of consumption.

Use `loop-state.sh note <key> "text"` (or omit text and pipe multiline stdin),
`loop-state.sh note --clear <key>`, and `loop-state.sh notes`. Numeric notes targeting a
`done|merged` phase are refused unless `--force`. Cleanup is best-effort because the Observatory
derives NOTE visibility from both file presence and unfinished lifecycle.

## Hard rules

- Never force-push anything; integration pushes are fast-forward via loop-merge, except step
  3's initial branch push and verified remediation commits pushed by the Opus coordinator.
- Never steal a foreign lock — it means a colleague exists.
- Single plan-writer: only the orchestrator pushes the plan — to the daemon via
  `loop-plan.sh push`; runners emit events/notes only (the phase-scoped pickup/handoff auto
  ops) and never push git or the plan. The Opus remediation coordinator is an orchestrator-delegate for verified
  integration-branch commits/pushes only; it never writes the plan.
- Every prompt, transcript, and decision lands under `.loop/` — if it isn't in
  state.json or events.jsonl, it didn't happen (crash-resume depends on this).
- Budget: `--max-budget-usd` per attempt is the ceiling; on repeated 40s across lanes,
  pause scheduling and notify rather than burning the chain repeatedly.
- Two-tier: the `sub` NEVER runs pr-review and NEVER `AskUserQuestion` (HIL → files only);
  those belong to the supervise main. Recycle is a **sequential respawn** owned by
  `loop-orchestrator.sh` — a SUB never spawns its own successor (that would overlap two
  orchestrators on one run id, which the reentrant lock does not prevent). Recycle only at a
  persisted checkpoint; never hard-kill a live phase runner to hit the token ceiling.
- Supervise stays THIN: spawn only the orchestrator + staged review/remediation runners +
  milestone explainer runners, and read only compact local state. A runner writes explainer
  props. Supervise only renders, publishes and links the page. Skimming diffs / resolving conflicts / pulling the daemon snapshot in
  the main chat defeats the two-tier design — that work is the `sub`'s.
