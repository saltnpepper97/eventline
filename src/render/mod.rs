//! Human-friendly rendering of `eventline` journals with optional colors and filtering.
//!
//! Color is enabled via a `color: bool` flag. No external crates are used.
//! Filtering allows selective rendering of scopes and events based on criteria.

pub mod colour;
use colour::{RESET, RED, YELLOW, GREEN, BLUE};

use crate::{
    journal::Journal,
    journal::record::RecordKind,
    journal::outcome::Outcome,
    journal::event_kind::EventKind,
    journal::filter::Filter,
};

/// Render the journal as a human-friendly tree with optional color and filtering.
///
/// # Arguments
/// * `journal` - The journal to render
/// * `color` - Enable ANSI color codes
/// * `filter` - Optional filter to apply. If `None`, all scopes and events are rendered.
///
/// # Example
/// ```
/// use eventline::journal::Journal;
/// use eventline::journal::filter::{Filter, ScopeFilter};
/// use eventline::journal::outcome::Outcome;
///
/// let mut journal = Journal::new();
/// let scope = journal.enter_scope_unnamed(None);
/// journal.record(Some(scope), "test event");
/// journal.exit_scope(scope, Outcome::Success);
///
/// // Render everything
/// eventline::render::render_journal_tree(&journal, true, None);
///
/// // Render only failed scopes
/// let filter = Filter::scope(ScopeFilter::Outcome(Outcome::Failure));
/// eventline::render::render_journal_tree(&journal, true, Some(&filter));
/// ```
pub fn render_journal_tree(journal: &Journal, color: bool, filter: Option<&Filter>) {
    let default_filter = Filter::default();
    let filter = filter.unwrap_or(&default_filter);

    for scope in journal.scopes() {
        // Skip scopes that don't match the filter
        if !filter.matches_scope(scope, journal) {
            continue;
        }

        render_scope(journal, scope, 0, color, filter);
    }
}

/// Render a single scope with indentation, optional color, and filtering.
fn render_scope(
    journal: &Journal,
    scope: &crate::scope::Scope,
    indent: usize,
    color: bool,
    filter: &Filter,
) {
    let prefix = " ".repeat(indent * 2);

    // Find the scope exit record (if any) to determine outcome and duration
    let exit = journal.records().iter().find(|r| {
        matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id)
    });

    // Determine outcome of the scope
    let outcome = exit
        .and_then(|r| {
            if let RecordKind::ScopeExit { outcome, .. } = r.kind {
                Some(outcome)
            } else {
                None
            }
        })
        .unwrap_or(Outcome::Aborted);

    // Compute duration in seconds
    let duration_s = exit
        .map(|r| (r.time.saturating_sub(scope.entered_at)) as f64 / 1000.0)
        .unwrap_or(0.0);

    let name = scope.name.as_deref().unwrap_or("unnamed");

    // Colorize scope outcome
    let outcome_str = match outcome {
        Outcome::Success => if color { format!("{}Success{}", GREEN, RESET) } else { "Success".into() },
        Outcome::Failure => if color { format!("{}Failure{}", RED, RESET) } else { "Failure".into() },
        Outcome::Aborted => if color { format!("{}Aborted{}", YELLOW, RESET) } else { "Aborted".into() },
    };

    println!(
        "{}Scope: {} (ID: {}) [{}] [{:.3}s]",
        prefix,
        name,
        scope.id.0,
        outcome_str,
        duration_s
    );

    // Platform-safe bullet
    let bullet = if cfg!(windows) { "*" } else { "•" };

    // Render events belonging to this scope, applying event filter
    for record in journal.records().iter().filter(|r| r.scope == Some(scope.id)) {
        // Skip events that don't match the event filter
        if !filter.matches_event(record) {
            continue;
        }

        if let RecordKind::Event { kind, message } = &record.kind {
            // Determine event label and optional color
            let (label, color_code) = match kind {
                EventKind::Info => ("", None),
                EventKind::Debug => ("debug: ", Some(BLUE)),
                EventKind::Warning => ("warning: ", Some(YELLOW)),
                EventKind::Error => ("error: ", Some(RED)),
            };

            // Render the event line
            if color {
                let colored_label = color_code.map_or(label.to_string(), |c| format!("{}{}{}", c, label, RESET));
                println!("{}  {} {}", prefix, bullet, format!("{}{}", colored_label, message));
            } else {
                println!("{}  {} {}{}", prefix, bullet, label, message);
            }

            // Render arrow pointing right toward the event text for warnings/errors
            if matches!(kind, EventKind::Warning | EventKind::Error) {
                println!("{}    ↳ {}", prefix, message);
            }
        }
    }
}

/// Render a concise summary of the journal with optional color and filtering.
///
/// Displays:
/// - Total number of scopes and events (after filtering)
/// - Count of each scope outcome
/// - Total cumulative scope duration
/// - Per-scope summary including name, ID, outcome, and duration
///
/// # Arguments
/// * `journal` - The journal to summarize
/// * `color` - Enable ANSI color codes
/// * `filter` - Optional filter to apply. If `None`, all scopes and events are included.
pub fn render_summary(journal: &Journal, color: bool, filter: Option<&Filter>) {
    let default_filter = Filter::default();
    let filter = filter.unwrap_or(&default_filter);

    // Filter scopes
    let filtered_scopes: Vec<_> = journal
        .scopes()
        .iter()
        .filter(|s| filter.matches_scope(s, journal))
        .collect();

    let total_scopes = filtered_scopes.len();

    // Filter events
    let total_events = journal
        .records()
        .iter()
        .filter(|r| matches!(r.kind, RecordKind::Event { .. }))
        .filter(|r| filter.matches_event(r))
        .count();

    let mut success = 0;
    let mut failure = 0;
    let mut aborted = 0;

    for scope in &filtered_scopes {
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

    let total_duration: f32 = filtered_scopes.iter().map(|s| {
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
    for scope in &filtered_scopes {
        let exit = journal.records().iter()
            .find(|r| matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id));

        let outcome = exit.and_then(|r| if let RecordKind::ScopeExit { outcome, .. } = r.kind { Some(outcome) } else { None })
            .unwrap_or(Outcome::Aborted);

        let duration_s = exit.map(|r| (r.time.saturating_sub(scope.entered_at)) as f32 / 1000.0).unwrap_or(0.0);
        let name = scope.name.as_deref().unwrap_or("unnamed");

        // Colorize outcome
        let outcome_str = match outcome {
            Outcome::Success => if color { format!("{}Success{}", GREEN, RESET) } else { "Success".into() },
            Outcome::Failure => if color { format!("{}Failure{}", RED, RESET) } else { "Failure".into() },
            Outcome::Aborted => if color { format!("{}Aborted{}", YELLOW, RESET) } else { "Aborted".into() },
        };

        println!("  Scope: {} (ID: {}) → {} [{:.3}s]", name, scope.id.0, outcome_str, duration_s);
    }
}
