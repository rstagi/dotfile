# S5: Rust daemon, tmux driver, and hook round trip

## Question

Can one Rust binary control a tmux window and send a stop hook to a daemon over a Unix socket?
What are the build times and hook round-trip p95?

## Method

Date: 2026-10-16.
Environment: Apple M4, 24 GiB RAM, macOS 26.6.2, Rust 1.95.0, tmux 3.8, and Python 3.15.0.
All measurements use the debug build with the default Cargo development profile.
Agent home mode: none. This spike makes zero agent calls.

The integration test uses `tmux -L ws-rs-test-<pid>` with `/dev/null` as its initial tmux configuration.
It starts a detached keepalive session and creates one test window through the Rust driver.
The tmux server and daemon use a temporary `HOME`, `WS_HOME`, `WS_STATE`, and `WS_SOCK` under `/tmp`.
`WS_TMUX_SOCKET` identifies that sandbox server for hooks.
The test removes the sandbox server and daemon when it ends, including assertion failures.

The test performs these steps:

1. Create a window with a Python process that reads its terminal in raw mode.
2. Set `@ws_state` to `working`, then read it through the driver.
3. Enable bracketed paste in the process and wait for tmux to report that mode.
4. Paste text with quotes, shell characters, and a newline.
5. Check the received bytes, including `ESC [ 200 ~` and `ESC [ 201 ~`.
6. Run `ws hook stop` 100 times from that same process inside the window.
7. Read the daemon's hook records through `GET /v1/hooks`.
8. Check that all 100 records contain `event: stop` and the created window id.

Python measures each invocation with `time.monotonic_ns()` around `subprocess.run`.
The measured interval includes process creation, CLI parsing, tmux pane lookup, HTTP transport, daemon recording, and the response.
The p95 uses the nearest-rank method: sample 95 of 100 sorted samples.
The test has a timeout for completion but no machine-dependent latency assertion.

Run the test from the repository root:

```sh
cd wsd
cargo test -p ws --test tmux -- --nocapture
```

Build measurements use an isolated workspace copy and a fresh target directory under `/tmp`.
They use `cargo build --locked --offline`, with dependencies already present in the local registry.
The clean measurement includes compilation of all dependencies but excludes downloads.
The unchanged measurement repeats the command.
The edited measurement adds one comment to `crates/ws/src/main.rs` in the temporary copy, then repeats the command.
Wall time includes Cargo startup. Each build measurement is one observation.

Reproduce the build measurements from the repository root:

```sh
python3 - <<'PY'
import os, pathlib, shutil, subprocess, tempfile, time
with tempfile.TemporaryDirectory(prefix='ws-s5-build-', dir='/tmp') as directory:
    root = pathlib.Path(directory)
    workspace = root / 'wsd'
    shutil.copytree('wsd', workspace, ignore=shutil.ignore_patterns('target'))
    environment = dict(os.environ, CARGO_TARGET_DIR=str(root / 'target'))
    for mode in ['clean', 'unchanged', 'edited']:
        if mode == 'edited':
            main = workspace / 'crates/ws/src/main.rs'
            main.write_text(main.read_text() + '\n// Incremental build probe.\n')
        start = time.perf_counter()
        subprocess.run(['cargo', 'build', '--locked', '--offline'],
                       cwd=workspace, env=environment, check=True)
        print(mode, round(time.perf_counter() - start, 3), 'seconds')
PY
```

## Findings

The sandbox test passed. The pasted bytes matched exactly, including the newline and both paste delimiters.
All 100 hooks reached the daemon and carried the correct window id.

| Measurement | Result |
|---|---:|
| Clean debug build, fresh target | 8.538 s |
| Incremental build, unchanged source | 0.089 s |
| Incremental build, one CLI file edited | 0.303 s |
| Hook round-trip p50, 100 invocations | 7.842 ms |
| Hook round-trip p95, 100 invocations | 9.125 ms |
| Hook round-trip maximum | 11.262 ms |

These values describe one local run. They do not measure release builds, agent execution, or performance under sustained load.

The workspace keeps the crate choices from Phase 2. This spike adds no dependency:

| Crate or library | Locked version | Use |
|---|---|---|
| `tokio` | 1.53.3 | Async runtime, Unix socket, signals, and request timeout |
| `axum` | 0.8.9 | Versioned HTTP routes and JSON extraction |
| `hyper`, `hyper-util` | 1.12.0, 0.1.21 | HTTP client over the Unix socket |
| `http-body-util` | 0.1.5 | Request bodies and response collection |
| `clap` | 4.6.7 | `daemon`, `ping`, and `hook stop` commands |
| `serde`, `serde_json` | 1.0.229, 1.0.154 | Shared hook types and JSON transport |
| `tempfile` | 3.27.0 | Integration-test sandboxes |
| Rust standard library | 1.95.0 | tmux processes, paste input, and bounded hook storage |

The driver invokes tmux with argument arrays. It uses a unique buffer for each paste.
`load-buffer` receives the text through stdin. `paste-buffer -d -p -r` preserves newlines and requests bracketed paste.
tmux adds the delimiters only when the receiving application enables bracketed paste.
The driver removes the buffer after a failed paste.

`ws hook stop` resolves `TMUX_PANE` to a window id through tmux, then sends `POST /v1/hooks`.
The daemon records the event before it returns HTTP 201.
The CLI prints no output on success. Runtime errors exit 1.
The HTTP request has a two-second timeout.
Tests also check invalid window ids, unsupported events, and execution without a tmux pane.

The spike keeps the latest 256 hooks in memory. A daemon restart loses them.
It does not implement session identity, event ordering, durable logs, spool replay, or agent hook configuration.
The tmux subprocess itself has no deadline.

Validation passed: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo build`, and `cargo test`.
The Rust suite passed eight integration tests, including the sandbox round trip.
The repository shell suite, `zsh tests/run.sh`, also passed.

## Verdict

**go** for S5.

The Rust workspace supports the required tmux operations and the complete hook transport path.
The measured p95 is below Phase 19's 50 ms target for this minimal path.
Phase 19 must measure latency again after it adds durable recording and spool behavior.

## Impact on the plan

- Phase 13 can extend this driver with session creation, panes, other window options, capture, and liveness checks.
- Phase 18 can use `ws hook stop` as the initial transport path for the shared agent preparation contract.
- Phase 19 must replace the memory store with session log events and add ordering, state updates, and daemon-down spool behavior.
- Phase 19 must add a deadline for tmux lookup and repeat the p95 measurement with the full ingest path.
- S5 needs no architecture fallback or new crate. The milestone 1 review still assesses the spike verdict.
