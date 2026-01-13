//! Human-friendly rendering of `eventline` journals with optional colors and filtering.
//!
//! This module uses the canonical rendering format defined in `canonical.rs` to ensure
//! consistent output across all contexts. Color is enabled via a `color: bool` flag.

pub mod colour;
pub mod canonical;
pub mod console;

use canonical::{render_scope_header, render_event, RenderConfig};
use crate::{
    Journal,
    RecordKind,
    Outcome,
    Filter,
};

/// Render the journal as a human-friendly tree with optional color and filtering.
///
/// Uses canonical "Narrative Structured" format:
/// - Clean scope headers with timestamps and outcomes
/// - Aligned event bullets with consistent spacing
/// - No arrow duplication (arrows only when they add information)
///
/// # Arguments
/// * `journal` - The journal to render
/// * `color` - Enable ANSI color codes
/// * `filter` - Optional filter to apply. If `None`, all scopes and events are rendered.
///
/// # Example
/// ```
/// use eventline::Journal;
/// use eventline::{Filter, ScopeFilter};
/// use eventline::Outcome;
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

    let config = if color {
        RenderConfig::default()
    } else {
        RenderConfig::no_color()
    };

    for scope in journal.scopes() {
        // Skip scopes that don't match the filter
        if !filter.matches_scope(scope, journal) {
            continue;
        }

        render_scope_tree(journal, scope, &config, filter);
    }
}

/// Render a single scope with its events using canonical format.
fn render_scope_tree(
    journal: &Journal,
    scope: &crate::Scope,
    config: &RenderConfig,
    filter: &Filter,
) {
    // Render scope header
    let scope_header = render_scope_header(journal, scope, config);
    println!("{}", scope_header.header);

    // Render events belonging to this scope
    for record in journal.records().iter().filter(|r| r.scope == Some(scope.id)) {
        // Skip events that don't match the event filter
        if !filter.matches_event(record) {
            continue;
        }

        if let Some(rendered) = render_event(record, config, 1) {
            println!("{}", rendered.main);
            
            // Print detail line if present (arrow rule: only if it adds information)
            if let Some(detail) = rendered.detail {
                println!("{}", detail);
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
/// - Per-scope summary using canonical format
///
/// # Arguments
/// * `journal` - The journal to summarize
/// * `color` - Enable ANSI color codes
/// * `filter` - Optional filter to apply. If `None`, all scopes and events are included.
pub fn render_summary(journal: &Journal, color: bool, filter: Option<&Filter>, per_scope: bool) {
    let default_filter = Filter::default();
    let filter = filter.unwrap_or(&default_filter);

    let config = if color {
        RenderConfig::default()
    } else {
        RenderConfig::no_color()
    };

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

    let total_duration_ms: u64 = filtered_scopes.iter().map(|s| {
        journal.records().iter()
            .find(|r| matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(s.id))
            .and_then(|r| {
                if let RecordKind::ScopeExit { exited_at, .. } = r.kind {
                    Some(exited_at.saturating_sub(s.entered_at))
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }).sum();

    println!("Session summary: {} scopes, {} events", total_scopes, total_events);
    println!("  Successful scopes: {}", success);
    println!("  Failed scopes: {}", failure);
    println!("  Aborted scopes: {}", aborted);
    println!("  Total duration: {}ms", total_duration_ms);
        
    if per_scope {
        println!("\nPer-scope summary:");
        for scope in &filtered_scopes {
            let scope_header = render_scope_header(journal, scope, &config);
            println!("  {}", scope_header.header);
        }
    }
}
