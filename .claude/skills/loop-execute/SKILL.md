---
name: loop-execute
description: >-
  Loop engineering: autonomously execute an editable multi-phase plan registered on the Loop
  daemon; Kestral link optional) end-to-end — spawn a fresh headless runner per phase (Codex
  or Claude, parallel lanes in separate worktrees, cap 3), verify and merge each lane into its
  repository integration branch, escalate stuck work (retry → stronger model → HIL pause),
  open one effort PR per repository, execute each explicit review phase as three Astra/Fable
  adversarial passes with Opus remediation between them, post one final Opus verdict, then stop
  for humans to merge.
  By default the heavy orchestration runs in a detached, self-recycling sub-orchestrator so
  the interactive chat stays thin. Use when asked to
  "run the loop", "execute the multi-phase plan", "run the published plan autonomously",
  "loop-engineer this", or after multiphase-plan when the user wants the phases executed
  hands-off. NOT for implementing an in-chat plan yourself — it needs a registered plan with a
  Loop config.
argument-hint: "<project / plan doc> | resume | status | abort"
---

# Loop Execute

The execution engine for `multiphase-plan`. The plan backend is the **Loop daemon** (a
Kestral link is optional; default local-only). The human plans once and may keep editing;
this skill runs work phases through pickup → implement → handoff and treats explicit review
phases as orchestrator-owned barriers. Each headless runner runs the pickup/handoff auto modes itself
(both engines carry the Kestral MCP when the plan is linked); you — the orchestrator session
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
  orchestrator and review/remediation runners — the heavy scheduling/merge/escalation runs in
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
plans normalize to the synthetic `primary` repository. **Kestral only when the plan is LINKED:** if the plan
carries a Kestral link, the Kestral MCP must work in-session (`whoami`) AND you must probe
headless Kestral once per engine before launching lanes (runners claim their own Kestral
tasks only in linked mode) — a one-shot `codex exec` / `claude -p` asking for Kestral
`whoami`; an auth failure there means every lane dies silently, fix it first. For a
local-only plan, skip the probe entirely. Long runs: remind the user once to `caffeinate`
the Mac; do not manage power yourself.

## Workflow

### 1. Preflight

Resolve the plan LOCAL → daemon → Kestral-if-linked: `.loop/plan.md` header → the daemon
(`loop-plan.sh get --plan-id <id>`, else `loop-plan.sh list`) → Kestral **only when linked**
(argument → `.loop/plan.md` header → ask, like `loop-pickup` step 1). Parse phases, kinds,
lanes, repositories, `Depends on` edges, statuses, and per-phase `Verify`; when linked,
re-fetch from Kestral and reconcile against live task statuses. Validate: DAG acyclic;
every phase has *Done when*; every work phase has exactly one known repository; the final
document phase is `[kind: pr-review]`. Legacy plans without explicit kinds retain their
synthetic terminal review. Resolve each `owner/repo` through
`loop-repo.sh get`; before autonomy ask once for missing checkout paths and persist them
with `loop-repo.sh map`. Run `loop-repo.sh check` for origin/default-branch/collision
validation. Ensure coordinator `.loop/` is gitignored and repository worktrees are clean.
**Adopt the plan's
`planId` as the run id** — the planId IS the runId, so loop-execute drives the same daemon
record through planned → active → finished (one selector entry). If it is already `finished`,
re-read the plan: newly appended unfinished phases reopen that same record; only an unchanged
intentional re-run mints `<planId>-r<K>`. Take the lock now —
`loop-state.sh lock --owner <run-id>`: a foreign owner means a live orchestrator already runs
this effort; surface it and stop (never `--force` silently). This happens before the confirm
gate so the gate's no-further-contact promise holds.

### 2. One confirm gate, then autonomy

Show the user: the backend (local-only vs Kestral-linked), repository → checkout/default
branch/integration branch/verify table, phase/kind/lane/repository table, chains +
budget/timeouts from `loop-models.conf`, concurrency (the plan's **Concurrency** line
overrides `LOOP_MAX_PARALLEL`; default 3). After their go, do not contact them again
except through the HIL path or completion. `AskUserQuestion` is reserved for those two
moments.

### 3. Init

`loop-state.sh init` with runtime state v2 from the protocol. For every repository run
`loop-repo.sh prepare --repo <owner/repo> --run-id <runId> --integration-branch <branch>`.
It fetches GitHub's default ref and creates the integration branch/worktree from that exact
fetched SHA under `~/.loop/worktrees/<runId>/<owner--repo>/integration`; never use stale
local `main`. Push each initial integration branch and persist the returned repository fields
in state. Later phase merges go through loop-merge; only a review-phase remediation
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
   backstop).
4. **HIL asker.** Each poll, scan `hil/*.md` lacking a sibling `.answer.md`. For each: read the
   brief, `AskUserQuestion` (the ONLY user contact besides completion), write the reply to
   `hil/<slug>.answer.md`. The `sub` picks it up and requeues that lane — the human-facing HIL
   lives HERE in supervise, never in the SUB.
5. On `sub/REVIEW_READY` → reconcile the plan once more, then go to **step 8**. A late edit
   that makes the review no longer ready clears the marker and respawns the orchestrator.
   Legacy `sub/PHASES_DONE` uses the legacy final-review path. On `fatal`/`blocked` with no
   path forward, surface it and stop.

### 4. Schedule *(run by the `sub` instance — or inline in single-tier)*

At every safe scheduling boundary, re-read the newest plan (daemon; Kestral too when linked)
and reconcile it into `state.phases` without replacing existing entries. Phase numbers are
stable identities; document order is the sequence. Add each new phase as `todo` (and create
its linked subtask when applicable), then repush. Preserve completed and running state.

Enforce review topology as one coherent plan edit:

- Last review pending → place new work immediately before it and extend that review's
  dependency closure. Reuse the review.
- Last review done → keep it where it ran, place new work after it, and append a fresh
  terminal `[kind: pr-review]` phase using the next unused phase number.

Never renumber, move, or reopen a completed phase. If an edit races a live runner or changes
its body/repository/branch, preserve the running snapshot and apply it at the next safe
boundary; incompatible edits raise HIL. A review phase follows normal readiness rules, but
the SUB yields it to supervise with `status.json{outcome:"review-ready"}`.

A work phase is READY when `[status: todo]`, all its `Depends on` phases are done, its lane has
no phase running, and fewer runners are live than the concurrency cap (plan's
**Concurrency**, else `LOOP_MAX_PARALLEL`). For each READY phase:

1. Lane worktree on the Suggested branch, cut from the phase repository's **integration
   tip**. Reuse it only when the prior lane phase used the same repository. On a repository
   hop, remove the old worktree and create the next from the target repository tip.
2. Generate `runs/<phase-slug>-a<K>/prompt.md` from the protocol's skeleton — it opens
   with `loop-pickup --auto` (the runner claims its own task; both engines have the
   Kestral MCP) and closes with `loop-handoff --auto ... status:in-progress` +
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
  existing plan/Project Brain/code path. Validate the disposition with
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
  already posted their task-scoped sync). **Linked:** it flips markers, repushes the plan doc
  (`update_document`), updates the task (`update_task_status`), and posts a progress comment
  noting lane + engine. **Local-only:** it instead flips the plan's `[status: done]` marker +
  `loop-plan.sh push`, appends `.loop/progress.md`, and `loop-plan.sh note` (no Kestral).
  Cleanup removes the worktree on a repository hop; otherwise it may switch to the next lane
  branch. First merge in each repository → `gh pr create --draft` from that repository's
  integration branch, PR URL into `state.repositories[slug]` + its plan block. Later merges
  update only that PR. Schedule globally by DAG/lane capacity.
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
  integration worktree → clean it), don't treat as a conflict.

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
that repository's phases/task links plus a progress digest. Then record completion:

- **Linked:** link each work-phase task only to its repository PR (dedup via state); the
  aggregate review-phase task links the plan/effort rather than arbitrarily choosing one PR.
  Move statuses to awaiting-review and plan **Status: integrating** via `update_document` (repush),
  `trigger_brain_build`.
- **Local-only:** flip plan **Status: integrating** + `loop-plan.sh push`, append
  `.loop/progress.md` (no Kestral) — the daemon learns completion from `loop-state.sh finish`'s
  `loop.finish` below.

Sweep remaining lane worktrees. Re-read `notes/<reviewPhase>.md` plus legacy repository
review-note aliases, honor them against each integration worktree throughout this review phase,
and fold them verbatim into every reviewer/remediator prompt. For each repository run this exact
pipeline in `runs/review-p<N>-<owner--repo>-a<1..9>/`, passing `--phase <N>` and
`--repository <owner/repo>`. Different repositories may progress in parallel under Loop
Concurrency; stages within one repository are ordered:

1. Capture the current PR head SHA. Launch Astra 6 (`review-astra`, `a1`) and Fable 5.1
   (`review-fable`, `a2`) adversarially against that same SHA. They independently run the full
   `pr-review` inspection but **never post or mutate GitHub** and never read each other's output.
2. Launch Opus 5 (`review-fix`, `a3`) on the integration worktree. Its prompt invokes
   `/pr-review-fix-all` with both report paths, `RUN_DIR`, branch/remote, and repository verify
   command. That skill reproduces and reconciles every finding, then delegates independent fix
   groups to Opus 5 subagents (max 3), integrates, verifies, commits, and pushes. Rejected or
   duplicate findings require evidence; no accepted finding may be silently skipped.
3. Capture the new PR head. Repeat the independent local-only Astra/Fable reviews as `a4`/`a5`.
4. Repeat `/pr-review-fix-all` through Opus 5 (`review-fix`, `a6`), then verify/commit/push.
5. Capture the new PR head. Run the third independent local-only Astra/Fable reviews as
   `a7`/`a8`.
6. Launch Opus 5 (`review-final`, `a9`). It reads the two third-pass reports, reconciles them
   against the current code, and posts the pipeline's **only** GitHub review:
   `REQUEST_CHANGES` for one or more unresolved `[blocker]` findings; otherwise `COMMENT` for
   one or more unresolved `[major]` findings; otherwise `APPROVE`. Minor/nit/style findings do
   not prevent approval. Save the API response URL in `status.json.commentUrl`.

Persist `repositories[slug].reviewPipeline` with this review's `phase` and the **next** `stage`
(`round1 → fix1 → round2 → fix2 → round3 → final → done`), advancing it only after that stage
finishes. Initialize it to `{phase:<N>,stage:"round1"}` for a new explicit review phase. The
final review body includes the phase-scoped idempotency marker from the protocol; resume searches
GitHub for it before posting. Any reviewer/remediator infrastructure failure records `blocked`
without cancelling sibling repositories; do not substitute models or advance that repository.
For exit 10 with `checkpoint:true`, read the status and checkpoint report, give the
runner one concrete answer, and resume the same stage against the same PR head.
Do not count this as a reviewer failure or advance the stage; sibling repositories continue.
After `a9`, promote its verdict, report path, and comment URL into
`state.repositories[slug].review`, clear the review note, and emit one `review.finish`.

Wait for every repository. Aggregate verdict precedence is `blocked > question > done`. If any
repository is blocked, mark the explicit review phase `blocked` and use the normal HIL/retry path;
otherwise promote it to `done`. Repush after promotion. If later document phases exist, set plan
Status back to `in progress`, clear `REVIEW_READY`, and respawn the orchestrator for the next
segment. If this review is terminal, emit exactly one
`loop-state.sh finish --json '{status:"integrating",repositories:{...}}'`; the daemon rejects
finish unless the terminal explicit review is done. Keep plan Status `integrating` until humans
merge every PR. Notify once with final PRs/verdicts, unlock, stop.

### 9. `resume` / `status` / `abort`

- **resume** — re-lock with the state's run id, then reconcile per protocol (git >
  plan/daemon > state, + Kestral when linked): finish/abort any in-progress merge first;
  re-attach or fail dead attempts;
  repair status drift. Sweep `notes/<N>.md` for every phase already `done|merged`; this is
  idempotent housekeeping, not badge correctness. Never double-spawn a phase with a live pid. Call `loop-state.sh
  register` (idempotent ensure-daemon + re-register) so the loop reappears in the observer —
  no per-loop observer to relaunch. **If `LOOP_SUB=1` (you are a SUB instance):** FIRST, if
  `sub/handoff.md` exists, read it **once** and rename it to `sub/handoff.consumed-<k>.md`
  (single-consumption); then reconcile as above and run steps 4-7 under the recycle loop below.

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
(state.json via `loop-state.sh set`, Kestral, git are already externalized), write
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
  cleanup commands.

## Steering notes

Users may steer work without pausing the loop by writing `.loop/notes/<key>.md`.
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
- Never steal a Kestral claim (409) or a foreign lock — both mean a colleague exists.
- Single plan-writer: only the orchestrator pushes the plan — to the daemon via
  `loop-plan.sh push` and/or Kestral via `update_document` — and links PRs; runners emit
  events/notes only (the task-scoped pickup/handoff auto ops) and never push git or call
  `update_document`. The Opus remediation coordinator is an orchestrator-delegate for verified
  integration-branch commits/pushes only; it never writes the plan or Kestral.
- Every prompt, transcript, and decision lands under `.loop/` — if it isn't in
  state.json or events.jsonl, it didn't happen (crash-resume depends on this).
- Budget: `--max-budget-usd` per attempt is the ceiling; on repeated 40s across lanes,
  pause scheduling and notify rather than burning the chain repeatedly.
- Two-tier: the `sub` NEVER runs pr-review and NEVER `AskUserQuestion` (HIL → files only);
  those belong to the supervise main. Recycle is a **sequential respawn** owned by
  `loop-orchestrator.sh` — a SUB never spawns its own successor (that would overlap two
  orchestrators on one run id, which the reentrant lock does not prevent). Recycle only at a
  persisted checkpoint; never hard-kill a live phase runner to hit the token ceiling.
- Supervise stays THIN: spawn only the orchestrator + staged review/remediation runners, and
  read only compact local state. Skimming diffs / resolving conflicts / pulling the daemon snapshot in
  the main chat defeats the two-tier design — that work is the `sub`'s.
