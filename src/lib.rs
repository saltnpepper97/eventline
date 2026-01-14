//! eventline - a causality-aware execution journal.
//!
//! eventline records what happened, when it happened, and
//! in what causal context, without assuming logging,
//! tracing, or telemetry semantics.
//!
//! # Architecture
//!
//! eventline has two layers:
//!
//! ## Core Layer (Pure, Library-First)
//!
//! - [`Journal`] - Pure, append-only event store
//! - [`Scope`] - Logical units of work with outcomes
//! - [`Record`] - Individual events and scope exits
//! - [`Filter`] - Composable filtering criteria
//!
//! Use this layer when you need:
//! - Explicit ownership and control
//! - Custom storage or transmission
//! - Embedded systems or no_std environments
//! - Maximum flexibility
//!
//! ## Runtime Layer (Ergonomic, Daemon-Friendly)
//!
//! - [`runtime`] - Global, thread-safe facade
//! - Macros (`event_info!`, `scoped_eventline!`, etc.)
//!
//! Use this layer when you need:
//! - Fire-and-forget logging from anywhere
//! - No context passing or &mut borrows
//! - Integration with long-running daemons
//! - Zero-friction ergonomics
//!
//! # Quick Start
//!
//! ## Using the Runtime (Recommended for Applications)
//!
//! ```rust,no_run
//! # use eventline::{event_info, scoped_eventline};
//! # use eventline::runtime;
//! # async fn example() {
//!
//! // Initialize once at startup
//! runtime::init().await;
//!
//! // Log from anywhere
//! event_info!("Application started");
//!
//! // Create scoped contexts
//! scoped_eventline!("DatabaseMigration", {
//!     runtime::info("Applying migrations").await;
//!     runtime::info("Migration complete").await;
//! });
//!
//! // Access the journal for rendering or custom output
//! runtime::with_journal(|journal| {
//!     // Use JournalWriter for file output
//!     use eventline::journal::writer::JournalWriter;
//!     let writer = JournalWriter::new();
//!     // writer.write_to(&mut file, journal)?;
//! }).await;
//! # runtime::reset().await;
//! # }
//! ```
//!
//! ## Using the Core Journal (For Libraries)
//!
//! ```rust
//! use eventline::Journal;
//! use eventline::Outcome;
//! use eventline::JournalWriter;
//!
//! let mut journal = Journal::new();
//!
//! let scope = journal.enter_scope_unnamed(None);
//! journal.record(Some(scope), "Processing request");
//! journal.exit_scope(scope, Outcome::Success);
//!
//! // Use JournalWriter to output the journal
//! let writer = JournalWriter::new();
//! // writer.write_to(&mut std::fs::File::create("eventline.log")?, &journal)?;
//! ```

pub mod core;
pub mod journal;
pub mod macros;
pub mod render;
pub mod runtime;

pub use journal::filter::{EventFilter, Filter, ScopeFilter};
pub use journal::{Journal, JournalWriter};
pub use journal::buffer::JournalBuffer;
pub use core::Outcome;
pub use core::EventKind;
pub use core::{RecordId, ScopeId};
pub use render::{render_journal_tree, render_summary};
pub use core::{Record, RecordKind};
pub use core::{Scope, ScopeGuard};
pub use core::{Value, Fields, IntoFields};
