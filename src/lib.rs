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
//! - [`Scope`](scope::Scope) - Logical units of work with outcomes
//! - [`Record`](record::Record) - Individual events and scope exits
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
//! - Macros ([`event_info!`], [`event_scope!`], etc.)
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
//! ```rust
//! # #[doc(hidden)] use eventline::{event_info, event_scope};
//! # use eventline::runtime;
//!
//! // Initialize once at startup
//! runtime::init();
//!
//! // Log from anywhere
//! event_info!("Application started");
//!
//! // Create scoped contexts
//! event_scope!("DatabaseMigration", {
//!     event_info!("Applying migrations");
//!     event_info!("Migration complete");
//! });
//!
//! // Write to file via journal access
//! runtime::with_journal(|journal| {
//!     journal.write_to_file("eventline.log").unwrap();
//! });
//! # runtime::reset();
//! ```
//!
//! ## Using the Core Journal (For Libraries)
//!
//! ```rust
//! use eventline::Journal;
//! use eventline::outcome::Outcome;
//!
//! let mut journal = Journal::new();
//!
//! let scope = journal.enter_scope_unnamed(None);
//! journal.record(Some(scope), "Processing request");
//! journal.exit_scope(scope, Outcome::Success);
//!
//! journal.write_to_file("eventline.log").unwrap();
//! ```

pub mod journal;
pub mod macros;
pub mod render;
pub mod runtime;
pub mod scope;

pub use journal::event_kind::EventKind;
pub use journal::filter::{EventFilter, Filter, ScopeFilter};
pub use journal::{Journal, JournalWriter};
pub use journal::outcome::Outcome;
pub use journal::id::{RecordId, ScopeId};
pub use render::{render_journal_tree, render_summary};
pub use scope::scope_guard::ScopeGuard;
