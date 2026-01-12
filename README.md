# Eventline

**Causality-aware execution journal for systems-level programs.**

Eventline records *what happened*, *when it happened*, and *in what causal context* — without assuming logging, tracing, or telemetry semantics. Built for daemons, CLI tools, and eventually Linux distributions.

---

## Features

- **Append-only journal** — never mutates or removes records
- **Scoped execution** — track outcomes and durations
- **Event kinds** — Info, Warning, Error, Debug (separate from outcomes)
- **Runtime log levels** — filter events globally (Debug, Info, Warning, Error)
- **Dual output mode** — journal + optional real-time console printing
- **Live logging** — automatic, timestamped append to disk
- **Unified color control** — consistent ANSI colors across console and renderer
- **Flexible filtering** — by outcome, depth, duration, event kind, message content
- **Human-readable output** — Unicode bullets, optional color coding
- **High-throughput batching** — `JournalBuffer` for batch writes
- **Dual-layer API** — pure core + optional runtime facade
- **Async runtime support** — fire-and-forget logging and async scopes (`scoped_async()`)
- **Deterministic replay** — safe concurrent reads, reliable audit trails

---

## Quick Start

### Runtime API (Fire-and-Forget)

For applications and daemons:

```rust
use eventline::runtime;
use eventline::runtime::log_level::{set_log_level, LogLevel};
use eventline::{event_info, event_warn, event_error, event_debug, event_scope};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    runtime::init().await;

    // Enable dual output: journal + console
    runtime::enable_console_output(true).await;
    runtime::enable_console_color(true).await;

    // Enable live logging to disk
    let log_path = PathBuf::from("/tmp/eventline.log");
    runtime::enable_live_logging(log_path).await;

    // Set runtime log level
    set_log_level(LogLevel::Warning).await;

    event_info!("This will NOT be logged").await;
    event_warn!("Cache approaching limit").await;          // Journaled + printed + live
    event_error!("Database connection failed").await;      // Journaled + printed + live
    event_debug!("Verbose debug info").await;              // Filtered out

    event_scope!("RequestHandler", {
        event_info!("Processing request").await;           // Filtered out
        event_warn!("Retry attempt failed").await;         // Journaled + printed + live
    }).await;

    // Journal is now live-appended automatically; no manual flush required
}
```

### Runtime API (Async Scopes)

```rust
use eventline::runtime;
use eventline::{event_info, event_scope_async, event_scope_unnamed};

#[tokio::main]
async fn main() {
    // Initialize runtime
    runtime::init().await;

    // Enable console output
    runtime::enable_console_output(true).await;
    runtime::enable_console_color(true).await;

    // Fire-and-forget logging
    event_info!("Async logging example").await;

    // Named async scope
    event_scope_async!("AsyncTask", {
        event_info!("Inside async scope").await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }).await;

    // Unnamed async scope (note: requires 'async' keyword)
    event_scope_unnamed!(async {
        event_info!("Unnamed async work").await;
    }).await;
}
```

### Core Journal API (Explicit Control)

For libraries or embedded systems:

```rust
use eventline::journal::Journal;
use eventline::journal::outcome::Outcome;
use eventline::journal::writer::JournalWriter;

let mut journal = Journal::new();

journal.scoped(None, Some("Task"), |journal, scope| {
    journal.record(Some(scope), "Starting task");
    journal.warn(Some(scope), "Low memory");
});

// Use JournalWriter to output the journal
let writer = JournalWriter::new();
// writer.write_to(&mut std::fs::File::create("events.log")?, &journal)?;
```

---

## Architecture

```
┌─────────────┐
│   Macros    │  event_info!(), event_scope!()
└──────┬──────┘
       ↓
┌─────────────┐
│   Runtime   │  Global, thread-safe facade (optional)
└──────┬──────┘
       ↓
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│   Journal   │  │   Console   │  │ LiveLogFile │
└─────────────┘  └─────────────┘  └─────────────┘

```

**Core Layer** (always available):
- **Journal** — Pure data structure
- **Scope** — Logical units of work
- **Record** — Individual events
- **Filter** — Composable criteria

**Runtime Layer** (optional):
- **runtime** — Global facade
- **console** — Dual output control
- **live_log** — Automatic, timestamped disk logging
- **Macros** — Zero-overhead convenience
- **Log levels** — Runtime event filtering

---

## Live Logging

Live logging is **enabled per file path** using
`runtime::enable_live_logging(PathBuf)`

```rust
use std::path::PathBuf;

runtime::enable_live_logging(PathBuf::from("/tmp/eventline.log")).await;

event_info!("This event will be written to the live log file automatically").await;
event_warn!("Timestamped and indented according to scope depth").await;
```

---

## Dual Output Mode

Eventline supports two output modes:

### Silent Journaling (Default)
Events are recorded but not printed:
```rust
runtime::init().await;
runtime::enable_console_output(false).await; // Default

event_info!("Silent").await; // Recorded, not printed

// Later, examine the journal
runtime::with_journal(|journal| {
    eventline::render::render_journal_tree(journal, true, None);
}).await;
```

### Dual Output (Traditional Logging Feel)
Events are both journaled AND printed to console:
```rust
runtime::init().await;
runtime::enable_console_output(true).await;
runtime::enable_console_color(true).await;

event_info!("Starting").await;  // Journaled + printed
event_warn!("Warning").await;   // Journaled + printed in yellow
event_error!("Error").await;    // Journaled + printed to stderr in red
```

**Benefits:**
- Get traditional logging behavior when you want it
- Always have structured journal for post-mortem
- Single flag to toggle: `enable_console_output(bool)`

---

## Key Concepts

### Outcomes vs Events

**Event kinds** describe *what happened*:
- `Info` — routine progress
- `Warning` — unexpected but recoverable
- `Error` — something went wrong
- `Debug` — verbose diagnostics

**Scope outcomes** describe *the result*:
- `Success` — completed normally
- `Failure` — completed with errors
- `Aborted` — interrupted by panic

This separation enables:
- Warnings during successful operations
- Errors that don't cause failure
- Clear diagnostics vs results

### Filtering at Render Time

Filtering happens when **reading** the journal, not when writing:

```rust
use eventline::journal::filter::{Filter, ScopeFilter, EventFilter};
use eventline::journal::event_kind::EventKind;
use eventline::journal::outcome::Outcome;

// Show only failed scopes
let filter = Filter::scope(ScopeFilter::Outcome(Outcome::Failure));

// Show only warnings and errors
let filter = Filter::event(
    EventFilter::Kind(EventKind::Warning)
        .or(EventFilter::Kind(EventKind::Error))
);

// Complex: failed scopes with deep nesting
let filter = Filter::scope(
    ScopeFilter::Outcome(Outcome::Failure)
        .and(ScopeFilter::MinDepth(2))
);

render_journal_tree(&journal, true, Some(&filter));
```

Benefits:
- Zero overhead when not filtering
- Complete journal always preserved
- Multiple views from same data
- Composable with AND, OR, NOT

---

## Scoped Macros

### Named Scopes

```rust
// Standard named scope
event_scope!("TaskName", {
    event_info!("Work happening").await;
}).await;

// Async variant (explicit async support)
event_scope_async!("AsyncTask", {
    event_info!("Async work").await;
    tokio::time::sleep(Duration::from_millis(10)).await;
}).await;
```

### Unnamed Scopes

```rust
// Unnamed scope - requires 'async' keyword
event_scope_unnamed!(async {
    event_info!("Anonymous work").await;
}).await;

// Async variant
event_scope_unnamed_async!({
    event_info!("Anonymous async work").await;
}).await;
```

### Try Scopes (No-Panic)

```rust
// Named scope without panicking if runtime uninitialized
try_scope!("OptionalScope", {
    event_info!("Safe logging").await;
}).await;

// Unnamed scope without panicking
try_scope_unnamed!(async {
    event_info!("Safe anonymous logging").await;
}).await;
```

### Single-Line Scoped Events

For IPC handlers and event callbacks where you want scope context without wrapping a block:

```rust
// These create a scope and log a single event
event_info_scoped!("ProfileSwitch", "Switched to profile: {}", profile_name).await;
event_warn_scoped!("CacheCheck", "Cache at {}% capacity", 95).await;
event_error_scoped!("IpcHandler", "Failed to process: {}", error).await;
event_debug_scoped!("Trigger", "Window {} focused", window_id).await;
```

---

## CLI Integration

Typical pattern for command-line tools:

```rust
use clap::Parser;
use eventline::runtime::log_level::{set_log_level, LogLevel};

#[derive(Parser)]
struct Args {
    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Suppress console output
    #[arg(short, long)]
    quiet: bool,
    
    /// Disable colored output
    #[arg(long)]
    no_color: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    
    runtime::init().await;

    if args.quiet {
        runtime::enable_console_output(false).await;
        set_log_level(LogLevel::Warning).await;
    } else {
        runtime::enable_console_output(true).await;
        runtime::enable_console_color(!args.no_color).await;
        
        if args.verbose {
            set_log_level(LogLevel::Debug).await;
        } else {
            set_log_level(LogLevel::Info).await;
        }
    }

    event_info!("Application started").await;
    
    // Your application logic here
    
    // At exit, optionally render summary with same color setting
    runtime::with_journal(|journal| {
        let use_color = !args.no_color;
        eventline::render::render_summary(journal, use_color, None);
    }).await;
}
```

---

## Advanced Usage

### Batched Logging

```rust
use eventline::journal::Journal;
use eventline::journal::outcome::Outcome;

let mut journal = Journal::new();
let mut buffer = journal.create_buffer();

let scope = buffer.enter_scope(None, Some("BatchTask"));
for item in items {
    buffer.record(Some(scope), format!("Processing {}", item));
}
buffer.exit_scope(scope, Outcome::Success);

journal.flush_buffer(buffer); // Atomic ID rebase
```

### Custom Output

```rust
use eventline::journal::writer::JournalWriter;
use std::io;

let mut file = std::fs::File::create("output.log")?;

JournalWriter::new()
    .with_bullet("→")
    .write_to_all(
        &mut [
            &mut io::stdout() as &mut dyn io::Write,
            &mut file as &mut dyn io::Write,
        ],
        &journal
    )?;
```

### Nested Scopes

```rust
event_scope!("Deployment", {
    event_info!("Starting deployment").await;
    
    event_scope!("BuildImage", {
        event_info!("Building Docker image").await;
    }).await;
    
    event_scope!("PushRegistry", {
        event_info!("Pushing to registry").await;
    }).await;
    
    event_info!("Deployment complete").await;
}).await;
```

### Environment Variable Support

Respect common conventions:

```rust
use eventline::runtime::log_level::{set_log_level, LogLevel};

#[tokio::main]
async fn main() {
    runtime::init().await;
    
    // Respect NO_COLOR
    let use_color = std::env::var("NO_COLOR").is_err();
    runtime::enable_console_color(use_color).await;
    
    // Optional: RUST_LOG compatibility
    let log_level = std::env::var("RUST_LOG")
        .map(|s| match s.to_lowercase().as_str() {
            "debug" => LogLevel::Debug,
            "info" => LogLevel::Info,
            "warn" => LogLevel::Warning,
            "error" => LogLevel::Error,
            _ => LogLevel::Info,
        })
        .unwrap_or(LogLevel::Info);
    
    set_log_level(log_level).await;
    runtime::enable_console_output(true).await;
}
```

---

## Design Principles

### Append-Only Invariant

Once written, entries are **never modified or removed**:
- Deterministic replay
- Safe concurrent reads
- Reliable audit trails

### Separation of Concerns

- **Journal** — Pure data
- **JournalWriter** — Rendering policy
- **JournalBuffer** — Batching mechanism
- **Filter** — Selection criteria
- **Runtime** — Optional global facade
- **Console** — Dual output control

### Test-Friendly

```rust
#[tokio::test]
async fn test_task() {
    runtime::init().await;
    runtime::enable_console_output(false).await; // Quiet in tests
    
    event_info!("test").await;
    
    runtime::with_journal(|journal| {
        assert_eq!(journal.records().len(), 1);
    }).await;
    
    runtime::reset().await; // Clean up
}
```

---

## Installation

```toml
[dependencies]
eventline = "0.3.12"
tokio = { version = "1", features = ["full"] }
```

Optional features:

```toml
[dependencies]
eventline = { version = "0.3.12", features = ["colour"] }
```

---

## Roadmap

- Custom formatters (JSON, binary)
- Structured data (key-value pairs)
- Zero-copy query interface
- Tag-based filtering
- systemd journal integration

---

## Philosophy

Eventline is designed to:
- Be **intuitive for humans** reading logs
- Enable **deterministic replay** of execution
- Support both **traditional logging** and **structured journaling**
- Serve as **foundation for distribution-wide logging**
- Make debugging and monitoring **pleasant**

It is **not**:
- A metrics system (use Prometheus)
- A distributed tracing backend
- A replacement for structured logging (yet)

Focuses on local, human-readable execution traces with optional runtime log filtering and dual output modes.

---

## Version 0.3.0 — Highlights

- **Async runtime support** — `init().await`, `scoped_async()`, fire-and-forget logging from async tasks
- **Live logging** — automatic timestamped file append
- **Runtime output formatting** — indentation by scope depth, bullets, colors
- **Dual output mode** — console + journal, fully async-safe
- **Scoped event macros** — single-line events with scope context (`event_info_scoped!`, etc.)
- **Removed unnecessary manual flush** — runtime manages output automatically
- **Improved CLI integration** — verbose, quiet, color flags respected
- **Internal cleanup** — simpler, safer, maintainable runtime code

---

## License

MIT
