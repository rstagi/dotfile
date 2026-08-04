---
name: kestral-loop
description: >-
  Loop engineering: autonomously execute a published Kestral multi-phase plan end-to-end —
  spawn a fresh headless runner per phase (Codex or Claude, parallel lanes in separate
  worktrees, cap 3), verify and merge each lane into ONE integration branch, escalate
  stuck work (retry → stronger model → human-in-the-loop pause), open the single effort
  PR, run pr-review on it, then stop for the human to review and merge. Use when asked to
  "run the loop", "execute the Kestral plan", "run the published plan autonomously",
  "loop-engineer this", or after multiphase-plan when the user wants the phases executed
  hands-off. NOT for implementing an in-chat plan yourself — it needs a published Kestral
  plan with a Loop config.
argument-hint: "<project / plan doc> | resume | status | abort"
---

# Kestral Loop

The execution engine for `multiphase-plan`. The human plans (and answers questions) once;
this skill runs every phase through pickup → implement → handoff automatically and comes
back with one reviewed PR. Each headless runner runs the pickup/handoff auto modes itself
(both engines carry the Kestral MCP); you — the orchestrator session — are the sole
writer of the plan document and the integration branch, and the policy brain. Scripts do
the mechanics. The full contract (dir layout,
status.json, exit codes, chains, prompts) is `references/loop-protocol.md` — read it
before starting and follow it exactly.

**Division of labor:** scripts (`~/dotfile/loop-runner.sh`, `loop-merge.sh`,
`loop-notify.sh`, `loop-state.sh`) are mechanism; never shell raw `claude`/`codex`/`git
merge` yourself. You are policy: what to schedule, whether a result is acceptable, how to
answer a runner's question, when to escalate, when to wake the human.

## Prerequisites

Kestral MCP in-session (`whoami` works); `gh auth status` OK; `jq`; the four loop scripts
present; a plan published by `multiphase-plan` **with a Loop config section** (integration
branch + Verify command). Missing Loop config → offer to add it (ask the user for the
verify command — never invent one). Probe headless Kestral once per engine before
launching lanes (runners claim their own tasks): a one-shot `codex exec` / `claude -p`
asking for Kestral `whoami` — an auth failure there means every lane dies silently, fix it
first. Long runs: remind the user once to `caffeinate` the Mac; do not manage power
yourself.

## Workflow

### 1. Preflight

Resolve the plan like `kestral-pickup` step 1 (argument → `.kestral/plan.md` header →
ask). Re-fetch from Kestral; parse phases, lanes, `Depends on` edges, statuses, per-phase
`Verify`; reconcile against live task statuses. Validate: DAG acyclic; every phase has
*Done when*; Loop config present; working tree clean. Ensure `.kestral/` is gitignored.
Generate the run id (`loop-<effort-slug>-<date>-<HHMMSS|rand>` — opaque; the trailing
segment prevents same-effort-same-day store-key collisions across parallel checkouts) and
take the lock now — `loop-state.sh lock --owner <run-id>`: a foreign owner means a live
orchestrator already runs this effort; surface it and stop (never `--force` silently). This
happens before the confirm gate so the gate's no-further-contact promise holds.

### 2. One confirm gate, then autonomy

Show the user: integration branch, phase/lane table, verify command, chains +
budget/timeouts from `loop-models.conf`, concurrency (the plan's **Concurrency** line
overrides `LOOP_MAX_PARALLEL`; default 3). After their go, do not contact them again
except through the HIL path or completion. `AskUserQuestion` is reserved for those two
moments.

### 3. Init

`loop-state.sh init` with the state.json schema from the protocol. Create the integration
branch **named in the plan's Loop config** from the repo's default branch tip, plus its
worktree (`~/.kestral-loop-worktrees/<repo>-integration`), and push it (the one direct
push — everything after goes through loop-merge). Write the branch back into the Loop
config only if it was missing, set plan **Status: in progress**, repush the doc once.

**The observer runs itself (on by default).** `loop-state.sh init` sources `loop-emit.sh`,
which runs `loop_ensure_daemon` (starts the central Loop daemon if it isn't already up) then
POSTs `/api/loops/<runId>/register` — so this loop appears in the daemon's authoritative,
never-stale status the moment `.kestral/loop/` exists. Print `http://localhost:7717` once so
the human can open the **Loop Observatory** and pick this loop from the selector. It's an
optional read-only dashboard that never blocks the run; printing a URL is not a "contact"
that breaks the confirm gate's promise.

### 4. Schedule

A phase is READY when `[status: todo]`, all its `Depends on` phases are done, its lane has
no phase running, and fewer runners are live than the concurrency cap (plan's
**Concurrency**, else `LOOP_MAX_PARALLEL`). For each READY phase:

1. Lane worktree on the Suggested branch, cut from the **integration tip**: new lane →
   `git worktree add <path> -b <branch> <integration>`; lane continuing after a merged
   phase → reuse its worktree via `git -C <wt> switch -c <branch> <integration>`.
2. Generate `runs/<phase-slug>-a<K>/prompt.md` from the protocol's skeleton — it opens
   with `kestral-pickup --auto` (the runner claims its own task; both engines have the
   Kestral MCP) and closes with `kestral-handoff --auto ... status:in-progress` +
   status.json. Include the verbatim phase block + plan Goal/decisions + prior-attempt
   context (`answers/`, verify.log tails, HIL answers).
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

Serialize merges (one at a time). `loop-merge.sh --worktree <int-wt> --lane-branch <b>
--run-id <runId> --phase <N> --verify-cmd '<effort verify>'` (inherits `LOOP_DAEMON_URL`;
its EXIT trap emits `phase.merged` on rc 0, `merge.conflict` on a conflict):

- **0** → run `kestral-handoff --auto phase:<N> status:done lane:<X> engine:<engine>`
  inline — the orchestrator context of that skill: flips markers, repushes the plan doc
  (you are its sole writer; runners already posted their task-scoped sync), updates the
  task, progress comment noting lane + engine. Cleanup in protocol order: switch the worktree to the lane's next branch (or
  remove it if the lane is finished), **then** `git branch -d` the merged branch. First
  merge → `gh pr create --draft` from the integration branch, PR URL into state + Loop
  config. Schedule the lane's next phase.
- **2** → resolve the conflict yourself in the integration worktree
  (`resolving-merge-conflicts`, honoring both phases' *Done when*), then
  `loop-merge.sh --worktree <int-wt> --finish --verify-cmd '<effort verify>'`. Not
  confident it's semantically right → `--abort` + HIL.
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
hil.raise`, then ask the user in-session. **Only that lane pauses.** On answer, requeue the
phase with the answer in context.

### 8. Complete

All phases done → `gh pr ready`; PR body from the plan (Goal, phase list with task links,
Progress log digest — never a raw log dump). `link_pr_to_task` for every phase task
(dedup via state), statuses → awaiting-review, plan **Status: integrating**, repush,
`trigger_brain_build`. Sweep any remaining lane worktrees (review needs the branches
free). Then spawn the reviewer with a FULL runner prompt (the protocol skeleton, not a
one-liner — it must end with the RUN_DIR/status.json instructions or the run is
misclassified as a crash): task = "run the pr-review skill on <PR URL> with --headless
(includes the stacked-PR-split lens); write the full report to `RUN_DIR/report.md` and post
it as a PR comment; then write status.json with outcome done and the verdict as summary".
Spawn: `loop-runner.sh --chain review --worktree <int-wt> --run-dir <runs/review-a1> ...`.
When it returns, promote the review onto state — `loop-state.sh set '.review =
{outcome, summary, reportPath: "runs/review-a<K>/report.md", commentUrl}'` — log
`review.finish`, then `loop-state.sh finish --json '{...prUrl, review}'` (emits `loop.finish`
to the daemon). Finally `loop-notify.sh --level info` with PR + review verdict + split
recommendation, journal, unlock, and stop — the human merges.

### 9. `resume` / `status` / `abort`

- **resume** — re-lock with the state's run id, then reconcile per protocol (git > Kestral
  > state): finish/abort any in-progress merge first; re-attach or fail dead attempts;
  repair status drift. Never double-spawn a phase with a live pid. Call `loop-state.sh
  register` (idempotent ensure-daemon + re-register) so the loop reappears in the observer —
  no per-loop observer to relaunch.
- **status** — print the lane table from state + live pids + last events; read-only.
- **abort** — kill live runners, `--abort` any merge, flip in-progress phases back to
  todo, repush plan, notify, unlock. Leave worktrees for autopsy; tell the user the
  cleanup commands.

## Hard rules

- Never force-push anything; integration pushes are fast-forward via loop-merge only
  (sole exception: step 3's initial branch push).
- Never steal a Kestral claim (409) or a foreign lock — both mean a colleague exists.
- Single doc-writer: runners do task-scoped Kestral ops only (pickup/handoff auto); they
  never push git or call `update_document` — only you repush the plan doc and link PRs.
- Every prompt, transcript, and decision lands under `.kestral/loop/` — if it isn't in
  state.json or events.jsonl, it didn't happen (crash-resume depends on this).
- Budget: `--max-budget-usd` per attempt is the ceiling; on repeated 40s across lanes,
  pause scheduling and notify rather than burning the chain repeatedly.
