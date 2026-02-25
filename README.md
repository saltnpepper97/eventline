# eventline
Structured journaling + logging for Rust, with scopes you can replay.

![GitHub last commit](https://img.shields.io/github/last-commit/saltnpepper97/eventline?style=for-the-badge)
![License](https://img.shields.io/badge/License-MIT-E5534B?style=for-the-badge)
![Rust](https://img.shields.io/badge/Rust-1.90%2B-orange?style=for-the-badge&logo=rust&logoColor=white)
![Status](https://img.shields.io/badge/Status-Active-28A745?style=for-the-badge)

eventline records a complete, append-only execution history (events + scopes), while letting you control what gets emitted to console/files via log level and sink toggles.

---

## Features

- Append-only journal of structured records (events + scope exits)
- RAII scopes with duration tracking and success/failure/aborted outcomes
- Per-outcome exit messages for clean `done:` lines
- Emission gating (log level) that does NOT affect recording
- Console and file sinks (toggleable, configurable formatting)
- Automatic log rotation with configurable size limits and backup retention
- Run headers written as raw bytes at the start of each log session

---

## What is eventline?

eventline is a small, predictable journaling/logging crate built for real systems software.

Core rule:

> Always record the full structured history.  
> Only gate emission (console/file output) with log levels and sink toggles.

This makes eventline ideal for post-mortems, debugging, and deterministic "what happened?" replay.

---

## Quick Start

Add it to your project:

    [dependencies]
    eventline = "0.7.3"

Initialize once early:

    eventline::runtime::init().await;

Log events:

    eventline::info!("starting");
    eventline::debug!("debug details: {}", 123);
    eventline::warn!("something looks off");
    eventline::error!("something failed: {}", "oops");

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

    done: config#12 loaded [success] (3.2ms)

*(Exact formatting depends on runtime settings.)*

---

## Recording vs Emission (important)

- **Recording** always happens — the journal stores the full structured history.
- **Emission** is controlled by global log level and enabled sinks.

You get full fidelity history without spamming stdout.

---

## Runtime Configuration

Console output:

    eventline::runtime::enable_console_output(true);
    eventline::runtime::enable_console_color(true);
    eventline::runtime::enable_console_timestamp(false);
    eventline::runtime::enable_console_duration(true);

File output (append, no rotation):

    eventline::runtime::enable_file_output("/tmp/app.log")?;

File output with automatic rotation:

    use eventline::runtime::{LogPolicy, RunHeader};

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

Set global log level (emission threshold only):

    eventline::runtime::set_log_level(eventline::runtime::LogLevel::Debug);

---

## Log Rotation

`LogPolicy` controls when and how rotation happens:

    use eventline::runtime::LogPolicy;

    // Custom: 10 MiB max, keep 3 backups
    let policy = LogPolicy::new(10 * 1024 * 1024, 3);

    // Or use the defaults (5 MiB, 5 backups)
    let policy = LogPolicy::default();

Rotation is triggered automatically before a record is written that would push
the file past `max_bytes`. Set `keep_backups` to `0` to simply delete the log
on rotation rather than keeping any history.

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
- Rotation and headers operate on raw bytes and bypass the record pipeline intentionally

---

## Module Layout

- `core/`    — data types only (Record, Scope, ids, Outcome, guards)
- `journal/` — append-only buffer, writers, rotation logic, structured fields
- `render/`  — canonical formatting for console and file output
- `runtime/` — global config, sinks, filtering policy, run headers
- `macros`   — `info!`, `debug!`, `warn!`, `error!`, `scope!`

---

## License

MIT
