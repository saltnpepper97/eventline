# Eventline

**Eventline** is a human-friendly, append-only logging and tracing system written in Rust.  
It is designed for **systems-level programs**, daemons, and eventually Linux distributions, offering **deterministic replay**, easy inspection, and logs that make sense to both developers and regular users.

Eventline is ideal when you want logs that explain *what happened*,
*why it happened*, and *how long it took* — without parsing walls of text.

---

## Features

- Append-only journal for **scopes and events**  
- Human-readable rendering with **Unicode bullets** (`•`)  
- **Optional color output** for improved readability (success/green, failure/red, aborted/yellow)
- Summaries of scopes, outcomes, and durations  
- **Enhanced filtering** – selectively render scopes and events based on outcome, depth, duration, event kind, message content, and more
- RAII-based scope management (`ScopeGuard`)  
- Works for daemons, interactive apps, or CLI tools 
- Temporarily store records in memory and flush in batches
- Simultaneous logging to terminal and file for live monitoring and persistent storage
- Designed to **respark joy in using computers**  

---

## Roadmap

- **Custom formatters** – allow users to define how events are serialized for files or terminal output
- **Structured data** – support for key-value pairs and structured event metadata
- **Query interface** – zero-copy, in-memory querying of journal history for analysis and debugging
- **Tag-based filtering** – add tags to scopes/events for more flexible filtering

---

## Project Status

Eventline is **actively developed and usable today**.

- The core journal model and append-only guarantees are **stable**
- Renderer output and formatting APIs may evolve before 1.0
- Feedback and contributions welcome

---

## Quick Example

```rust
use eventline::{Journal, Outcome};

fn main() {
    let mut journal = Journal::new();
    
    // Scoped block with automatic success/failure tracking
    journal.scoped(None, Some("Startup"), |journal, scope| {
        journal.record(Some(scope), "Application starting");
        journal.record(Some(scope), "Loading configuration");
        journal.record(Some(scope), "Initializing modules");
    });
    
    // Manual scope management for more control
    let task_scope = journal.enter_scope(None, Some("BackgroundTask"));
    journal.record(Some(task_scope), "Performing background task");
    journal.record(Some(task_scope), "Task completed successfully");
    journal.exit_scope(task_scope, Outcome::Success);
    
    // Write to file
    journal.write_to_file("application.log").unwrap();
}
```

### Using Buffered Logging

For high-throughput scenarios, use `JournalBuffer` to batch writes:

```rust
use eventline::{Journal, Outcome};

fn main() {
    let mut journal = Journal::new();
    
    // Create a buffer for batched logging
    let mut buffer = journal.create_buffer();
    
    let scope = buffer.enter_scope(None, Some("Processing"));
    buffer.record(Some(scope), "Processing item 1");
    buffer.record(Some(scope), "Processing item 2");
    buffer.exit_scope(scope, Outcome::Success);
    
    // Flush buffer to journal (IDs are rebased atomically)
    journal.flush_buffer(buffer);
    
    journal.write_to_file("batch.log").unwrap();
}
```

### Custom Output with JournalWriter

```rust
use eventline::{Journal, JournalWriter};
use std::io;

fn main() {
    let mut journal = Journal::new();
    
    journal.scoped(None, Some("Task"), |journal, scope| {
        journal.record(Some(scope), "Doing work...");
    });
    
    // Write to multiple destinations simultaneously
    let mut file = std::fs::File::create("output.log").unwrap();
    
    JournalWriter::new()
        .with_bullet("→")
        .write_to_all(
            &mut [
                &mut io::stdout() as &mut dyn io::Write,
                &mut file as &mut dyn io::Write
            ],
            &journal
        )
        .unwrap();
}
```

### Rendering with Color

```rust
use eventline::{Journal, render_journal_tree, render_summary};

fn main() {
    let mut journal = Journal::new();
    
    journal.scoped(None, Some("Task"), |journal, scope| {
        journal.record(Some(scope), "Processing data...");
    });
    
    // Render with colors enabled
    render_journal_tree(&journal, true, None);
    
    // Or render a summary with colors
    render_summary(&journal, true, None);
    
    // Pass false to disable colors for file output or non-color terminals
    render_journal_tree(&journal, false, None);
}
```

### Filtering Logs

Filter scopes and events based on various criteria:

```rust
use eventline::{Journal, Filter, ScopeFilter, EventFilter, Outcome, EventKind};
use eventline::render_journal_tree;

fn main() {
    let mut journal = Journal::new();
    
    // Create some scopes with different outcomes
    let scope1 = journal.enter_scope(None, Some("database-query"));
    journal.record(Some(scope1), "Querying users table");
    journal.warn(Some(scope1), "Slow query detected");
    journal.exit_scope(scope1, Outcome::Success);
    
    let scope2 = journal.enter_scope(None, Some("api-request"));
    journal.error(Some(scope2), "Connection timeout");
    journal.exit_scope(scope2, Outcome::Failure);
    
    // Filter 1: Show only failed scopes
    let filter = Filter::scope(ScopeFilter::Outcome(Outcome::Failure));
    render_journal_tree(&journal, true, Some(&filter));
    
    // Filter 2: Show only warnings and errors
    let filter = Filter::event(
        EventFilter::Kind(EventKind::Warning)
            .or(EventFilter::Kind(EventKind::Error))
    );
    render_journal_tree(&journal, true, Some(&filter));
    
    // Filter 3: Complex filter - failed scopes with errors
    let filter = Filter::new(
        ScopeFilter::Outcome(Outcome::Failure),
        EventFilter::Kind(EventKind::Error)
    );
    render_journal_tree(&journal, true, Some(&filter));
}
```

See [FILTERING_GUIDE.md](FILTERING_GUIDE.md) for comprehensive filtering examples and use cases.

---

## Philosophy

Eventline is not just another logging library. It is meant to:

- Be **intuitive for humans** reading logs
- Provide **full traceability** of application behavior
- Enable **replay and inspection** without mutation
- Serve as a foundation for logging across multiple apps in a Linux distro
- Make debugging and monitoring a **pleasant experience**

---

## Design Principles

### Append-Only Invariant

Once written, journal entries are **never modified or removed**. This guarantees:
- Deterministic replay of program execution
- Safe concurrent reads
- Reliable audit trails

### Separation of Concerns

- **Journal**: Pure data structure for scopes and events
- **JournalWriter**: Rendering policy (output format, destinations)
- **JournalBuffer**: Batching mechanism for high-throughput scenarios
- **Filter**: Criteria-based selection of scopes and events for rendering

This separation allows the same journal to be:
- Rendered to terminal with colors
- Written to files in plain text
- Filtered for specific outcomes, event kinds, or other criteria
- Serialized to JSON/binary formats
- Streamed to remote logging systems

### Outcomes vs Events

Event kinds (`Info`, `Warning`, `Error`) describe **what happened**, while scope outcomes (`Success`, `Failure`, `Aborted`) describe **the result**. This distinction allows:
- Warnings during successful operations
- Successful completion despite errors encountered
- Clear separation between diagnostics and results

### Filtering Philosophy

Filtering is applied at **render time**, not at journal construction. This means:
- Zero overhead when not filtering
- The complete journal is always preserved
- Different views can be created from the same data
- Filters are composable using logical operators (AND, OR, NOT)

---

## Non-Goals

Eventline is not:
- A metrics system (use Prometheus, etc.)
- A distributed tracing backend
- A replacement for structured logging frameworks (yet)

It focuses on *local, human-readable execution traces*.

---

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
eventline = "0.1"
```

---

## License

MIT

---
