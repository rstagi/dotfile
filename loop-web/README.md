# Loop Observatory (`loop-web`)

A **live, read-only** web graph for the Kestral loop-engineering flow. When `kestral-loop`
runs a plan autonomously, all of its state is written to local files under `.kestral/loop/`.
This app watches those files, normalizes them, and renders the plan as a left-to-right graph
that live-updates to show what each node is working on — model, branch, attempt, problems
(verify-fail / crash / timeout / blocked / chain-exhausted), and HIL escalations.

It **never writes** to `.kestral/loop/` or the plan. HIL is answered in chat to the
orchestrator, exactly as today.

## Run

```bash
../loop-web.sh                       # discover .kestral/ from the cwd (walk up)
../loop-web.sh --plan fixtures/plan.md          # static: preview a plan, no loop running
../loop-web.sh --dir fixtures/.kestral/loop     # observe a specific loop dir
../loop-web.sh --port 7717
```

The launcher runs `node server/index.mjs`, which serves the built `dist/` plus the data
endpoints. Open `http://localhost:7717`.

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
- **`server/index.mjs`** — zero-dep Node HTTP server. Does all I/O, then calls the model.
  `fs.watch` (2s debounce) **plus a mandatory 15s reconcile timer** — a hung runner emits no fs
  event, yet its stale heartbeat must still flip a node to flatline. Endpoints: `GET /events`
  (SSE — a full snapshot on connect and per change; state is tiny, no deltas), `GET /api/model`
  (snapshot for first paint), `GET /api/attempt/:slug/:k` (lazy log tails for the drawer).
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
guaranteed clean bands with a synthetic Plan root and PR Review terminal spanning every lane.
