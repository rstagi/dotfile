# Loop Protocol

The shared contract between the `kestral-loop` orchestrator skill, the `loop-*.sh` scripts
(`~/dotfile/loop-runner.sh`, `loop-merge.sh`, `loop-notify.sh`, `loop-state.sh`,
`loop-models.conf`), and the `--auto` modes of `kestral-pickup` / `kestral-handoff` /
`pr-review`. Everything machine-parsed lives here; change it in lockstep everywhere.

## Roles

- **Orchestrator** — the interactive Claude Code session (Fable 5) running `kestral-loop`.
  Owns the plan **document** (sole `update_document` writer — parallel repushes are
  last-writer-wins), post-merge task status flips, PR creation/linking, all pushes to the
  integration branch, scheduling, escalation, HIL.
- **Runner** — one headless agent process (`codex exec` or `claude -p`) working one phase
  attempt inside a lane worktree. Both hosts have the Kestral MCP (Claude: user plugin;
  Codex: `kestral@kestral-plugins` plugin) and the skills installed. A runner opens with
  `kestral-pickup --auto` (claims its own task, loads context) and closes with
  `kestral-handoff --auto` (task status + progress comment — **task-scoped ops only**).
  Runners NEVER: push, open PRs, call `update_document` or any plan-doc/PR-link operation,
  touch files outside their worktree, or ask the user. A runner's final act is writing
  `status.json`.
- **Scripts** — mechanism only (spawn/merge/notify/state). They never decide; the
  orchestrator switches on their exit codes.

## Directory layout

State lives in the orchestrator's checkout of the target repo (add `.kestral/` to
`.gitignore`):

```
.kestral/loop/
  state.json                 # orchestrator bookkeeping (loop-state.sh, atomic writes)
  events.jsonl               # append-only journal: {ts, event, phase, detail}
  lock/                      # mkdir-lock; owner = run id (sessions have no stable pid);
                             # same owner re-locks freely, another owner needs --force
  runs/<phase-slug>-a<K>/    # one dir per attempt K of a phase
    prompt.md                # exact prompt sent
    transcript.jsonl         # full stream-json/JSONL (kept; mtime doubles as heartbeat)
    last.md                  # final assistant message
    status.json              # runner-written result (schema below)
    stderr.log  verify.log   # wrapper-captured
    meta.json                # wrapper-written: engine, model, sessionId, exit, head shas
  answers/<phase-slug>.md    # orchestrator guidance injected into a retry
  hil/<phase-slug>.md        # HIL request; answer arrives as hil/<phase-slug>.answer.md
```

Worktrees live outside the repo (ralph convention):

- Lane: `~/.kestral-loop-worktrees/<repo>-<lane-branch-slug>` on the phase's Suggested
  branch, cut from the **current integration-branch tip**.
- Integration: `~/.kestral-loop-worktrees/<repo>-integration` on the integration branch.
  All merges and pushes happen only here.

## state.json schema

Crash-resume reads exactly these fields — keep the shape:

```json
{
  "runId": "loop-<effort-slug>-<date>",
  "effort": "...", "projectId": "...", "workContextId": "...",
  "integrationBranch": "feat/<effort-slug>", "integrationWorktree": "<abs path>",
  "prUrl": null,
  "phases": {
    "3": { "slug": "<task-slug>", "taskId": "...", "lane": "A",
           "branch": "feat/api-client-token-refresh", "worktree": "<abs path or null>",
           "status": "todo|claimed|running|merged|blocked|done",
           "attempt": 2, "runDir": ".kestral/loop/runs/<phase-slug>-a2",
           "pid": 4242, "sessionId": "<from meta.json>", "questionRounds": 0 }
  },
  "linkedPrTasks": []
}
```

## Runner spawn (how the orchestrator launches loop-runner.sh)

Detached, so runners survive orchestrator death:

```sh
nohup ~/dotfile/loop-runner.sh <args> > <runDir>/spawn.log 2>&1 &
echo $! > <runDir>/pid; disown
```

There is no completion notification — the orchestrator monitors by polling: `meta.json`
exists → the attempt ended (read `status.json` + runner exit from spawn.log tail);
otherwise `transcript.jsonl` mtime is the heartbeat, stale >25 min → kill the pid tree,
treat as exit 124. The 25-minute threshold lives here only.

## status.json (runner → orchestrator)

The runner's final act is writing `$RUN_DIR/status.json` (the absolute `RUN_DIR` path is
given in its prompt):

```json
{
  "outcome": "done | question | blocked",
  "summary": "1-3 lines: what happened / what was built",
  "question": "only when outcome=question — ONE concrete question, with options if useful",
  "details": "only when outcome=blocked — what was found, what was tried"
}
```

- `done` — the phase's *Done when* holds, work is committed, local verify passed.
- `question` — a decision is needed (ambiguity, unexpected finding with options). Stop
  immediately after writing; the orchestrator answers and resumes the session.
- `blocked` — cannot proceed and no question would unblock (missing dep, broken base,
  contradiction in the plan).

No sentinel tags in prose. A process that exits without a valid `status.json` is a crash.

## loop-runner.sh exit codes (orchestrator switches on these)

| Code | Meaning | Orchestrator reaction |
|------|---------|----------------------|
| 0    | `done`, status.json valid, `--verify-cmd` passed (if given) | merge gate |
| 10   | `question` | escalation L2: answer + resume |
| 12   | claimed `done` but verify failed (verify.log has output) | retry with failure context; 2nd time → L3 |
| 20   | `blocked` | escalation L3 |
| 40   | whole model chain exhausted on API errors | backoff (60s·2ⁿ, cap 15m), retry from top of chain — max 3 per phase, then L3; two or more lanes hitting 40 → pause all scheduling + notify |
| 50   | crash: process exited without valid status.json | count as failed attempt → L3 after 2 |
| 124  | watchdog timeout (TERM then KILL) | one retry, then L3 |
| 1    | usage/infra error (bad args, missing tools) | fix invocation, not the phase |

Every attempt restarts at the **top** of its chain (no sticky fallback). `meta.json` records
`headBefore`/`headAfter`; exit 0 with `headBefore == headAfter` (no new commits) is treated
by the orchestrator as a stall → escalate, never accept.

## Model chains (loop-models.conf)

```sh
CHAIN_TASK=("codex:gpt-5.6-sol" "claude:opus+sonnet")
CHAIN_ESCALATE=("claude:fable+opus")
CHAIN_REVIEW=("claude:fable" "codex:gpt-5.6-sol" "claude:opus")
CODEX_EXTRA_ARGS=(-c 'model_reasoning_effort="xhigh"')   # "ultra" tier; add delegation cfg here
LOOP_BUDGET_USD=15        # per attempt, claude legs only (codex has no budget flag)
LOOP_TIMEOUT_TASK=2700    # 45m
LOOP_TIMEOUT_ESCALATE=1800
LOOP_TIMEOUT_REVIEW=2400
LOOP_MAX_PARALLEL=3
```

Leg grammar: `engine:model[+fallback[,fallback2]]`. The `+` list maps to Claude's native
`--fallback-model` (comma-separated; CLI retries the primary each turn) — so intra-Claude
fallback is one leg. Codex→Claude hops are the wrapper's job. Claude legs use model
aliases (`fable`, `opus`, `sonnet`) so the newest generation resolves automatically.

Failure classing per leg (from structured error events first — `.is_error` result events in
Claude stream-json, `error` events in Codex JSONL — stderr regex last):

- transient (`5xx|overloaded|ECONNRESET|ETIMEDOUT|empty result`) → retry same leg, 3
  attempts total, backoff 10/30s;
- rate/usage (`429|rate.?limit|usage limit|quota`) → advance to next leg immediately;
- other nonzero → one retry, then next leg.

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

FIRST: run `$kestral-pickup --auto <project> phase:<N>` (Claude: `/kestral-pickup`) — it
claims your task and loads context. If it returns `REFUSED: <reason>`, write
RUN_DIR/status.json with outcome "blocked" and the reason as details, and stop.

RULES: never push; never open PRs; Kestral only through the pickup/handoff auto skills
(never update the plan document or link PRs); never ask the user in prose; stay inside
this worktree. Commit per logical step with descriptive messages. Follow the repo's
CLAUDE.md conventions (TDD where it applies).

PHASE (from the shared plan):
<verbatim phase block: title, Depends on, Touches, Done when, Verify, Notes>

CONTEXT: <goal + key decisions from plan header; prior-attempt failure summary;
answers/<phase>.md content; hil answer — whichever exist>

WHEN FINISHED (Done when holds and `<verify cmd>` passes locally): run
`$kestral-handoff --auto phase:<N> status:in-progress lane:<X> engine:<engine>` (comment:
implementation complete, awaiting merge — "done" is the orchestrator's post-merge call),
then write RUN_DIR/status.json exactly per this schema <schema> and end the session. If
you must stop early, skip the handoff and write status.json with outcome question/blocked.
```

## Merge policy (loop-merge.sh, serialized — one merge at a time)

`loop-merge.sh --worktree <int-wt> --lane-branch <b> --verify-cmd '<effort verify>'`
(the script refuses to push unverified without an explicit `--no-verify`):

- **0** — merged, verified, pushed (also idempotent `{"already":true}` for a re-merge).
- **2** — textual conflict, merge left in progress, `{"conflicts":[files]}` on stdout.
  The orchestrator resolves it itself in the integration worktree (use
  `resolving-merge-conflicts`, honoring both phases' *Done when*), then
  `loop-merge.sh --worktree <int-wt> --finish --verify-cmd '<effort verify>'`.
  Not confident → `--abort` + HIL.
- **4** — merge committed but Verify failed (semantic conflict). Orchestrator reverts the
  merge commit (`git revert -m1 HEAD`) and escalates L3.
- **3** — lane branch missing: reconcile from git (already merged + deleted → treat as
  done; otherwise escalate). Occurs after crash-resume repairs.
- **1** — merge failed to start (dirty integration worktree) — clean it, don't "resolve".

**First successful merge → open the draft PR** (`gh pr create --draft` from the
integration branch); later merges just push. Only the orchestrator pushes the integration
branch; plain `git push` (fast-forward only — never force). Lane branches never get
pushed. Cleanup order: switch or remove the lane worktree **first**, then
`git branch -d <lane-branch>` (a branch checked out in a worktree can't be deleted); a
lane continuing to its next phase reuses its worktree via
`git -C <wt> switch -c <next-branch> <integration>`. Lanes launched later cut from the new
tip; already-running lanes pick up siblings' work at their own merge time.

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
3. `loop-notify.sh --level question --title "HIL: <phase>" --body "<question>"`.
4. Ask in-session (AskUserQuestion / plain question). Other lanes keep running. On answer:
   phase back to ready with the answer injected; status back to `in-progress`.

## Kestral mapping

Split: **runners** own task-scoped ops via the auto skills; the **orchestrator** owns the
plan document and everything cross-task.

- Claim (runner, via `kestral-pickup --auto`): `claim_task_and_branch { taskId,
  branchName: <lane branch> }` on the branch the orchestrator pre-created — re-claiming
  your own branch is a no-op; a 409 means someone else owns it → `REFUSED` →
  status.json `blocked` → HIL, never steal. The claim performs no git action.
- Runner handoff (via `kestral-handoff --auto ... status:in-progress`): task progress
  comment + status kept task-scoped; **never** `update_document`, never PR links.
- After merge (orchestrator): `update_task_status` (statusKey via `list_statuses`) +
  `post_progress_comment` (2–4 conversational lines, noting lane + engine) + flip
  `[status: done]` in the plan doc + append Progress log + `update_document`. "done" in
  loop mode = merged into the integration branch (the human PR-merge gate applies to the
  single effort PR).
- At effort completion (orchestrator, not at PR creation): `link_pr_to_task { taskId,
  prUrl }` once for **every** phase task (tasks carry plural prLinks; dedup via state.json
  `linkedPrTasks`).
- Completion (orchestrator): statuses → the workspace's awaiting-review status; plan
  `Status: integrating`; `trigger_brain_build`.

## Crash-resume

`kestral-loop resume`: re-acquire the lock with the run id from state.json (a different
owner means another orchestrator claims this effort — stop and ask before `--force`), then
reconcile three sources — **git >
Kestral > state.json** (git is truth for code, Kestral for claims, state for attempt
bookkeeping):

- attempt marked running: pid alive → re-attach (watch transcript mtime; stale >25m → kill,
  treat as 124); pid dead + status.json present → process it normally; pid dead + none →
  failed attempt.
- `MERGE_HEAD` in integration worktree → finish or abort the merge before anything else.
- Phase `done` in Kestral but lane branch not merged (or vice versa) → repair from git.
- Runners are spawned detached, so they survive orchestrator death; never double-spawn a
  phase whose run dir has a live pid.
