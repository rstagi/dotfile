# Loop daemon (`loop-web`)

The loop-web daemon stores plans and lifecycle events for the old Loop engine.
It runs as a launchd LaunchAgent on `127.0.0.1:7717`.
Use `ws top` to view loops and change steering controls.

`multiphase-plan` registers a plan before a run starts.
The Loop scripts send lifecycle events through `loop-emit.sh`.
The daemon also reads each registered `.loop/` directory.
Events promote phase status through the `todo<claimed<running<done<merged` lattice.
Each loop persists in `~/.loop/loops/<runId>.json`, even after its worktree disappears.

Steering endpoints write the same `notes/<phase>.md` and `.loop/control/` files as the Loop tools.
The legacy note path is `notes/pr-review.<owner--repo>.md`.
Plan registration and review-tier changes can update plan text.

## Run

```bash
../loop-web.sh --daemon                 # central daemon
../loop-web.sh                          # discover .loop/ from the current directory
../loop-web.sh --plan fixtures/plan.md   # serve model data for a static plan
../loop-web.sh --dir fixtures/.loop      # observe one flattened loop directory
```

`install.sh loop-web` installs dependencies and configures the `com.rstagi.loop-web` LaunchAgent.
The daemon starts on login and remains running.
`loop-emit.sh` also starts it when a loop begins.

## Develop

```bash
npm install
npm test          # model and server tests
npm run typecheck # tsc --noEmit
```

The server imports TypeScript through Node's native type stripping.
It needs Node 22.18 or later and no build step.

## Architecture

- **`src/model/`** contains pure parsing and derivation functions.
  These functions take file contents as strings or objects and perform no filesystem operations.
  The server folds events through `reduce-loop.ts` and creates snapshots through `materialize.ts`.
- **`server/index.mjs`** provides the HTTP API and performs filesystem operations.
  Each live loop uses `fs.watch` with a two-second debounce and a 15-second reconciliation interval.
  The interval detects stale runner heartbeats when no file event arrives.
- **`server/store.mjs`** persists one JSON record per loop through an atomic rename.
  It flushes on finish, SIGTERM and SIGINT.
  Pure serialization functions live in `src/model/store-serde.ts`.
- **`vitest.config.ts`** configures the model and server tests with a Node environment.

The API provides these endpoints:

- `POST /api/loops/:runId/{register,state,event,finish,note,control}` records lifecycle events and steering changes.
  Controls include pause, resume, model and review tier.
- `GET /api/loops` lists loops.
- `GET /events?runId=` streams loop snapshots through server-sent events.
- `GET /api/loops/:runId/{plan,snapshot,review,attempt/:slug/:k}` returns plan data, snapshots and live logs.
  `review?repository=<owner/repo>` selects a repository.
  Archived snapshots remain available after the worktree disappears.
  Live logs and steering require the worktree.
- `GET /api/health` reports daemon health.
- Legacy `/api/model`, `/api/snapshot` and bare `/events` select the default loop.

A registered plan starts as `planned`.
The first state or lifecycle event changes it to `active`.

### Milestones

A plan may carry several `## Milestone N — …` sections.
The model and `ws top` group phases by milestone.
Each milestone ends with a `[gate: merge]` review.
After the gate verdict, the loop status becomes `awaiting-merge`.
The daemon records `milestone.awaiting-merge` and `state.awaitingMerge`, with one PR link per repository.
A human merges and runs `/loop-execute resume`.
The `milestone.resumed` event changes `state.milestone` and the branch, PR and review fields.

A plan header with `engine: skills` selects the skills engine.
The old orchestrator and `/loop-execute <plan>` refuse it and name `ws loop start <loop dir>`.
At a merge gate, `/loop-execute resume` keeps steps 1–4 and starts the selected engine in step 5.
See [Engine switch and rollback](../.claude/skills/loop-execute/references/loop-protocol.md#engine-switch-and-rollback) for the switch and recovery commands.

At resume, `loop-plan.sh archive` moves each shipped milestone into `milestones/m<N>.md`.
It leaves a stub section whose heading ends with `` `[archived]` ``.
The parser reads the stub's `Phases (all done)` list as existing `done` phases.
A dependency on an archived phase is satisfied.
The milestone carries `archive: { file, outcome }`, and its model data carries `archived: true`.
The pure transform lives in `src/model/archive-milestone.ts`.
A pending `milestones/m<N>.retro.md` precedes `## Raw log` in the archive.
Its Outcome bullet becomes the stub's `Outcome:`.
For an archived milestone, `attachRetro` adds the retro to the existing archive file.

To load updated status and events, restart the Loop daemon after upgrading.
Restart the loop-web LaunchAgent with `launchctl kickstart -k`, or run `../loop-web.sh --daemon` when no LaunchAgent runs it.

### AR review protocol

New PR-review phases use `[review: standard|max]` (default standard), with `[rounds: N]`
and `[inner: N]` both defaulting to 2 (1..5). A reviewer R reviews a frozen artifact; critic C
audits the review from another provider. GPT→Claude, Claude→Grok, and Grok→GPT rotate across
outer rounds. Standard uses Sol / Opus / Grok; max uses Astra / Fable / Grok. Only the fixer
edits code between rounds. The first review's `APPROVE` plus C's `AGREE` triggers a third-provider
confirmation: `AGREE` skips later rounds and proceeds to final; a veto makes that provider the
critic for the remaining inner exchanges. Disagreement at the inner cap writes `.disputed.md`
for verification against code. An Opus final reviewer posts the sole GitHub review.

The daemon reads `runs/review-p<N>-<owner--repo>.ledger.tsv`: six columns (`a<k>`, stage,
role, provider, verdict token, run directory relative to `.loop/`). Roles are
`review|critic|respond|confirm|disputed|fix|final`. The model folds these completed runs and
infers skipped rounds from clean confirmation; no skipped ledger rows are stored. Resume uses
`loop-review.sh recover`, reconciles the GitHub final marker, then calls `next`. Legacy
shallow/medium plans remain readable, but execution and the tier endpoint
reject those tiers.

After pulling the AR update, restart with
`launchctl kickstart -k gui/$(id -u)/com.rstagi.loop-web`, check
`curl -fsS http://localhost:7717/api/health`, and retag active legacy review phases to standard
or max before execution. See [the protocol](../.claude/skills/loop-execute/references/loop-protocol.md).

### Jev status and replay

Snapshots contain the latest Jev mode, decision count and fallback count.
Phase data keeps the recommendation, applied action, confidence, fallback and evidence separate.
Archived snapshots retain these fields.

To inspect agreement and fallback behavior, replay one or more persisted records:

```bash
node ../loop-jev-replay.mjs ~/.loop/loops/<run-id>.json | jq .
```

This is read-only. Operational metrics are `unavailable` unless recorded, and synthetic fixture
reports must not be interpreted as time or cost savings.

### Derivation is grounded in the real scripts, not the docs

The parser is built to what `~/dotfile/loop-*.sh` **actually** writes:

- `meta.json.engineExit` is `{0, 40, 124}` — 124 = timeout, 40 = chain-exhausted, 0 = otherwise.
  Its mere existence means "attempt ended". The semantic 10/12/20/50 codes are not in any file.
- **verify-fail** (exit 12) can't be told from a passing verify by `verify.log` alone — both
  write it. The only file-observable signal is the `spawn.log` line
  `claimed done but verify failed`, so that is the disambiguator.
- **crash** = `meta.json` present but no valid `status.json.outcome`.
- Heartbeat = `transcript.jsonl` mtime; stale > 25 min ⇒ flatline (the `loop-protocol.md` threshold).
- Phase ↔ attempt mapping is `state.json`-driven: `state.phases[N].slug` is the `runs/<slug>-a<K>` prefix.

Cross-file skew (e.g. `state=running` while `meta.json` already exists) is normal/transient and
is never rendered as an error.

Plans declare GitHub `owner/repo` slugs and assign one repository to every work phase;
explicit review phases cover all repositories. Re-registering edited plan text preserves
completed phase ranks and reopens a finished loop when unfinished phases are appended. Runtime
state/store schema v2 keeps independent integration branches, base SHAs, PRs, and reviews per
repository; v1 scalar records migrate to an implicit `primary` repository. Local checkout
mappings live outside plans in `~/.loop/repos.json`, managed by `../loop-repo.sh`.
