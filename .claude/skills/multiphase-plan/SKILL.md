---
name: multiphase-plan
description: >-
  Turn a goal into a multi-phase implementation plan, marking any genuinely independent
  lanes that could run in separate Conductor worktrees (and staying fully sequential when
  the work doesn't parallelize), then register it on the local Loop daemon as the shared
  source of truth. Use when asked to "plan this out", "make a multi-phase plan", "break
  this into parallel phases", or before spinning up parallel worktrees for a larger effort.
  For Loop plans, estimate and size phases to fit one 30-minute execution attempt where the
  work can be bounded. The draft is cross-checked by an adversarial plan review (Codex + Claude + Grok) unless invoked as `light <goal>`.
argument-hint: "[light] <goal> (optional)"
---

# Multi-Phase Plan

Produce a correct phased plan, and register it on the local **Loop daemon**
(http://localhost:7717) as the shared source of truth so any coding agent — **Claude
Code or Codex**, in any worktree — can pick it up and continue. *Where the work genuinely allows it*, mark phases into **lanes** that
can be worked concurrently in separate worktrees (Conductor). Pairs with `loop-pickup`
(start a lane in a fresh worktree) and `loop-handoff` (reconcile progress and repush the
plan).

**Correctness before parallelism.** The job is a plan that reflects the real shape of the
work. Parallelism is an *observation* about that plan (some phases happen to be
independent), never a *goal* you bend the plan toward. A fully sequential, single-lane plan
is a normal, expected, good outcome — most plans are mostly sequential. More lanes is not
better.

**Modes.** By default the drafted plan goes through an **adversarial plan review** (step 3)
before it is registered. If the argument starts with the word `light` (e.g.
`/multiphase-plan light add a --json flag`), strip it from the goal and skip step 3 — use it
for small or low-risk plans.

**Front-load the interaction.** All questions go to the user in step 1, before drafting. From
step 2 on, work autonomously through review and registration to the report; come back to the
user only for a decision the review surfaced that genuinely needs them (step 3).

## Prerequisites

`jq` and the **Loop daemon** (http://localhost:7717). The daemon is auto-ensured for you by
`loop-plan.sh register` / `loop-emit.sh`, so no manual setup is needed.

## Workflow

### 1. Understand the goal

The argument (or the conversation) is the goal. Build enough understanding to phase the
work well — **do not plan blind**:

- Explore the codebase for the areas the work touches (dispatch an `Explore`/general
  agent or read the relevant files). Note the modules, seams, and shared files that
  phases will contend over — this drives the parallelization analysis.
- **Question gate — ask everything now, once.** After exploring, list every unknown that
  would change the plan and ask them all in **one batch** (`AskUserQuestion` in Claude Code;
  a numbered list elsewhere), each with your recommended default. Cover whatever is not
  already settled by the request or the code:
  - scope & non-goals; definition of done
  - hard constraints and sequencing (deadlines, must-land-first, compatibility)
  - repositories touched and each repository's `Verify:` gate (never invent one)
  - integration branch, when the default would be wrong
  - review tier (`shallow`/`medium`/`max`, rounds) — propose one from size and risk

  Settle the "silly" things here — naming, which repo, which command runs the tests — so the
  adversarial review spends its effort on the plan's real weaknesses. Ask a follow-up batch
  only if an answer opens a new unknown. Don't ask what the code or the request already
  answers; if nothing is open, say so and move on. Offer `grilling` if the user wants the
  plan stress-tested first.

### 2. Draft the phased plan

Author the plan in the **canonical plan format** (see
`references/plan-format.md` — read it and follow it exactly; `loop-handoff` and
`loop-pickup` parse the same markers). The plan must decide:

- **Phases** — each a coherent, independently-reviewable unit of work with a clear
  *Done when* (acceptance criteria). Decompose the work into the phases it *naturally* has
  — the phases you'd write with no parallelism in mind at all. Order them; record
  `Depends on` edges. **Title each phase as an imperative outcome** ("Add OAuth token
  refresh endpoint"), never "Phase 1" or a vague label — the title usually seeds the PR
  title.
- **Terminal review phase** — every executable plan ends with one explicit
  `` `[kind: pr-review]` `` phase in lane `review`, depending on every terminal work phase.
  It represents the bounded per-repository reviews run by `loop-execute`; it has no
  `Repository:`, branch, or Verify command. **If the request already names a review tier**
  (e.g. "review: max", "shallow review", "max review, 2 rounds" — from the user or a calling
  agent's args), use it verbatim (with `[rounds: N]` if given) and skip the heuristic below.
  Otherwise propose its depth with
  `` `[review: shallow|medium|max]` `` from the effort's size and risk: `shallow` for small,
  low-risk changes; `medium` (the default — omit the tag) for normal work; `max` for large
  efforts or anything touching security, auth, money, or data integrity. Add
  `` `[rounds: N]` `` only to deviate from the tier default (1/3/3, max 5). State the chosen
  tier and why in the plan so the user can override it. The plan remains editable during
  execution and after review; follow the insertion rules in `references/plan-format.md`.
- **Time bounds for Loop plans** — estimate each phase's active work from pickup through
  implementation, focused verification, handoff, and the expected merge gate. Inspect the
  touched code and verification commands; use actual durations from comparable Loop
  attempts when available. Record a range, confidence, and brief basis in `Estimate:`.
  Use a plausible slow-case upper bound, not an optimistic midpoint. Aim for an upper
  bound of 20–25 minutes, leaving room before the runner's 30-minute
  checkpoint. Split larger work into smaller verifiable outcomes while preserving the
  vertical-slice and lane rules below. Do not invent precision: if uncertainty remains,
  label it low confidence and state the risk. A verification command that alone exceeds
  30 minutes, or work that cannot be split coherently, is an explicit exception to report,
  not a phase to label "30 min". Queue waits, retries, and the terminal review pipeline are
  outside work-phase estimates and can make elapsed wall time longer.
- **Repositories** — resolve every touched checkout to its GitHub `owner/repo` slug from
  its `origin`; declare it under `## Repositories`, give it a mandatory repository-wide
  `Verify:` command, and put `Repository:` on every work phase. One work phase belongs to one
  repository; a lane may hop repositories. Store no checkout paths in the plan. Default the
  shared integration branch to the current checkout's branch when it is not the repository's
  default branch (loop-execute adopts it), else `<type>/<effort-slug>`; override per
  repository only when needed. Newly authored
  plans always use repository blocks; the legacy scalar format is read-only compatibility.
- **Slices, not layers** — for full-stack work, each phase is a **vertical slice**: the
  backend *and* the frontend of one user-visible capability ship in the same phase, never
  "all the backend phases, then all the frontend phases". (Within a slice the backend is
  still built first, the frontend taps in after — but as one phase, one branch, one PR.)
  If a slice is genuinely too big for one phase, split it into adjacent backend and
  frontend phases — but they stay in the **same lane**, share the **same `Suggested
  branch`**, and land in the **same PR**; mark each with "same PR as Phase N" in its
  Notes. A layer-only phase is legitimate only when the work itself is single-layer
  (pure API change, pure UI polish) or a shared foundation several slices build on.
- **Lanes** — **default: one lane, fully sequential.** A lane is a chain of phases one
  worktree owns start-to-finish. Only after the phases exist, look for independence and
  split into separate lanes — and only when they pass the **Independence test** below.
- **Parallelization guide** — *only if there are 2+ lanes.* For each lane, which worktree
  runs it and what it can start on immediately; the **integration points** (where lanes
  merge and who owns the merge); and a **conflict watch** listing files/areas that multiple
  lanes touch. If there's one lane, omit this section.
- **Verify** — every repository has a runnable **Verify:** command (exit 0 = pass). A
  phase may override it with a narrower command that checks its *Done when*. Ask the user
  for missing repository gates; never invent them. When the effort runs under
  `loop-execute`, emit Loop config + Repositories per `references/plan-format.md`.
- **Suggested branch** per phase — a **descriptive** `<type>/<imperative-outcome-slug>`
  (e.g. `feat/oauth-token-refresh`, `refactor/invoice-retry-state`), so `loop-pickup`
  claims it deterministically and the worktree/commits/PR all read clearly.
  **Name the work, not the index:** never `phase-4`, `<effort>-phase-N`, or a bare number —
  the phase↔branch link is carried by this plan doc, so it must not clutter the branch name. See "Naming" in `references/plan-format.md`. Exception:
  phases that split one vertical slice (see **Slices, not layers**) share a single branch —
  name it after the capability, not the layer.

#### Independence test (the safety gate of this skill)

Two phases may go in **different lanes** only when **all three** hold. If any fails, keep
them in the **same** lane:

1. **No dependency** — neither needs the other's output; both can start from the freshly
   fetched GitHub default branch of their respective repositories.
2. **Low file contention** — they don't edit the same files/modules. Incidental overlap
   (a shared type, a config line) is tolerable only if a tiny, one-time interface is
   settled up front and listed in the conflict watch — not if they'd fight over the same
   core files.
3. **No coordination tax** — parallelizing them does **not** require stubs, mocks, frozen
   interfaces, feature flags, duplicated scaffolding, or any seam you wouldn't otherwise
   write. If parallelism only works by adding artificial structure or making "something
   weird" happen, it isn't worth it — sequence them instead.

**Never reshape, split, or reorder the natural phases to manufacture parallelism.** When in
doubt, keep phases sequential — that is always the safe answer. If the work is inherently
sequential, produce a single-lane plan and say so plainly; that is a success, not a
shortfall. Silent over-parallelization causes merge pain and is the exact failure this gate
exists to prevent.

### 3. Adversarial plan review (skipped in `light` mode)

Write the draft to `.loop/plan.md` (with the header from step 4) and run three independent,
read-only reviewers — Sol (Codex), Opus (Claude), Grok — in parallel, one round:

    ~/dotfile/loop-plan-review.sh run     # defaults: --dir .loop, --plan-file .loop/plan.md, --worktree .

It blocks until all three finish (≤15 min each; models/effort in `loop-models.conf`
`CHAIN_PLAN_REVIEW_*`) and prints one `<reviewer> \t <exit> \t <report.md|->` row per
reviewer. Exit 2 = no usable report: tell the user and continue unreviewed. A failed
reviewer is noted, not retried. Grok sends the repository to xAI.

**Reconcile here, in this chat.** Read every report. For each finding, check it against the
code yourself, then accept (edit the plan) or reject (one-line reason). Merge duplicates
across reviewers — agreement between providers raises confidence, but isn't proof. Show the
user a compact disposition table (`finding · reviewers · accept/reject · why`). Only findings
that need a **user decision** (scope, product trade-off, a missing gate) go back to the user,
batched into one question; settle everything else yourself. Then continue to step 4.

### 4. Register on the daemon

**This always runs** — every plan lives on the daemon. Write the
plan markdown to `.loop/plan.md` in the current worktree (already there unless `light`), prefixed with the canonical
`<!-- loop-plan -->` header (see `references/plan-format.md`). Mint the `planId`
(`loop-<effort-slug>-<date>-<HHMMSS|rand>`) and set `daemon: http://localhost:7717`:

    <!-- loop-plan
    planId: loop-<effort-slug>-<date>-<HHMMSS|rand>
    daemon: http://localhost:7717
    -->

Then register it:

    loop-plan.sh register            # defaults to --dir .loop, --plan-file .loop/plan.md

`register` POSTs `/register` to the daemon (auto-starting it if it isn't up) and prints the
`planId`; the plan lands with daemon status `planned`. This lets the current worktree start
immediately and lets `loop-handoff` repush from here. Mention the user can add `.loop/` to
`.gitignore` if they don't want the plan committed.

The review tier was settled in step 1 — don't ask again. (It stays changeable later, before
the review phase starts: rewrite its `` `[review: …]` `` / `` `[rounds: …]` `` tags and
`loop-plan.sh push`, or `m` on the review phase in `loop-top`.) Only if step 1 left it open —
e.g. the user skipped it — ask it now, in one message.

### 5. Report

State the shape honestly — do not imply parallelism that isn't there. Fill every `<…>` in
the templates below with the real captured value (planId, phase & lane counts) — never print
a literal placeholder. The pickup/execute argument is the **planId** (or effort name).

**Single-lane (sequential) plan — the common case:**

> **Plan registered** · N phases, sequential · plan review: `<n accepted / n findings>` (or `light`).
> State backend: **local daemon** · planId `<id>` · Observatory http://localhost:7717
>
> This work is sequential — each phase builds on the last, so it runs in one worktree in
> order. Start with **Phase 1**: run **`/loop-pickup <planId>`** (Codex: **`$loop-pickup`**).
> When a phase advances, run **`/loop-handoff`** to repush the plan.
>
> **Run the loop:** **`/loop-execute <planId>`** (Codex: **`$loop-execute`**),
> hands-off once registered.

**Multi-lane plan — only when phases passed the Independence test:**

> **Plan registered** · N phases in M lanes · plan review: `<n accepted / n findings>` (or `light`).
> State backend: **local daemon** · planId `<id>` · Observatory http://localhost:7717
>
> **Parallel lanes:**
> - **Lane A** (worktree 1): Phase 1 → Phase 3 — start now.
> - **Lane B** (worktree 2): Phase 2 — independent, start now.
> - **Integration:** Phase 4 merges A + B (single worktree, after both).
> - **Conflict watch:** Phases 2 & 3 both touch `src/api/*` — rebase before merge.
>
> In each new Conductor worktree, run **`/loop-pickup <planId>`** (Codex: **`$loop-pickup`**)
> and claim its lane. When a lane advances, run
> **`/loop-handoff`** to reconcile and repush the plan.
>
> **Run the loop:** **`/loop-execute <planId>`** (Codex: **`$loop-execute`**) runs all lanes
> hands-off.

If you split into lanes, be ready to justify each split against the Independence test if the
user asks. If it was a close call, prefer reporting it as sequential and note the *optional*
parallelism rather than presenting it as the required structure.

## Cross-agent notes

The durable contract is the **daemon plan (planId) + `.loop/plan.md`** — not this chat, so a Codex agent in another
worktree resumes from the same state a Claude Code agent wrote. Keep the plan mechanics to
`loop-plan.sh` + git + local files only; do not rely on any
host-specific feature. Reference both invocation styles (`/name` for Claude Code,
`$name` for Codex).
