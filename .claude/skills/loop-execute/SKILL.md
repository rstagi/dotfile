---
name: loop-execute
description: >-
  Loop engineering: autonomously execute a PUBLISHED multi-phase plan (registered on the Loop
  daemon; Kestral link optional) end-to-end — spawn a fresh headless runner per phase (Codex
  or Claude, parallel lanes in separate worktrees, cap 3), verify and merge each lane into ONE
  integration branch, escalate stuck work (retry → stronger model → human-in-the-loop pause),
  open the single effort PR, run pr-review on it, then stop for the human to review and merge.
  By default the heavy orchestration runs in a detached, self-recycling sub-orchestrator so
  the interactive chat stays thin. Use when asked to
  "run the loop", "execute the multi-phase plan", "run the published plan autonomously",
  "loop-engineer this", or after multiphase-plan when the user wants the phases executed
  hands-off. NOT for implementing an in-chat plan yourself — it needs a published plan with a
  Loop config.
argument-hint: "<project / plan doc> | resume | status | abort"
---

# Loop Execute

The execution engine for `multiphase-plan`. The plan backend is the **Loop daemon** (a
Kestral link is optional; default local-only). The human plans (and answers questions) once;
this skill runs every phase through pickup → implement → handoff automatically and comes
back with one reviewed PR. Each headless runner runs the pickup/handoff auto modes itself
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
  answers HIL from on-disk briefs, and finalizes on `sub/PHASES_DONE`. It spawns **only** the
  orchestrator and the final review-runner — the heavy scheduling/merge/escalation runs in
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

`gh auth status` OK; `jq`; the loop scripts present; a plan (registered on the Loop daemon or
in `.loop/plan.md`) published by `multiphase-plan` **with a Loop config section** (integration
branch + Verify command). Missing Loop config → offer to add it (ask the user for the
verify command — never invent one). **Kestral only when the plan is LINKED:** if the plan
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
(argument → `.loop/plan.md` header → ask, like `loop-pickup` step 1). Parse phases, lanes,
`Depends on` edges, statuses, per-phase `Verify`; when linked, re-fetch from Kestral and
reconcile against live task statuses. Validate: DAG acyclic; every phase has *Done when*;
Loop config present; working tree clean. Ensure `.loop/` is gitignored. **Adopt the plan's
`planId` as the run id** — the planId IS the runId, so loop-execute drives the same daemon
record through planned → active → finished (one selector entry); if that record is already
`finished` (a re-run), mint `<planId>-r<K>` and re-register. Take the lock now —
`loop-state.sh lock --owner <run-id>`: a foreign owner means a live orchestrator already runs
this effort; surface it and stop (never `--force` silently). This happens before the confirm
gate so the gate's no-further-contact promise holds.

### 2. One confirm gate, then autonomy

Show the user: the backend (local-only vs Kestral-linked), integration branch,
phase/lane table, verify command, chains +
budget/timeouts from `loop-models.conf`, concurrency (the plan's **Concurrency** line
overrides `LOOP_MAX_PARALLEL`; default 3). After their go, do not contact them again
except through the HIL path or completion. `AskUserQuestion` is reserved for those two
moments.

### 3. Init

`loop-state.sh init` with the state.json schema from the protocol. Create the integration
branch **named in the plan's Loop config** from the repo's default branch tip, plus its
worktree (`~/.loop/worktrees/<repo>-integration`), and push it (the one direct
push — everything after goes through loop-merge). Write the branch back into the Loop
config only if it was missing, set plan **Status: in progress**, repush the doc once.

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
   SUB's, and reading it here defeats the purpose. If `sub/orch.pid` is dead and there's no
   `sub/PHASES_DONE`, re-spawn the orchestrator (it `resume`s cleanly — the dead-orchestrator
   backstop).
4. **HIL asker.** Each poll, scan `hil/*.md` lacking a sibling `.answer.md`. For each: read the
   brief, `AskUserQuestion` (the ONLY user contact besides completion), write the reply to
   `hil/<slug>.answer.md`. The `sub` picks it up and requeues that lane — the human-facing HIL
   lives HERE in supervise, never in the SUB.
5. On `sub/PHASES_DONE` → go to **step 8** (finalize). On an orchestrator `fatal`/`blocked`
   notification with no path forward, surface it and stop.

### 4. Schedule *(run by the `sub` instance — or inline in single-tier)*

A phase is READY when `[status: todo]`, all its `Depends on` phases are done, its lane has
no phase running, and fewer runners are live than the concurrency cap (plan's
**Concurrency**, else `LOOP_MAX_PARALLEL`). For each READY phase:

1. Lane worktree on the Suggested branch, cut from the **integration tip**: new lane →
   `git worktree add <path> -b <branch> <integration>`; lane continuing after a merged
   phase → reuse its worktree via `git -C <wt> switch -c <branch> <integration>`.
2. Generate `runs/<phase-slug>-a<K>/prompt.md` from the protocol's skeleton — it opens
   with `loop-pickup --auto` (the runner claims its own task; both engines have the
   Kestral MCP) and closes with `loop-handoff --auto ... status:in-progress` +
   status.json. Include the verbatim phase block + plan Goal/decisions +
   `notes/<phaseNumber>.md` verbatim on **every** attempt, including the first. Retry-only
   context (`answers/`, verify.log tails, HIL answers) remains separate.
3. Spawn detached per the protocol's Runner-spawn section (nohup + pid file):
   `~/dotfile/loop-runner.sh --worktree <wt> --run-dir <abs> --prompt-file <p> --run-id
   <runId> --phase <N> --chain task --verify-cmd '<phase-or-effort verify>'` (it inherits
   `LOOP_DAEMON_URL` via env, so its EXIT-trap `phase.attempt.*` events reach the daemon).
   Record pid + run dir in state; journal the event. A pickup `REFUSED` surfaces as the
   runner's `blocked` status → ladder, keep scheduling other lanes.

There are no completion notifications from detached runners — monitor by polling per the
protocol: `meta.json` appearing means the attempt ended; a `transcript.jsonl` staler than
the protocol's threshold means a hung runner (kill the pid tree, treat as 124).

### 5. Handle a runner exit

Switch on the exit code (protocol table). The extra checks only you can do:

- **exit 0** — before merging, skim `git diff <base>...HEAD` in the lane worktree against
  the phase's *Done when* (the verify command proves it runs; you prove it's the right
  work). `headBefore == headAfter` in meta.json → stall: escalate, never accept. Also
  check the runner didn't push or leave junk (`git -C <wt> status`).
- **exit 10** — read `status.json`'s question. Answer it yourself from the plan, Project
  Brain, and the code (this is why the orchestrator is the big model). Resume the session
  **in a fresh attempt dir** (protocol Q&A-resume): `loop-runner.sh --resume <sessionId>
  --engine <meta.engine> --run-dir <new a<K+1>> ...` with your answer as the prompt. Cap 3
  rounds per phase, then treat as blocked.
- **exit 12** — relaunch once with the verify.log tail in the prompt; second failure →
  L3.
- **exit 20 / 50×2 / 124×2** — escalate per the ladder. **exit 40** — follow the
  protocol's exit-40 rule (bounded backoff, then L3; multi-lane 40s pause scheduling).

### 6. Merge and sync

Immediately before merging phase `<N>`, re-read `notes/<N>.md` and honor it (for example,
rebase the lane onto the current integration tip first). This catches notes dropped after the
runner started. After the merge is complete and the phase is promoted `done|merged`, run
`loop-state.sh note --dir .loop --clear <N>` as best-effort housekeeping.

Serialize merges (one at a time). `loop-merge.sh --worktree <int-wt> --lane-branch <b>
--run-id <runId> --phase <N> --verify-cmd '<effort verify>'` (inherits `LOOP_DAEMON_URL`;
its EXIT trap emits `phase.merged` on rc 0, `merge.conflict` on a conflict):

- **0** → run `loop-handoff --auto phase:<N> status:done lane:<X> engine:<engine>`
  inline — the orchestrator context of that skill (you are the plan's sole writer; runners
  already posted their task-scoped sync). **Linked:** it flips markers, repushes the plan doc
  (`update_document`), updates the task (`update_task_status`), and posts a progress comment
  noting lane + engine. **Local-only:** it instead flips the plan's `[status: done]` marker +
  `loop-plan.sh push`, appends `.loop/progress.md`, and `loop-plan.sh note` (no Kestral).
  Cleanup is unchanged in both modes, in protocol order: switch the worktree to the lane's
  next branch (or remove it if the lane is finished), **then** `git branch -d` the merged
  branch. First merge → `gh pr create --draft` from the integration branch, PR URL into state
  + Loop config. Schedule the lane's next phase.
- **2** → conflict. **`sub` mode: spawn a detached merge-runner** (protocol's
  Merge-runner skeleton — `loop-runner.sh --chain escalate --worktree <int-wt>` with NO
  `--verify-cmd`), so the diff never enters your context; promote on the next tick when its
  `phase.merged` fires. **single-tier:** resolve inline (`resolving-merge-conflicts`, honoring
  both phases' *Done when*), then `loop-merge.sh --worktree <int-wt> --finish --verify-cmd
  '<effort verify>'`. Either way, not confident it's semantically right → `--abort` + HIL.
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

### 8. Complete *(supervise / single-tier — never the `sub`)*

In supervise mode the `sub` does NOT finalize: when every phase is merged it writes
`status.json{outcome:"complete"}`, `loop-orchestrator.sh` touches `sub/PHASES_DONE`, and the
supervise main (step 3b.5) runs this step. The `sub` never runs pr-review.

All phases done → `gh pr ready`; PR body from the plan (Goal, phase list with task links,
Progress log digest — never a raw log dump). Then record completion on the backend:

- **Linked:** `link_pr_to_task` for every phase task (dedup via state), statuses →
  awaiting-review, plan **Status: integrating** via `update_document` (repush),
  `trigger_brain_build`.
- **Local-only:** flip plan **Status: integrating** + `loop-plan.sh push`, append
  `.loop/progress.md` (no Kestral) — the daemon learns completion from `loop-state.sh finish`'s
  `loop.finish` below.

Sweep any remaining lane worktrees (review needs the branches free). Re-read
`notes/pr-review.md` now: honor any pre-review action against the integration worktree, then
fold the note verbatim into the reviewer prompt. Then spawn the reviewer with a FULL runner
prompt (the protocol skeleton, not a
one-liner — it must end with the RUN_DIR/status.json instructions or the run is
misclassified as a crash): task = "run the pr-review skill on <PR URL> with --headless
(includes the stacked-PR-split lens); write the full report to `RUN_DIR/report.md` and post
it as a PR comment; then write status.json with outcome done and the verdict as summary".
Spawn: `loop-runner.sh --chain review --worktree <int-wt> --run-dir <runs/review-a1> ...`.
When it returns, promote the review onto state — `loop-state.sh set '.review =
{outcome, summary, reportPath: "runs/review-a<K>/report.md", commentUrl}'` — log
`review.finish`, clear `notes/pr-review.md` with `loop-state.sh note --clear pr-review`, then
`loop-state.sh finish --json '{...prUrl, review}'` (emits `loop.finish`
to the daemon). Finally `loop-notify.sh --level info` with PR + review verdict + split
recommendation, journal, unlock, and stop — the human merges.

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
recycle. When every phase is merged instead, write `{outcome:"complete"}`; on an unrecoverable
error `{outcome:"fatal"}` (bad config / lost lock) or `{outcome:"blocked"}` (HIL is the only
thing left and no lane can progress).
- **status** — print the lane table from state + live pids + last events; read-only.
- **abort** — kill live runners, `--abort` any merge, flip in-progress phases back to
  todo, repush plan, notify, unlock. Leave worktrees for autopsy; tell the user the
  cleanup commands.

## Steering notes

Users may steer work without pausing the loop by writing `.loop/notes/<key>.md`.
Phase keys are plan numbers (`notes/2.md`); `notes/pr-review.md` is the only reserved step key.
A note persists until that phase or review completes. The `sub` reads phase notes for runner
prompts and again before lane merges; the thin supervise main reads the review note before
spawning pr-review. Any note consumed by a runner appears verbatim in that attempt's
`prompt.md`, which is the durable proof of consumption.

Use `loop-state.sh note <key> "text"` (or omit text and pipe multiline stdin),
`loop-state.sh note --clear <key>`, and `loop-state.sh notes`. Numeric notes targeting a
`done|merged` phase are refused unless `--force`. Cleanup is best-effort because the Observatory
derives NOTE visibility from both file presence and unfinished lifecycle.

## Hard rules

- Never force-push anything; integration pushes are fast-forward via loop-merge only
  (sole exception: step 3's initial branch push).
- Never steal a Kestral claim (409) or a foreign lock — both mean a colleague exists.
- Single plan-writer: only the orchestrator pushes the plan — to the daemon via
  `loop-plan.sh push` and/or Kestral via `update_document` — and links PRs; runners emit
  events/notes only (the task-scoped pickup/handoff auto ops) and never push git or call
  `update_document`.
- Every prompt, transcript, and decision lands under `.loop/` — if it isn't in
  state.json or events.jsonl, it didn't happen (crash-resume depends on this).
- Budget: `--max-budget-usd` per attempt is the ceiling; on repeated 40s across lanes,
  pause scheduling and notify rather than burning the chain repeatedly.
- Two-tier: the `sub` NEVER runs pr-review and NEVER `AskUserQuestion` (HIL → files only);
  those belong to the supervise main. Recycle is a **sequential respawn** owned by
  `loop-orchestrator.sh` — a SUB never spawns its own successor (that would overlap two
  orchestrators on one run id, which the reentrant lock does not prevent). Recycle only at a
  persisted checkpoint; never hard-kill a live phase runner to hit the token ceiling.
- Supervise stays THIN: spawn only the orchestrator + the review-runner, and read only
  compact local state. Skimming diffs / resolving conflicts / pulling the daemon snapshot in
  the main chat defeats the two-tier design — that work is the `sub`'s.
