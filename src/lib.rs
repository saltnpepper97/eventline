//! eventline - a causality-aware execution journal.
//!
//! eventline records what happened, when it happened, and
//! in what causal context, without assuming logging,,
//! tracing, or telemetry semantics.

pub mod colour;
pub mod event_kind;
pub mod filter;
pub mod id;
pub mod journal;
pub mod outcome;
pub mod record;
pub mod renderer;
pub mod scope;
pub mod scope_guard;

pub use event_kind::EventKind;
pub use filter::{Filter, EventFilter, ScopeFilter};
pub use outcome::Outcome;
pub use scope_guard::ScopeGuard;
pub use journal::{Journal, JournalWriter};
pub use renderer::{render_journal_tree, render_summary};
