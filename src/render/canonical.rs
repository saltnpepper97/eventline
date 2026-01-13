//! Canonical rendering format for Eventline journals.
//!
//! This module defines the single source of truth for how journals are rendered.
//! All output systems (render tree, writer, console) use these primitives to ensure
//! consistent formatting across all contexts.
//!
//! ## Design Philosophy
//!
//! **Narrative Structured**: Human-readable, grep-friendly, minimal visual noise.
//!
//! ### Scope Headers
//! ```text
//! [19:04:12.381] Scope startup (id=12) → Failure (412ms)
//! ```
//!
//! ### Event Lines
//! ```text
//!   • info      message here
//!   • warning   disk space low
//!   • error     failed to bind socket
//! ```
//!
//! ### Detail Lines (arrows - only when adding new information)
//! ```text
//!   • error     failed to bind socket
//!       ↳ addr=0.0.0.0:8080 errno=EADDRINUSE
//! ```
//!
//! **Arrow Rule**: If the arrow would repeat the message, don't render it.
//! Arrows exist to show *why* or *detail*, not to echo *what*.

use crate::{
    EventKind,
    Journal,
    RecordKind,
    Outcome,
    journal::utils::millis_to_local,
};

#[cfg(test)]
use crate::RecordId;

#[cfg(feature = "colour")]
use crate::render::colour::{RESET, RED, YELLOW, GREEN, BLUE};

/// A rendered event line with optional detail.
#[derive(Debug, Clone)]
pub struct RenderedEvent {
    /// The main bullet line (always present)
    pub main: String,
    /// Optional detail line (arrow) - only if it adds information
    pub detail: Option<String>,
}

/// A rendered scope header.
#[derive(Debug, Clone)]
pub struct RenderedScope {
    /// The scope header line
    pub header: String,
}

/// Configuration for rendering output.
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// Enable ANSI color codes
    pub color: bool,
    /// Include timestamps in scope headers
    pub timestamps: bool,
    /// Bullet character for events
    pub bullet: String,
    /// Indent size per scope level (spaces)
    pub indent_size: usize,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            color: true,
            timestamps: true,
            bullet: if cfg!(windows) { "*".to_string() } else { "•".to_string() },
            indent_size: 2,
        }
    }
}

impl RenderConfig {
    /// Create a config with color disabled
    pub fn no_color() -> Self {
        Self {
            color: false,
            ..Default::default()
        }
    }

    /// Create a config without timestamps
    pub fn no_timestamps() -> Self {
        Self {
            timestamps: false,
            ..Default::default()
        }
    }
}

/// Render a scope header in canonical format.
///
/// Format: `[HH:MM:SS.mmm] Scope name (id=N) → Outcome (Nms)`
pub fn render_scope_header(
    journal: &Journal,
    scope: &crate::Scope,
    config: &RenderConfig,
) -> RenderedScope {
    // Find the scope exit record
    let exit = journal.records().iter().find(|r| {
        matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id)
    });

    // Determine outcome
    let outcome = exit
        .and_then(|r| {
            if let RecordKind::ScopeExit { outcome, .. } = r.kind {
                Some(outcome)
            } else {
                None
            }
        })
        .unwrap_or(Outcome::Aborted);

    // Calculate duration in milliseconds
    let duration_ms = exit
        .and_then(|r| {
            if let RecordKind::ScopeExit { exited_at, .. } = r.kind {
                Some(exited_at.saturating_sub(scope.entered_at))
            } else {
                None
            }
        })
        .unwrap_or(0);

    let name = scope.name.as_deref().unwrap_or("unnamed");

    // Format outcome with optional color
    let outcome_str = if config.color {
        #[cfg(feature = "colour")]
        {
            match outcome {
                Outcome::Success => format!("{}Success{}", GREEN, RESET),
                Outcome::Failure => format!("{}Failure{}", RED, RESET),
                Outcome::Aborted => format!("{}Aborted{}", YELLOW, RESET),
            }
        }
        #[cfg(not(feature = "colour"))]
        {
            format!("{:?}", outcome)
        }
    } else {
        format!("{:?}", outcome)
    };

    // Build header
    let header = if config.timestamps {
        let ts = millis_to_local(scope.entered_at);
        format!(
            "[{}] Scope {} (id={}) → {} ({}ms)",
            ts,
            name,
            scope.id.0,
            outcome_str,
            duration_ms
        )
    } else {
        format!(
            "Scope {} (id={}) → {} ({}ms)",
            name,
            scope.id.0,
            outcome_str,
            duration_ms
        )
    };

    RenderedScope { header }
}

/// Render an event in canonical format.
///
/// Format: `  • kind      message`
///
/// **Important**: Detail lines (arrows) are NOT rendered here unless you have
/// structured detail to add. The current implementation doesn't support structured
/// details yet, so arrows are omitted by design.
pub fn render_event(
    record: &crate::Record,
    config: &RenderConfig,
    indent_level: usize,
) -> Option<RenderedEvent> {
    if let RecordKind::Event { kind, message } = &record.kind {
        let indent = " ".repeat(config.indent_size * indent_level);

        // Format kind label (aligned, lowercase)
        let kind_label = match kind {
            EventKind::Info => "info     ",
            EventKind::Warning => "warning  ",
            EventKind::Error => "error    ",
            EventKind::Debug => "debug    ",
        };

        // Apply color to kind label if enabled 
        #[cfg(feature = "colour")]
        let display_label = if config.color {
            let colored = match kind {
                EventKind::Info => format!("{}info{}", GREEN, RESET),
                EventKind::Warning => format!("{}warning{}", YELLOW, RESET),
                EventKind::Error => format!("{}error{}", RED, RESET),
                EventKind::Debug => format!("{}debug{}", BLUE, RESET),
            };

            // Color codes don't count toward visual width
            // "warning" = 7 chars, needs 2 spaces to reach 9
            // "error" = 5 chars, needs 4 spaces to reach 9
            // "debug" = 5 chars, needs 4 spaces to reach 9
            // "info" = 4 chars, needs 5 spaces to reach 9
            let padding = match kind {
                EventKind::Warning => "  ",
                EventKind::Error => "    ",
                EventKind::Debug => "    ",
                EventKind::Info => "     ",
            };
            format!("{}{}", colored, padding)
        } else {
            kind_label.to_string()
        };

        #[cfg(not(feature = "colour"))]
        let display_label = kind_label.to_string();

        let main = format!("{}{} {} {}", indent, config.bullet, display_label, message);

        // No arrow/detail lines for now - they would only repeat the message
        // Future: Add when structured details (like errno, addr, etc.) exist

        Some(RenderedEvent {
            main,
            detail: None,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;

    #[test]
    fn test_scope_header_format() {
        let mut journal = Journal::new();
        let scope = journal.enter_scope_unnamed(None);
        journal.exit_scope(scope, Outcome::Success);

        let config = RenderConfig::no_color();
        let rendered = render_scope_header(&journal, journal.scopes().first().unwrap(), &config);

        assert!(rendered.header.contains("Scope unnamed"));
        assert!(rendered.header.contains("→ Success"));
        assert!(rendered.header.contains("ms)"));
    }

    #[test]
    fn test_event_format() {
        let _journal = Journal::new();
        let record = crate::Record {
            id: RecordId(1),
            scope: None,
            kind: RecordKind::Event {
                kind: EventKind::Warning,
                message: "test message".to_string(),
            },
            time: 0,
        };

        let config = RenderConfig::no_color();
        let rendered = render_event(&record, &config, 1).unwrap();

        assert!(rendered.main.contains("•"));
        assert!(rendered.main.contains("warning"));
        assert!(rendered.main.contains("test message"));
        assert!(rendered.detail.is_none());
    }

    #[test]
    fn test_no_arrow_duplication() {
        // Verify that we don't create arrow lines that just repeat the message
        let _journal = Journal::new();
        let record = crate::Record {
            id: RecordId(1),
            scope: None,
            kind: RecordKind::Event {
                kind: EventKind::Error,
                message: "failed to bind socket".to_string(),
            },
            time: 0,
        };

        let config = RenderConfig::default();
        let rendered = render_event(&record, &config, 1).unwrap();

        // Should have no detail line since we have no extra info to add
        assert!(rendered.detail.is_none());
    }
}
