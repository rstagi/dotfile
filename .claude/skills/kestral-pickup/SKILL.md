---
name: kestral-pickup
description: >-
  Resume a Kestral-tracked multi-phase effort in a fresh chat/worktree: download the shared
  plan document from Kestral, pick (or accept) a parallel lane, conflict-check it against
  sibling worktrees, claim the phase task + branch, and load just enough context to start.
  Use at the start of a new Conductor worktree or clean chat, or when asked to "pick up the
  plan", "resume the Kestral effort", "start a lane", or "continue where the handoff left off".
argument-hint: "<project name> [lane/phase] (optional)"
---

# Kestral Pickup

The receiving end of `multiphase-plan` and `kestral-handoff`. A fresh chat has no memory of
prior sessions — this skill rebuilds working context from Kestral (the durable source of
truth), so a **Claude Code or Codex** agent in **any** worktree can grab an unblocked lane
and start. Designed for Conductor's parallel worktrees: each worktree runs this and claims a
different lane.

## Prerequisites

The **Kestral** MCP server must be in this session. Auth is at the MCP layer (OAuth) —
proceed directly; on auth failure (401, `unauthorized`, `Not authenticated`) tell the user
to reconnect via their host UI (Claude Code: `/mcp`; Codex: Settings → MCP Servers →
Authenticate) and stop. Keep Kestral IDs internal; show `slug - title` and readable
names + URLs.

## Workflow

### 1. Find the effort and download the plan

Resolve the project + plan document:

1. **Argument:** project name / plan-doc URL / task slug → `entity_lookup` (URL/slug) or
   `search_operations`→`search_projects` / `find_documents`.
2. **Local copy already here:** if `.kestral/plan.md` exists in the worktree, use its header
   coordinates (`projectId`, `workContextId`) — but still re-fetch from Kestral to get the
   latest, since another worktree may have handed off since.
3. **Ask** which project to resume if ambiguous.

Download the current plan: `execute_operation("get_document_content", { workContextId })`
(follow `isTruncated`/`nextOffset` if paged). Save it to `.kestral/plan.md` with the header
comment block (see `../multiphase-plan/references/plan-format.md`). Optionally pull the
Project Brain for background (`entity_lookup({ type: "project_brain", id: projectId })`, or
invoke `kestral-context`) — keep it to a short digest, don't dump it.

### 2. Pick a lane

Parse the plan's phases, lanes, and `[status: …]` markers, and reconcile with live task
status (`entity_lookup` on the phase tasks, or `list_tasks_by_status`). Then:

- **If the user named a lane/phase** → use it.
- **Otherwise** show the lanes with their state and recommend the next **unblocked** one
  (all its `Depends on` phases are `done`, and it isn't already `in-progress` in another
  worktree):

  > **[project] — [effort]** · N phases in M lanes.
  > - **Lane A:** Phase 1 ✅ done · Phase 3 ⏳ ready ← recommended
  > - **Lane B:** Phase 2 🔒 claimed (in progress, another worktree)
  > - **Integration:** Phase 4 — blocked on Lane A + B
  >
  > Which lane should this worktree take? (recommend Lane A / Phase 3)

Do not start integration phases until their dependency phases are `done`.

### 3. Conflict-check before claiming

Apply `kestral-sync`'s **Conflict Check** so two worktrees don't collide:

- Is the chosen phase's task already assigned / `in-progress` (claimed by another worktree)?
  If so, warn with assignee + status and offer a different lane.
- Read the plan's **Conflict watch**: if a sibling lane in flight touches the same files,
  surface it now so the user sequences merges deliberately.

### 4. Claim the phase

Confirm with the user, then claim the task + branch. Prefer invoking **`kestral-sync`**
(it owns branch derivation, `claim_task_and_branch`, status discovery, and the 409
already-linked conflict signal). Use the phase's **Suggested branch** from the plan as the
branch name. Create/switch to that git branch in the worktree.

> Claiming **[slug] - <phase title>** on branch `phase-3-...`, set to In Progress. Proceed?

### 5. Load focused context and start

Pull only what this phase needs — its task description (*Depends on* / *Touches* /
*Done when*), the relevant plan sections, and any linked docs/brain bullets. Summarize in a
few lines, then explore the code paths the phase *Touches* and outline the first concrete
steps. Hand control back to the user to begin implementation.

> Ready on **Phase 3 — <title>**. Done when: <criteria>. Touches: <areas>. First steps: …
> When this lane advances or you switch worktrees, run **`/kestral-handoff`**
> (Codex: **`$kestral-handoff`**) to repush the plan.

## Cross-agent notes

Everything needed to resume comes from the **Kestral plan doc + phase tasks**, re-downloaded
fresh — so it doesn't matter which agent or worktree wrote the last handoff. Use only
Kestral MCP + git + local files; reference both `/name` and `$name` invocation. Delegate
claim/status/conflict mechanics to `kestral-sync` rather than reimplementing them.
