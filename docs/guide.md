# Eventline user guide

This guide covers eventline 0.10.0. Start with the [README](../README.md) for
installation, or see the [changelog](../CHANGELOG.md) when upgrading.

- [Recording events](#recording-events)
- [Result-aware scopes](#result-aware-scopes)
- [Async scopes](#async-scopes)
- [Scope timing](#scope-timing)
- [Recording and output](#recording-and-output)
- [Configuration](#configuration)
- [Log rotation](#log-rotation)
- [Replay](#replay)
- [Integrations](#integrations)
- [Run headers](#run-headers)

## Recording events

Call `eventline::init_sync()` once during startup. The async
`eventline::runtime::init().await` entry point is also available.

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

## Result-aware scopes

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
fn load_config(cancelled: bool) -> Result<String, std::io::Error> {
    let mut guard = eventline::scope_guard("config");
    if cancelled {
        guard.aborted();
        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
    }
    let result = std::fs::read_to_string("config.toml");
    guard.exit_result(&result);
    result
}
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

## Async scopes

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
For example, in an application already using Tokio, create the wrapper before
spawning (the application needs Tokio's `rt` feature):

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

## Scope timing

Live scope durations use `std::time::Instant`. Exit records store the measured
duration in `duration_ns` without rounding it to milliseconds. Precision
depends on the platform's monotonic clock. Once a scope exits, its duration is
frozen, including when async work moves between threads.

Human-readable and exported timestamps still use `SystemTime`: record
`time_ns` is nanoseconds since the Unix epoch, while scope `entered_at` and
`exited_at` remain milliseconds since the Unix epoch. Clock adjustments can
change these timestamps without changing the measured duration. These timestamp
fields keep their existing units; JSONL version 1 adds the metadata described in the
[format reference](jsonl-format.md).

Query accurate elapsed time for retained runtime scopes:

```rust
let guard = eventline::scope_guard("request");
let id = guard.id();
let running = eventline::scope_elapsed(id).unwrap();
guard.success();
let finished = eventline::scope_elapsed(id).unwrap();
```

For a direct journal, use `journal.scope_elapsed(id)`. Both queries return
`None` after the scope is cleared or trimmed. Internal clocks are removed with
their scopes; they are never serialized.

`Scope::elapsed()` remains a millisecond wall-clock estimate for standalone
metadata, preserving existing struct literals and serialized data. Use the
monotonic query for live work and the exit record's `duration_ns` for completed
history, JSONL export, and replay.

---

## Recording and output

- **Recording** happens independently from output sinks — by default all built-in levels are recorded.
- **Emission** is controlled by per-sink log levels and enabled sinks.

You get full fidelity history without spamming stdout.

If you need to reduce formatting/recording overhead, set an explicit recording
threshold:

    eventline::set_record_level(eventline::LogLevel::Info);

The runtime retains 500 records by default and trims periodically, so record
counts can briefly exceed the configured cap. The same cap applies to closed
scopes; open scopes remain available for later exits and updates. Long-running
processes should use a file sink or JSONL export when they need saved history.

Set runtime journal retention:

    eventline::set_journal_retention(10_000);

---

## Configuration

Use `setup` to resolve sink levels and install a configuration in one request:

```rust
async fn configure_logging() -> std::io::Result<()> {
    eventline::setup(eventline::Setup {
        verbose: true,
        level: None,
        console_level: Some(eventline::LogLevel::Info),
        file_level: Some(eventline::LogLevel::Debug),
        file: Some(eventline::FileSetup::Jsonl { path: "app.jsonl".into() }),
        journal_retention: Some(10_000),
    }).await
}
```

Per-sink overrides take precedence over `level`. Without an explicit `level`,
the shared fallback is Debug when `verbose` is true and Info otherwise.
`verbose` also controls console enablement. `file: None` preserves the existing
file sink; `Some(FileSetup::Off)` disables it. Failed file setup leaves the
previous configuration active. The individual runtime setters below are useful
for later changes.

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

Exports include retained scope metadata as well as records. JSONL sinks write
scope metadata when scopes begin, even if those scopes emit no events. Text
output keeps its existing behavior.

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
    let health = eventline::writer_diagnostics();
    // health.dropped_records, health.failed_operations, health.last_error

File initialization returns open, rotation, and run-header write errors. A
failed replacement keeps the previously active sinks and configuration. File
enablement preserves the configured file level, including `Setup::file_level`
and its shared fallback. A failed setup request can be retried with the same
path after fixing the underlying problem.

Background write and flush failures increment `failed_operations` and update
`last_error`. These diagnostics live in memory, retain only the latest error
text plus cumulative counters, and survive successful writes and writer
reconfiguration. `eventline::runtime::clear()` drains pending output, then
resets the journal and these diagnostics. No diagnostic log files are created.
The failure count covers failed operations, not lost records or individual
sinks; one operation may fail on more than one sink.

Call `eventline::flush()?` at an appropriate boundary to wait for queued output.
It returns the first background write error since the previous flush even if
the underlying file flush succeeds, or a flush/worker error. Reporting an error
does not retry the record, and a later successful flush does not erase the
diagnostics. Runtime reconfiguration also drains output before opening,
rotating, or writing a header to the replacement file. Multiple sinks are all
attempted even when one fails.

The default queue policy remains nonblocking and counts output entries dropped
under pressure, including JSONL scope metadata as well as records. To wait for capacity instead, opt into blocking backpressure:

```rust
eventline::set_writer_mode(eventline::WriteMode::Blocking)?;
// This policy is also preserved when calling eventline::setup(...).
eventline::enable_file_output("app.log")?;
eventline::info!("queued without a queue-pressure drop");
eventline::flush()?;
```

Blocking mode waits on the calling thread, so use it where blocking is
acceptable. It prevents queue-pressure drops but cannot prevent disk errors,
retry partial writes safely, or guarantee persistence after a power loss.
`flush` does not call `fsync`. Use `WriteMode::BestEffort` to restore the default.
Standalone `AsyncWriter` users can select a policy with
`try_spawn_with_mode` and inspect the writer's `diagnostics()` snapshot.

---

## Log rotation

`LogPolicy` controls when and how rotation happens:

    use eventline::LogPolicy;

    // Custom: 10 MiB max, keep 3 backups
    let policy = LogPolicy::new(10 * 1024 * 1024, 3);

    // Or use the defaults (5 MiB, 5 backups)
    let policy = LogPolicy::default();

Rotation is triggered automatically before a record is written that would push
the file past `max_bytes`. Set `keep_backups` to `0` to simply delete the log
on rotation rather than keeping any history.
Set `max_bytes` to `0` to disable rotation. Missing backup files are harmless;
other rotation errors are returned during setup or reported through the
background diagnostics and `flush`.

---

## Replay

The journal can be inspected directly:

    let records = eventline::records();
    let scopes = eventline::scopes();

    let tree = eventline::replay::render_scope_tree(&scopes);
    let failed = eventline::replay::failed_scopes(&records, &scopes);
    let slowest = eventline::replay::slowest_scopes(&records, &scopes, 10);

Load an earlier run after restarting the application:

```rust
let saved = eventline::replay::load_jsonl("app.jsonl")?;
for run in &saved.runs {
    println!("{}", eventline::replay::render_scope_tree(&run.scopes));
    let failed = eventline::replay::failed_scopes(&run.records, &run.scopes);
    let unfinished = run.unfinished_scopes();
    println!("{} failed, {} unfinished", failed.len(), unfinished.len());
}
for issue in &saved.issues {
    eprintln!("{issue:?}");
}
```

`load_jsonl_files(paths)` combines rotated files in the supplied order, normally
oldest first. `read_jsonl(reader)` accepts any buffered reader. Loading does not
change the live journal or runtime configuration. The returned records/scopes
work with the existing replay helpers.

The loader preserves valid entries around damaged or truncated lines and
reports recovery issues with source paths and line numbers. Missing exits stay
unfinished; no success or abortion is invented after a crash. Missing ancestors
are marked as placeholders. Only surviving exit records establish outcomes.
Inspect `saved.issues` whenever interpreting incomplete history.

New JSONL uses format version `1` and a run identity on every structured entry,
so restarted applications can reuse IDs without mixing histories. The runtime
keeps the same identity when sinks are rebuilt. Existing unversioned files are
accepted as legacy version `0`; their unmarked restart boundaries can be
ambiguous. Unsupported future versions are explicitly reported and skipped.
See the [JSONL format and recovery reference](jsonl-format.md) for schemas,
compatibility rules, and recovery limits.

Inspect saved files from the command line with the example:

```sh
cargo run --example replay_file -- app.jsonl.2 app.jsonl.1 app.jsonl
```

For repeated queries over one snapshot or saved run, build a borrowed index:

```rust
let records = eventline::records();
let scopes = eventline::scopes();
let index = eventline::replay::ReplayIndex::new(&records, &scopes);
for scope in &scopes {
    let count = index.records_for_scope(scope.id).count();
    println!("{:?}: {count} records", scope.id);
}
```

`ReplayIndex` also provides `scope(id)`, `record(id)`, and `scope_outcomes()`.
It borrows immutable input, preserves record order and conflicting observations,
and avoids cloning records for scope queries. Index each independent saved run
separately because IDs are local to a run. The existing one-off helpers remain
available; tree and outcome reconstruction now index scopes internally.

---

## Performance

The journal uses indexed scope lookups and deque storage for record eviction.
Record-ID indexes are built on the first ID query, keeping their allocation and
update cost out of recording-only workloads. Open scopes remain retained when
old closed scopes are evicted. Snapshots keep their original insertion order.

Run benchmarks for event recording, concurrent runtime logging, scope nesting,
retention churn, ID lookups, and replay queries:

```sh
cargo bench --locked --bench journal
```

See [performance measurements and methodology](performance.md) for the
before/after results, workload sizes, and indexing tradeoffs.

---

## Integrations

Core eventline stays dependency-light. Existing instrumentation can be bridged with optional features:

    [dependencies]
    eventline = { version = "0.10.0", features = ["log", "tracing"] }

`log` facade bridge:

    eventline::integrations::log::init(log::LevelFilter::Debug)?;

`tracing` event bridge:

    use tracing_subscriber::prelude::*;

    let subscriber = tracing_subscriber::registry()
        .with(eventline::integrations::tracing::EventlineLayer);

    tracing::subscriber::set_global_default(subscriber)?;

The tracing layer currently records tracing events into eventline. Native eventline scopes remain the stronger scoped-outcome model.

---

## Run headers

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

## Design notes

- Records are append-only and never rewritten
- Scopes are created on enter and finalized exactly once on exit
- Scope durations use a monotonic clock; wall-clock timestamps are retained for display
- The journal is the canonical execution history
- Rendering and runtime only affect presentation, not data
- Synchronous runtime scope context is thread-local; explicit async wrappers carry their own context
- In-memory retention is bounded and configurable
- Writer queues are bounded; pressure drops and I/O failures are counted separately
- Rotation and headers operate on raw bytes and bypass the record pipeline intentionally

---

## Module layout

- `core/`    — data types only (Record, Scope, ids, Outcome, guards)
- `journal/` — append-only buffer, writers, rotation logic, structured fields
- `render`   — canonical formatting for console and file output
- `runtime/` — global config, sinks, filtering policy, run headers
- `replay`   — saved JSONL loading/recovery, scope trees, and summaries
- `integrations` — optional `log` and `tracing` bridges
- `macros`   — `info!`, `debug!`, `warn!`, `error!`, `scope!`, `scope_result!`
