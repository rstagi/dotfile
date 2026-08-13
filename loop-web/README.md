# Loop Observatory (`loop-web`)

A **live** web graph for the loop-engineering flow, rendered as a left-to-right graph
that live-updates to show what each node is working on — model, branch, attempt, problems
(verify-fail / crash / timeout / blocked / chain-exhausted), HIL escalations, and pending
steering notes.

It runs as a **perpetual central daemon** (a launchd LaunchAgent on `127.0.0.1:7717`). It is the
**base backend for every loop**: `multiphase-plan` registers a plan on it (status `planned`)
before any run starts, and every `loop-execute` run **registers** with it and **pushes clean
lifecycle events** (via `loop-emit.sh`, sourced by the `loop-*.sh` scripts) *in addition to* the
daemon watching `.loop/` on disk. Status is therefore **authoritative, never stale**: a
`phase.attempt.finish{done,exit0}` event promotes a phase to `done` even when the
orchestrator's `state.json` bookkeeping lags (a monotone `todo<claimed<running<done<merged`
lattice — ranks never regress). Every loop is kept **forever** in a per-loop JSON store
(`~/.loop/loops/<runId>.json`), reviewable after its worktree is gone; a header **selector**
switches between loops. Kestral is an **opt-in linked backend**, not a requirement.

It reads `.loop/` and folds pushed events. The one write path is the node drawer's
steering-note editor, which writes or clears the same `notes/<phase>.md` /
`notes/pr-review.<owner--repo>.md`
files as `loop-state.sh note`; it never writes the plan. A distinct aqua NOTE badge remains
visible until that phase or review finishes.

For a **two-tier self-recycling** run, the header also surfaces the sub-orchestrator's live
occupancy + recycle count (a `sub-orch: N recycles · ~Xk tok` chip, fed by `sub.recycle` /
`sub.saturation` events) and the selector shows a per-loop pending-HIL count.

## Run

```bash
../loop-web.sh --daemon              # the central daemon (what the LaunchAgent runs)
../loop-web.sh                       # legacy single-loop: discover .loop/ from cwd (walk up)
../loop-web.sh --plan fixtures/plan.md   # static: preview a plan, no loop running
../loop-web.sh --dir fixtures/.loop      # observe one specific flattened loop dir
```

`install.sh loop-web` renders + bootstraps the `com.rstagi.loop-web` LaunchAgent (auto-start on
login + KeepAlive); `loop-emit.sh` also ensure-starts it when a loop begins. Open
`http://localhost:7717` and pick a loop from the selector.

## Develop

```bash
npm install
npm test          # vitest over the pure model layer (src/model)
npm run dev       # Vite dev server (proxies /events + /api to the Node server on 7717)
npm run build     # build dist/
npm run typecheck # tsc --noEmit
```

For live dev you run two processes: `npm run dev` (UI on 5173) and
`node server/index.mjs --dir <loop>` (data on 7717); Vite proxies the data endpoints.

## Architecture

- **`src/model/`** — pure, TDD-first parsing + derivation. Takes already-read file *contents*
  (strings/objects), never touches the filesystem, so it is fully unit-testable with fixtures.
  It is **isomorphic**: the Node server imports it via native TS type-stripping (no build step
  for the backend), and Vite bundles it for the browser's "paste plan" preview.
- **`server/index.mjs`** — zero-dep Node HTTP server, a `Map<runId, entry>` registry bound to
  `127.0.0.1`. Does all I/O, then folds each ingest through the pure model (`reduce-loop.ts`) and
  renders it (`materialize.ts`). Per live loop: `fs.watch` (2s debounce) **plus a 15s reconcile**
  that self-heals missed POSTs — a hung runner emits no fs event, yet its stale heartbeat must
  still flip a node to flatline. Endpoints: `POST /api/loops/:runId/{register,state,event,finish,note}`,
  `GET /api/loops` (selector), `GET /events?runId=` (per-loop SSE), `/api/loops/:runId/{plan,
  snapshot,review,attempt/:slug/:k}` (`review?repository=<owner/repo>` selects a repository;
  bare `review` is the legacy alias; `plan` returns `{runId,effort,status,integrationBranch,
  planText}` with no worktree needed — how a fresh checkout fetches the plan; `snapshot`/`review`/
  `attempt` 410 when the worktree is gone), `/api/health`; back-compat `/api/model` ·
  `/api/snapshot` · bare `/events` resolve to the default loop. A register-only record surfaces
  as status **`planned`** (plan on the daemon, not yet running); the first state/event flips it
  to `active`.
- **`server/store.mjs`** — per-loop persistence (`~/.loop/loops/<runId>.json`), atomic
  mktemp+rename, per-loop debounce, sync flush on finish + SIGTERM/SIGINT, `loadAll` on startup
  (corrupt file → skip + warn). Pure serialize/parse (with the event cap) live in
  `src/model/store-serde.ts`; the reducer + materialize are `src/model/{reduce-loop,materialize}.ts`.
- **`src/ui/`** — Vite + React + TypeScript. React Flow for the DAG, Framer-Motion CSS for the
  three load-bearing motions (heartbeat, flatline, HIL alarm). "Phosphor Bench" oscilloscope theme.

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

### Layout note

Layout is a deterministic longest-path layered layout with horizontal swimlane bands
(`src/ui/graph/layout.ts`). elkjs was intentionally dropped: its `layered` algorithm doesn't model
horizontal lanes as a first-class concept, and our DAGs are tiny, so a deterministic layout gives
  guaranteed clean bands with a synthetic Plan root and stacked repository PR Review terminals.

Plans declare GitHub `owner/repo` slugs and assign one repository to every phase. Runtime
state/store schema v2 keeps independent integration branches, base SHAs, PRs, and reviews per
repository; v1 scalar records migrate to an implicit `primary` repository. Local checkout
mappings live outside plans in `~/.loop/repos.json`, managed by `../loop-repo.sh`.
