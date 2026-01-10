//! Human-friendly rendering of `eventline` journals.
//!
//! This module provides simple, opinionated renderers intended for
//! direct human consumption (stdout, debug logs, snapshots).
//!
//! It deliberately avoids colors, filtering, or output configuration.
//! For structured or configurable output, use [`JournalWriter`] instead.
//!
//! Provided views:
//! 1. [`render_journal_tree`] – hierarchical view of scopes and events
//! 2. [`render_summary`] – concise session overview

use crate::{
    journal::Journal,
    record::RecordKind,
    outcome::Outcome,
    event_kind::EventKind,
};

/// Render the journal as a human-friendly tree.
///
/// Each scope is displayed with:
/// - Scope name (or `"unnamed"`)
/// - Scope ID
/// - Outcome
/// - Duration
///
/// Events belonging to the scope are listed underneath with a bullet.
/// Event kinds are rendered textually (e.g. `error:`, `warning:`) to make
/// failures immediately visible without relying on symbols or colors.
///
/// This renderer is intended for developer-facing diagnostics and debugging.
pub fn render_journal_tree(journal: &Journal) {
    for scope in journal.scopes() {
        render_scope(journal, scope, 0);
    }
}

fn render_scope(journal: &Journal, scope: &crate::scope::Scope, indent: usize) {
    let prefix = " ".repeat(indent * 2);

    // Find the scope exit record (if any) to determine outcome and duration
    let exit = journal.records().iter().find(|r| {
        matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id)
    });

    let outcome = exit
        .and_then(|r| {
            if let RecordKind::ScopeExit { outcome, .. } = r.kind {
                Some(outcome)
            } else {
                None
            }
        })
        .unwrap_or(Outcome::Aborted);

    let duration_s = exit
        .map(|r| (r.time.saturating_sub(scope.entered_at)) as f64 / 1000.0)
        .unwrap_or(0.0);

    let name = scope.name.as_deref().unwrap_or("unnamed");

    println!(
        "{}Scope: {} (ID: {}) [{:?}] [{:.3}s]",
        prefix,
        name,
        scope.id.0,
        outcome,
        duration_s
    );

    // Platform-safe bullet
    let bullet = if cfg!(windows) { "*" } else { "•" };

    // Render events belonging to this scope
    for record in journal.records().iter().filter(|r| r.scope == Some(scope.id)) {
        if let RecordKind::Event { kind, message } = &record.kind {
            let label = match kind {
                EventKind::Info => "",
                EventKind::Debug => "debug: ",
                EventKind::Warning => "warning: ",
                EventKind::Error => "error: ",
            };

            println!(
                "{}  {} {}{}",
                prefix,
                bullet,
                label,
                message
            );
        }
    }
}

/// Render a concise summary of the journal.
///
/// This view is intended for snapshots, CI output, or post-run summaries.
///
/// Displays:
/// - Total number of scopes and events
/// - Count of each scope outcome
/// - Total cumulative scope duration
/// - Per-scope summary including name, ID, outcome, and duration
pub fn render_summary(journal: &Journal) {
    let total_scopes = journal.scopes().len();
    let total_events = journal.records().iter()
        .filter(|r| matches!(r.kind, RecordKind::Event { .. }))
        .count();

    let mut success = 0;
    let mut failure = 0;
    let mut aborted = 0;

    for scope in journal.scopes() {
        let outcome = journal.records().iter()
            .find(|r| matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id))
            .map(|r| {
                if let RecordKind::ScopeExit { outcome, .. } = r.kind {
                    outcome
                } else {
                    Outcome::Aborted
                }
            })
            .unwrap_or(Outcome::Aborted);

        match outcome {
            Outcome::Success => success += 1,
            Outcome::Failure => failure += 1,
            Outcome::Aborted => aborted += 1,
        }
    }

    let total_duration: f32 = journal.scopes().iter().map(|s| {
        journal.records().iter()
            .find(|r| matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(s.id))
            .map(|r| (r.time.saturating_sub(s.entered_at)) as f32 / 1000.0)
            .unwrap_or(0.0)
    }).sum();

    println!("Session summary: {} scopes, {} events", total_scopes, total_events);
    println!("  Successful scopes: {}", success);
    println!("  Failed scopes: {}", failure);
    println!("  Aborted scopes: {}", aborted);
    println!("  Total duration: {:.3}s", total_duration);

    println!("\nPer-scope summary:");
    for scope in journal.scopes() {
        let exit = journal.records().iter()
            .find(|r| matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id));

        let outcome = exit
            .and_then(|r| {
                if let RecordKind::ScopeExit { outcome, .. } = r.kind {
                    Some(outcome)
                } else {
                    None
                }
            })
            .unwrap_or(Outcome::Aborted);

        let duration_s = exit
            .map(|r| (r.time.saturating_sub(scope.entered_at)) as f32 / 1000.0)
            .unwrap_or(0.0);

        let name = scope.name.as_deref().unwrap_or("unnamed");

        println!(
            "  Scope: {} (ID: {}) → {:?} [{:.3}s]",
            name,
            scope.id.0,
            outcome,
            duration_s
        );
    }
}
