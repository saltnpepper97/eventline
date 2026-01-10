# Eventline

**Eventline** is a human-friendly, append-only logging and tracing system written in Rust.  
It is designed for **systems-level programs**, daemons, and eventually Linux distributions, offering **deterministic replay**, easy inspection, and logs that make sense to both developers and regular users.

---

## Features

- Append-only journal for **scopes and events**  
- Human-readable rendering with **Unicode bullets** (`•`)  
- **Optional color output** for improved readability (success/green, failure/red, aborted/yellow)
- Summaries of scopes, outcomes, and durations  
- RAII-based scope management (`ScopeGuard`)  
- Works for daemons, interactive apps, or CLI tools 
- Temporarily store records in memory and flush in batches
- Simultaneous logging to terminal and file for live monitoring and persistent storage
- Designed to **respark joy in using computers**  

---

## Roadmap

- **Enhanced filtering** – log only certain scopes or events based on criteria (e.g., outcome, tags, or scope depth)
- **Custom formatters** – allow users to define how events are serialized for files or terminal output
- **Structured data** – support for key-value pairs and structured event metadata
- **Query interface** – programmatic querying of journal history for analysis and debugging

---

## Quick Example

```rust
use eventline::journal::Journal;
use eventline::outcome::Outcome;

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
use eventline::journal::Journal;
use eventline::outcome::Outcome;

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
use eventline::journal::{Journal, JournalWriter};
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
use eventline::journal::Journal;
use eventline::render::{render_journal_tree, render_summary};

fn main() {
    let mut journal = Journal::new();
    
    journal.scoped(None, Some("Task"), |journal, scope| {
        journal.record(Some(scope), "Processing data...");
    });
    
    // Render with colors enabled
    render_journal_tree(&journal, true);
    
    // Or render a summary with colors
    render_summary(&journal, true);
    
    // Pass false to disable colors for file output or non-color terminals
    render_journal_tree(&journal, false);
}
```

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

This separation allows the same journal to be:
- Rendered to terminal with colors
- Written to files in plain text
- Serialized to JSON/binary formats
- Streamed to remote logging systems

### Outcomes vs Events

Event kinds (`Info`, `Warning`, `Error`) describe **what happened**, while scope outcomes (`Success`, `Failure`, `Aborted`) describe **the result**. This distinction allows:
- Warnings during successful operations
- Successful completion despite errors encountered
- Clear separation between diagnostics and results

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
