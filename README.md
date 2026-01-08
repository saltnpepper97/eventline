# Eventline

**Eventline** is a human-friendly, append-only logging and tracing system written in Rust.  
It is designed for **systems-level programs**, daemons, and eventually Linux distributions, offering **deterministic replay**, easy inspection, and logs that make sense to both developers and regular users.

---

## Features

- Append-only journal for **scopes and events**  
- Human-readable rendering with **Unicode bullets** (`•`)  
- Summaries of scopes, outcomes, and durations  
- RAII-based scope management (`ScopeGuard`)  
- Works for daemons, interactive apps, or CLI tools  
- Designed to **respark joy in using computers**  

---

## Roadmap
- **Buffered logging** – temporarily store records in memory and flush in batches to improve performance.
- **Dual output** – simultaneous logging to terminal and file for live monitoring and persistent storage.
- **Enhanced filtering** – log only certain scopes or events based on criteria (e.g., outcome, tags, or scope depth).
- **Custom formatters** – allow users to define how events are serialized for files or terminal output.

---

## Quick Example

```rust
use eventline::{Journal, Outcome, ScopeGuard, render_journal_tree, render_summary};

fn main() {
    let mut journal = Journal::new();

    // Enter a scope
    let scope_id = journal.enter_scope(None);
    let mut guard = ScopeGuard::new(&mut journal, scope_id);

    // Record some events
    journal.record(Some(scope_id), "Starting Stasis daemon...");
    journal.record(Some(scope_id), "Loading profiles...");

    // Exit the scope
    guard.exit(Outcome::Success);

    // Render tree and summary
    render_journal_tree(&journal);
    render_summary(&journal);
}
```


## Philosophy
Eventline is not just another logging library. It is meant to:
- Be intuitive for humans reading logs
- Provide full traceability of application behavior
- Enable replay and inspection without mutation
- Serve as a foundation for logging across multiple apps in a Linux distro
