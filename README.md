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
- **Unified color control** — consistent ANSI colors across console and renderer
- **Flexible filtering** — by outcome, depth, duration, event kind, message content
- **Human-readable output** — Unicode bullets, optional color coding
- **High-throughput batching** — `JournalBuffer` for batch writes
- **Dual-layer API** — pure core + optional runtime facade
- **Deterministic replay** — safe concurrent reads, reliable audit trails

---

## Quick Start

### Runtime API (Fire-and-Forget)

For applications and daemons:

```rust
use eventline::runtime;
use eventline::runtime::log_level::{set_log_level, LogLevel};
use eventline::{event_info, event_warn, event_error, event_debug};

fn main() {
    runtime::init();
    
    // Enable dual output: journal + console
    runtime::enable_console_output(true);
    runtime::enable_console_color(true);  // Optional ANSI colors
    
    // Only record warnings and errors
    set_log_level(LogLevel::Warning);

    event_info!("This will NOT be logged");
    event_warn!("Cache approaching limit");          // Journaled + printed in yellow
    event_error!("Database connection failed");      // Journaled + printed in red
    event_debug!("Verbose debug info");              // Filtered out
       
    event_scope!("RequestHandler", {
        event_info!("Processing request");           // Filtered out
        event_warn!("Retry attempt failed");         // Journaled + printed
    });
    
    // Save journal for later analysis
    runtime::with_journal(|journal| {
        journal.write_to_file("events.log").unwrap();
    });
}
```

### Core Journal API (Explicit Control)

For libraries or embedded systems:

```rust
use eventline::{Journal, Outcome};

let mut journal = Journal::new();

journal.scoped(None, Some("Task"), |journal, scope| {
    journal.record(Some(scope), "Starting task");
    journal.warn(Some(scope), "Low memory");
});

journal.write_to_file("events.log").unwrap();
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
┌─────────────┐  ┌─────────────┐
│   Journal   │  │   Console   │
└─────────────┘  └─────────────┘
```

**Core Layer** (always available):
- **Journal** — Pure data structure
- **Scope** — Logical units of work
- **Record** — Individual events
- **Filter** — Composable criteria

**Runtime Layer** (optional):
- **runtime** — Global facade
- **console** — Dual output control
- **Macros** — Zero-overhead convenience
- **Log levels** — Runtime event filtering

---

## Dual Output Mode

Eventline supports two output modes:

### Silent Journaling (Default)
Events are recorded but not printed:
```rust
runtime::init();
runtime::enable_console_output(false); // Default

event_info!("Silent"); // Recorded, not printed

// Later, examine the journal
runtime::with_journal(|journal| {
    eventline::renderer::render_journal_tree(journal, true, None);
});
```

### Dual Output (Traditional Logging Feel)
Events are both journaled AND printed to console:
```rust
runtime::init();
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
use eventline::{Filter, ScopeFilter, EventFilter, EventKind, Outcome};

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

## CLI Integration

Typical pattern for command-line tools:

```rust
use clap::Parser;

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

fn main() {
    let args = Args::parse();
    
    runtime::init();

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
        eventline::renderer::render_summary(journal, use_color, None);
    });
}
```

---

## Advanced Usage

### Batched Logging

```rust
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
runtime::scoped(Some("Deployment"), || {
    event_info!("Starting deployment");
    
    runtime::scoped(Some("BuildImage"), || {
        event_info!("Building Docker image");
    });
    
    runtime::scoped(Some("PushRegistry"), || {
        event_info!("Pushing to registry");
    });
    
    event_info!("Deployment complete");
});
```

### Environment Variable Support

Respect common conventions:

```rust
fn main() {
    runtime::init();
    
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
#[test]
fn test_task() {
    runtime::init();
    runtime::enable_console_output(false); // Quiet in tests
    
    event_info!("test");
    
    runtime::with_journal(|journal| {
        assert_eq!(journal.records().len(), 1);
    });
    
    runtime::reset(); // Clean up
}
```

---

## Installation

```toml
[dependencies]
eventline = "0.2.12"
```

Optional features:

```toml
[dependencies]
eventline = { version = "0.2.12", features = ["colour"] }
```

---

## Roadmap

- [ ] Custom formatters (JSON, binary)
- [ ] Structured data (key-value pairs)
- [ ] Zero-copy query interface
- [ ] Tag-based filtering
- [ ] Async runtime support
- [ ] systemd journal integration

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

## What's New in 0.2.1

- **Dual output mode** — Optional real-time console printing alongside journaling
- **Unified color control** — Consistent ANSI colors across console and renderer
- **Runtime color toggle** — `enable_console_color(bool)` for dynamic control
- **Better CLI integration** — Standard patterns for `--verbose`, `--quiet`, `--no-color`
- **NO_COLOR support** — Respects environment variable conventions

---

## License

MIT
