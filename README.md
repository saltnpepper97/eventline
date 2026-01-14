# Eventline

**Causality-aware execution journal for systems-level programs.**

Eventline records *what happened*, *when it happened*, and *in what causal context* — without assuming logging, tracing, or telemetry semantics. Built for daemons, CLI tools, and eventually Linux distributions.

---

## Features

- **Append-only journal** — never mutates or removes records
- **Scoped execution** — track outcomes and durations
- **Structured data** — attach key-value fields to events for rich context
- **Runtime log levels** — filter events globally (Debug, Info, Warning, Error)
- **Dual output mode** — journal + optional real-time console printing
- **Live logging** — automatic, timestamped append to disk
- **Unified color control** — consistent ANSI colors across console and renderer
- **Flexible filtering** — by outcome, depth, duration, event kind, message content
- **High-throughput batching** — `JournalBuffer` for batch writes
- **Async runtime support** — fire-and-forget logging and async scopes
- **Deterministic replay** — safe concurrent reads, reliable audit trails

---

## Why Eventline?

Eventline is not "better logging" - It's **structured execution history.**

- Events are **append-only** and never rewritten
- Work is grouped into scopes with outcomes and durations
- **Event != results** (warnings can happen in successful work)
- Journals can be **replayed deterministically**
- Console output is optional - Structure is always preserved

You get:
- human-readable output
- A complete execution record for post-mortem analysis

---

## Quick Start

### Runtime API (Fire-and-Forget Async)

```rust
use eventline::runtime;
use eventline::{event_info, event_info_scoped, scoped_eventline};

#[tokio::main]
async fn main() {
    // Initialize runtime
    runtime::init().await;

    // Enable console output
    runtime::enable_console_output(true);
    runtime::enable_console_color(true);

    // Fire-and-forget logging
    event_info!("Application started");

    // Single scoped event
    event_info_scoped!("DatabaseMigration", "Applying schema changes");

    // Multiple events in a scope
    scoped_eventline!("Startup", {
        runtime::info("Loading configuration").await;
        runtime::info("Connecting to database").await;
        runtime::info("Server ready").await;
    });
}
```

### Core Journal API (Explicit Control)

For libraries or embedded systems:

```rust
use eventline::Journal;
use eventline::Outcome;
use eventline::JournalWriter;

let mut journal = Journal::new();

let scope = journal.enter_scope(None, Some("Task"));
journal.record(Some(scope), "Starting task");
journal.exit_scope(scope, Outcome::Success);

// Use JournalWriter to output the journal
let writer = JournalWriter::new();
// writer.write_to(&mut std::fs::File::create("events.log")?, &journal)?;
```

### Structured Event Example

```rust
use eventline::runtime;
use eventline::{fields, event_info_fields};

#[tokio::main]
async fn main() {
    runtime::init().await;

    // Build structured fields using the helper macro
    let f = fields!({
        "user_id" => 12345,
        "action" => "login",
        "success" => true
    });

    // Fire-and-forget info event with structured fields
    event_info_fields!("User login attempt", f);

    runtime::reset().await;
}
```

---

## API Overview

### Event Macros (Fire-and-Forget)

**Unscoped events:**
```rust
event_info!("message");
event_debug!("message");
event_warn!("message");
event_error!("message");
```

**Single scoped events:**
```rust
event_info_scoped!("ScopeName", "message");
event_debug_scoped!("ScopeName", "message");
event_warn_scoped!("ScopeName", "message");
event_error_scoped!("ScopeName", "message");
```

**Structured data:**
```rust
let f = fields!({ "key" => value });
event_info_fields!("message", f);
event_debug_fields!("message", f);
event_warn_fields!("message", f);
event_error_fields!("message", f);
```

**Complex scopes:**
```rust
scoped_eventline!("ScopeName", {
    runtime::info("first event").await;
    runtime::debug("second event").await;
});

scoped_eventline_fields!("ScopeName", {
    let f = fields!({ "key" => value });
    runtime::event::info_fields("event", f).await;
});
```

---

### Console Output (Simple Format)
Clean, minimal output optimized for watching logs during development:
```text
Starting server
Binding to 0.0.0.0:8080
Server started successfully
warning: cache at 95% capacity
```

### Live Log File (Canonical Format)
Structured output with scope headers, timestamps, and aligned formatting:
```text
[19:04:12.381] Scope startup (id=1) → Success (142ms)
  • info      Starting server
  • info      Binding to 0.0.0.0:8080
  • info      Server started successfully
  • warning   cache at 95% capacity
```

---

## Architecture

```
┌─────────────┐
│   Macros    │  event_info!(), scoped_eventline!()
└──────┬──────┘
       ↓
┌─────────────┐
│   Runtime   │  Global, thread-safe facade (optional)
└──────┬──────┘
       ↓
┌─────────────┐   ┌─────────────┐   ┌─────────────┐
│   Journal   │   │   Console   │   │ LiveLogFile │
└──────┬──────┘   └──────┬──────┘   └──────┬──────┘
       │                 │                 │
       └─────────────────┴─────────────────┘
                         ↓
              ┌─────────────────────┐
              │  Canonical Format   │  Single rendering source
              └─────────────────────┘
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

Live logging is **enabled per file path** using `runtime::enable_live_logging(PathBuf)`:

```rust
use std::path::PathBuf;

runtime::enable_live_logging(PathBuf::from("/tmp/eventline.log"));

event_info!("This event will be written to the live log file automatically");
event_warn!("Timestamped and indented according to scope depth");
```

---

## Dual Output Mode

Eventline supports two output modes:

### Silent Journaling (Default)
Events are recorded but not printed:
```rust
runtime::init().await;
runtime::enable_console_output(false); // Default

event_info!("Silent"); // Recorded, not printed

// Later, examine the journal
runtime::with_journal(|journal| {
    eventline::render::render_journal_tree(journal, true, None);
}).await;
```

### Dual Output (Traditional Logging Feel)
Events are both journaled AND printed to console:
```rust
runtime::init().await;
runtime::enable_console_output(true);
runtime::enable_console_color(true);

event_info!("Starting");  // Journaled + printed
event_warn!("Warning");   // Journaled + printed in yellow
event_error!("Error");    // Journaled + printed to stderr in red
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
use eventline::Filter;
use eventline::ScopeFilter;
use eventline::Outcome;

let filter = Filter::scope(ScopeFilter::Outcome(Outcome::Failure));
render_journal_tree(&journal, true, Some(&filter));
```

Benefits:
- Zero overhead when not filtering
- Complete journal always preserved
- Multiple views from same data

---

## Advanced Usage

### Batched Logging

```rust
use eventline::Journal;
use eventline::Outcome;

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
use eventline::JournalWriter;
use std::io;

let mut file = std::fs::File::create("output.log")?;

// Customize canonical format
JournalWriter::new()
    .with_color(false)           // Disable colors
    .with_timestamps(true)       // Include timestamps
    .with_bullet("→")            // Custom bullet
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
scoped_eventline!("Deployment", {
    runtime::info("Starting deployment").await;
    
    scoped_eventline!("BuildImage", {
        runtime::info("Building Docker image").await;
    });
    
    scoped_eventline!("PushRegistry", {
        runtime::info("Pushing to registry").await;
    });
    
    runtime::info("Deployment complete").await;
});
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
    runtime::enable_console_color(use_color);
    
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
    
    set_log_level(log_level);
    runtime::enable_console_output(true);
}
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
        runtime::enable_console_output(false);
        set_log_level(LogLevel::Warning);
    } else {
        runtime::enable_console_output(true);
        runtime::enable_console_color(!args.no_color);
        
        if args.verbose {
            set_log_level(LogLevel::Debug);
        } else {
            set_log_level(LogLevel::Info);
        }
    }

    event_info!("Application started");
    
    // Your application logic here
    
    // At exit, optionally render summary with same color setting
    runtime::with_journal(|journal| {
        let use_color = !args.no_color;
        eventline::render::render_summary(journal, use_color, None, false);
    }).await;
}
```

---

## Design Principles

- **Append-only by default** - safe, auditable, deterministic
- **Separation of concerns** - data != rendering != runtime
- **Human-first output** - readable without tooling
- **Optional global state** - usable in libraries
- **Async-safe** - fire-and-forget from any task

### Test-Friendly

```rust
#[tokio::test]
async fn test_task() {
    runtime::init().await;
    runtime::enable_console_output(false); // Quiet in tests
    
    event_info!("test");
    
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
eventline = "0.4.2"
tokio = { version = "1", features = ["full"] }
```

Optional features:

```toml
[dependencies]
eventline = { version = "0.4.2", features = ["colour"] }
```

---

## Roadmap

- Custom formatters (JSON, binary)
- Zero-copy query interface
- Tag-based filtering
- systemd journal integration

---

## License

MIT
