//! eventline - a causality-aware execution journal.
//!
//! eventline records what happened, when it happened, and
//! in what causal context, without assuming logging,,
//! tracing, or telemetry semantics.

pub mod id;
pub mod journal;
pub mod record;
pub mod scope;
