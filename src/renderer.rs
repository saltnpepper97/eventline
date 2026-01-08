//! Human-friendly rendering of `eventline` journals.
//!
//! Provides two views:
//! 1. `render_journal_tree` – a detailed tree with scopes and events, suitable for developers.
//! 2. `render_summary` – a concise summary of all scopes, outcomes, and durations, suitable for humans or snapshots.

use std::time::Duration;

use crate::{journal::Journal, record::RecordKind, outcome::Outcome};

/// Render the journal as a human-friendly tree.
/// 
/// Each scope is shown with its outcome and duration.
/// Events within the scope are listed with a bullet (`•`), falling back to `*` on Windows.
pub fn render_journal_tree(journal: &Journal) {
    for scope in journal.scopes() {
        render_scope(journal, scope, 0);
    }
}

fn render_scope(journal: &Journal, scope: &crate::scope::Scope, indent: usize) {
    let prefix = " ".repeat(indent * 2);

    // Find scope exit for duration and outcome
    let exit = journal.records().iter().find(|r| {
        matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id)
    });

    let outcome = if let Some(exit_rec) = exit {
        match exit_rec.kind {
            RecordKind::ScopeExit { outcome, .. } => format!("{:?}", outcome),
            _ => "?".to_string(),
        }
    } else {
        "Aborted".to_string()
    };

    let duration = if let Some(exit_rec) = exit {
        exit_rec.time.duration_since(scope.entered_at)
    } else {
        Duration::ZERO
    };

    println!(
        "{}Scope: {} ({}) [{:.3}s]",
        prefix,
        scope.id.0,
        outcome,
        duration.as_secs_f32()
    );

    // Decide bullet marker
    let bullet = if cfg!(windows) {
        "*" // fallback for Windows if Unicode looks bad
    } else {
        "•" // Unicode bullet
    };

    // Print events inside this scope
    for record in journal.records().iter().filter(|r| r.scope == Some(scope.id)) {
        if let RecordKind::Event { message } = &record.kind {
            println!("{}  {} {}", prefix, bullet, message);
        }
    }
}

/// Render a concise summary of the journal.
/// 
/// Shows:
/// - Total scopes and events
/// - Counts of each outcome (Success, Failure, Aborted)
/// - Total duration of all scopes
/// - Per-scope summary with outcome and duration
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
            .map(|r| if let RecordKind::ScopeExit { outcome, .. } = r.kind { outcome } else { Outcome::Aborted })
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
            .map(|r| r.time.duration_since(s.entered_at).as_secs_f32())
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

        let outcome = if let Some(exit_rec) = exit {
            if let RecordKind::ScopeExit { outcome, .. } = exit_rec.kind {
                outcome
            } else {
                Outcome::Aborted
            }
        } else {
            Outcome::Aborted
        };

        let duration = exit.map(|r| r.time.duration_since(scope.entered_at).as_secs_f32())
            .unwrap_or(0.0);

        println!("  Scope {} → {:?} [{:.3}s]", scope.id.0, outcome, duration);
    }
}
