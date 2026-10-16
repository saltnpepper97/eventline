# eventline
An application flight recorder for Rust: structured execution history, scoped outcomes, and logging views.

![GitHub last commit](https://img.shields.io/github/last-commit/saltnpepper97/eventline?style=for-the-badge)
![License](https://img.shields.io/badge/License-MIT-E5534B?style=for-the-badge)
![Rust](https://img.shields.io/badge/Rust-1.89%2B-orange?style=for-the-badge&logo=rust&logoColor=white)
![Status](https://img.shields.io/badge/Status-Active-28A745?style=for-the-badge)

eventline records structured execution history (events + scopes) first. Console logs, text files, JSONL files, and replay helpers are views over that journal.

---

## Features

- Append-only journal of structured records (events + scope exits)
- RAII scopes with duration tracking and success/failure/aborted outcomes
- Result-aware scopes that record `Ok` as success and `Err` as failure
- Per-outcome exit messages for clean `done:` lines
- Emission gating (sink log levels) that does NOT affect recording
- Structured fields on events
- JSONL rendering/export for machine-readable journals
- Console and file sinks (toggleable, configurable formatting)
- Automatic log rotation with configurable size limits and backup retention
- Thread-local runtime scope stacks so concurrent threads do not share active scope state
- Executor-independent async scope contexts that follow futures across polls and threads
- Dropped async-writer record counters and observable writer setup errors
- Replay helpers for scope trees, failed scopes, slowest scopes, and filtered records
- Optional `log` and `tracing` bridges behind feature flags
- Run headers written as raw bytes at the start of each log session

---

## What is eventline?

eventline is a small, predictable execution journal built for real systems software. It should be treated as a local flight recorder: the journal answers "what happened?", while logging output answers "what should I show right now?"

Core rule:

> Record structured history independently from output sinks.
> Only gate emission (console/file output) with log levels and sink toggles.

This makes eventline useful for post-mortems, debugging, and deterministic "what happened?" replay without turning every debug line into console noise.

---

## Quick Start

Add it to your project:

    [dependencies]
    eventline = "0.9.0"

Initialize once early:

    eventline::init_sync();

`eventline::runtime::init().await` is still available when async startup code prefers that shape.

Log events:

    eventline::info!("starting");
    eventline::debug!("debug details: {}", 123);
    eventline::warn!("something looks off");
    eventline::error!("something failed: {}", "oops");

Attach structured fields:

    eventline::info!("user login", user_id = 42, method = "oauth");

Create a scope:

    eventline::scope!("config", {
        eventline::info!("loading config");
        // work...
    });

Scopes with exit messages:

    eventline::scope!(
        "config",
        success="loaded",
        failure="failed",
        aborted="aborted",
    {
        // work...
    });

This produces a final line like:

    done:  config#12 loaded [success] (3.2ms)

Default text output uses a calm, aligned lowercase style:

    debug: loading config {path="config.toml"}
    info:  server listening {addr="127.0.0.1:8080"}
    warn:  retrying request {attempt=2}
    error: request failed {status=500}
    done:  request#12 served [success] (18.4ms)

*(Exact formatting depends on runtime settings.)*

---

## Result-Aware Scopes

Use `scope_result!` when the block returns a `Result`. Both a returned `Err`
and an early `?` record `Failure`; `Ok` records `Success`:

```rust
fn load_config() -> Result<String, std::io::Error> {
    eventline::scope_result!("config", success = "loaded", failure = "failed", {
        let config = std::fs::read_to_string("config.toml")?;
        Ok(config)
    })
}
```

The macro returns the original result. Its block runs in a closure: `?` and
`return` leave that closure. If the surrounding function should return early
too, use the macro as its final expression (as above) or apply `?` to the macro
call. `break` and `continue` cannot target a surrounding loop.

The closure API is also available:

```rust
let result = eventline::scope_result("config", || {
    std::fs::read_to_string("config.toml")
});
```

For async work, use `scope_async_result`:

```rust
let result = eventline::scope_async_result("request", async {
    // Async operations and `?` can go here.
    Ok::<_, std::io::Error>(42)
}).await;
```

This uses the same context isolation as `scope_async`. `Ok` records `Success`,
`Err` records `Failure`, dropping a pending future after it starts records
`Aborted`, and unwinding records `Failure`. An unpolled future creates no scope.
Wrap each spawned or concurrently polled operation separately.

Existing guards support `exit_result(&result)`, which borrows the result
without consuming it. To report application-specific cancellation explicitly,
use `guard.aborted()` or `guard.exit(eventline::core::Outcome::Aborted)`:

```rust
let mut guard = eventline::scope_guard("config");
if cancellation_requested {
    guard.aborted();
    return Err(Error::Cancelled);
}
let result = read_config();
guard.exit_result(&result);
result
```

When this guard spans `.await`, run it inside `ScopeContext::wrap` as described
below. An explicitly selected outcome is never overwritten by `exit_result`
or by dropping the guard. Returning a cancellation error through an automatic
result scope still counts as `Err` and records `Failure`; use explicit abortion
when that distinction matters. Neither successful values nor errors need to
implement `Clone`, `Debug`, `Display`, or `std::error::Error`.

Ordinary `scope!`, `scope_guard`, and `scope_async` retain their existing drop
and completion behavior. They do not inspect return values; use the result-aware
APIs when a `Result` should determine the outcome.

---

## Async Scopes

Use `scope_async` for operations that hold scopes open across `.await`:

```rust
let answer = eventline::scope_async("request", async {
    eventline::info!("processing request");
    eventline::scope_async("lookup", async {
        // Async work can suspend here without leaking its active scope.
        42
    }).await
}).await;
```

The parent context is captured when `scope_async` is called. The named scope
starts on the first poll. Each wrapped future carries an independent scope
stack, installed only while that future is polled or dropped. This supports
interleaved tasks on one thread and tasks moving between worker threads, with
no executor dependency or feature flag.

Completion records `Success`, cancellation after starting records `Aborted`,
and unwinding records `Failure`. An unpolled future creates no scope.
Completion does not inspect the future's output: returning `Err` still counts
as completion. Use `scope_async_result` to derive the outcome from a `Result`,
or a `scope_guard` inside a context wrapper to choose an outcome explicitly.

For existing guards or scopes, propagate context with `ScopeContext::capture()`.
For example, with Tokio, create the wrapper before spawning:

```rust
eventline::scope_async("request", async {
    let context = eventline::ScopeContext::capture();
    let worker = tokio::spawn(context.wrap(async {
        let guard = eventline::scope_guard("worker");
        tokio::task::yield_now().await;
        eventline::info!("working under request");
        guard.success();
    }));
    worker.await.unwrap();
}).await;
```

Cloning a context gives each wrapped future its own stack. Use
`ScopeContext::new().wrap(future)` for work with no inherited scope. Capturing a
context does not keep its scopes open; await child work before exiting its
parents. Executors do not automatically propagate context to spawned tasks.

Wrap each concurrently polled operation separately, including siblings in
`join!` and `select!`. Wrapping only their common outer future cannot isolate
unwrapped siblings. Existing `scope!`, `scope_guard`, and `enter_scope` calls
can cross `.await` inside an isolated wrapper. Outside a wrapper, these APIs
retain their lightweight synchronous, thread-local behavior and should not
span suspension points. Existing guards keep their usual drop outcomes.

---

## Recording vs Emission (important)

- **Recording** happens independently from output sinks — by default all built-in levels are recorded.
- **Emission** is controlled by per-sink log levels and enabled sinks.

You get full fidelity history without spamming stdout.

If you need to reduce formatting/recording overhead, set an explicit recording
threshold:

    eventline::set_record_level(eventline::LogLevel::Info);

The in-memory runtime journal is bounded, so long-running processes should use a
file sink or JSONL export if they need durable history.

Set runtime journal retention:

    eventline::set_journal_retention(10_000);

---

## Runtime Configuration

Console output:

    eventline::runtime::enable_console_output(true);
    eventline::runtime::enable_console_color(true);
    eventline::runtime::enable_console_scope_labels(false);
    eventline::runtime::enable_console_scope_exits(false);
    eventline::runtime::enable_console_timestamp(false);
    eventline::runtime::enable_console_duration(false);

Per-sink levels:

    eventline::runtime::set_console_level(eventline::runtime::LogLevel::Info);
    eventline::runtime::set_file_level(eventline::runtime::LogLevel::Debug);

File output (append, no rotation):

    eventline::runtime::enable_file_output("/tmp/app.log")?;

JSONL file output:

    eventline::runtime::enable_file_output_jsonl("/tmp/app.jsonl")?;

Export the current in-memory journal as JSONL lines:

    let lines = eventline::records_jsonl();

File output with automatic rotation:

    use eventline::{LogPolicy, RunHeader};

    eventline::runtime::enable_file_output_rotating(
        "logs/app.log",
        LogPolicy::default(),              // 5 MiB max, 5 backups
        Some(RunHeader::new("my-app")),    // with PID annotation
    )?;

Rotation renames the active log to `app.log.1`, shifts older backups up, and
opens a fresh `app.log`. The oldest backup beyond `keep_backups` is silently
dropped.

Disable all output (still records):

    eventline::runtime::disable_all_output();

Set global output fast-path level (normally derived from enabled sinks):

    eventline::runtime::set_log_level(eventline::runtime::LogLevel::Debug);

Inspect writer health:

    let dropped = eventline::dropped_writer_records();
    let last_error = eventline::last_writer_error();

---

## Log Rotation

`LogPolicy` controls when and how rotation happens:

    use eventline::LogPolicy;

    // Custom: 10 MiB max, keep 3 backups
    let policy = LogPolicy::new(10 * 1024 * 1024, 3);

    // Or use the defaults (5 MiB, 5 backups)
    let policy = LogPolicy::default();

Rotation is triggered automatically before a record is written that would push
the file past `max_bytes`. Set `keep_backups` to `0` to simply delete the log
on rotation rather than keeping any history.

---

## Replay

The journal can be inspected directly:

    let records = eventline::records();
    let scopes = eventline::scopes();

    let tree = eventline::replay::render_scope_tree(&scopes);
    let failed = eventline::replay::failed_scopes(&records, &scopes);
    let slowest = eventline::replay::slowest_scopes(&records, &scopes, 10);

This is the main difference between eventline and ordinary logging: log output is only one way to view the execution history.

---

## Ecosystem Bridges

Core eventline stays dependency-light. Existing instrumentation can be bridged with optional features:

    [dependencies]
    eventline = { version = "0.9.0", features = ["log", "tracing"] }

`log` facade bridge:

    eventline::integrations::log::init(log::LevelFilter::Debug)?;

`tracing` event bridge:

    use tracing_subscriber::prelude::*;

    let subscriber = tracing_subscriber::registry()
        .with(eventline::integrations::tracing::EventlineLayer);

    tracing::subscriber::set_global_default(subscriber)?;

The tracing layer currently records tracing events into eventline. Native eventline scopes remain the stronger scoped-outcome model.

---

## Run Headers

A `RunHeader` is a single decorated line written as raw bytes at the start of a
log session, before structured records begin. It is always visible in the file
regardless of log level.

    use eventline::runtime::RunHeader;

    // With PID:
    // ==================== my-daemon (pid=18432) ====================
    RunHeader::new("my-daemon")

    // Without PID:
    // ====================== my-daemon ======================
    RunHeader::without_pid("my-daemon")

    // Custom width:
    RunHeader::new("my-daemon").with_width(80)

When appending to an existing non-empty log file, a blank separator line is
inserted automatically before the header so run boundaries are clearly visible.

---

## Design Notes

- Records are append-only and never rewritten
- Scopes are created on enter and finalized exactly once on exit
- The journal is the canonical execution history
- Rendering and runtime only affect presentation, not data
- Synchronous runtime scope context is thread-local; explicit async wrappers carry their own context
- In-memory retention is bounded and configurable
- Writer queues are bounded; dropped output records are counted
- Rotation and headers operate on raw bytes and bypass the record pipeline intentionally

---

## Module Layout

- `core/`    — data types only (Record, Scope, ids, Outcome, guards)
- `journal/` — append-only buffer, writers, rotation logic, structured fields
- `render/`  — canonical formatting for console and file output
- `runtime/` — global config, sinks, filtering policy, run headers
- `replay`   — replay helpers for scope trees and summaries
- `integrations` — optional `log` and `tracing` bridges
- `macros`   — `info!`, `debug!`, `warn!`, `error!`, `scope!`, `scope_result!`

---

## License

MIT
