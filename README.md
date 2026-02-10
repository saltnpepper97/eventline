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
- Per-outcome exit messages for clean “done:” lines
- Emission gating (log level) that does NOT affect recording
- Console and file sinks (toggleable, configurable formatting)

---

## What is eventline?

eventline is a small, predictable journaling/logging crate built for real systems software.

Core rule:

Always record the full structured history.  
Only gate emission (console/file output) with log levels and sink toggles.

This makes eventline ideal for post-mortems, debugging, and deterministic “what happened?” replay.

---

## Quick Start

Add it to your project:

    [dependencies]
    eventline = "0.6.1"

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

(Exact formatting depends on runtime settings.)

---

## Recording vs Emission (important)

- Recording always happens — the journal stores the full structured history.
- Emission is controlled by global log level and enabled sinks.

You get full fidelity history without spamming stdout.

---

## Runtime Configuration

Console output:

    eventline::runtime::enable_console_output(true);
    eventline::runtime::enable_console_color(true);
    eventline::runtime::enable_console_timestamp(false);
    eventline::runtime::enable_console_duration(true);

File output (append):

    eventline::runtime::enable_file_output("/tmp/app.log")?;

Disable all output (still records):

    eventline::runtime::disable_all_output();

Set global log level (emission threshold only):

    eventline::runtime::set_log_level(eventline::runtime::LogLevel::Debug);

---

## Design Notes

- Records are append-only and never rewritten
- Scopes are created on enter and finalized exactly once on exit
- The journal is the canonical execution history
- Rendering and runtime only affect presentation, not data

---

## Module Layout

- core/     — data types only (Record, Scope, ids, Outcome, guards)
- journal/  — append-only buffer, writers, structured fields
- render/   — canonical formatting for console and file output
- runtime/  — global config, sinks, filtering policy
- macros    — info!, debug!, warn!, error!, scope!

---

## License

MIT
